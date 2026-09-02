from __future__ import annotations

import io
import json
import sys
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
import wave
from http import HTTPStatus
from http.server import ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from gateway import GatewayError, OrionGateway, make_handler  # noqa: E402

JOINTS = (
    "base_yaw_joint", "shoulder_pitch_joint", "elbow_pitch_joint",
    "head_roll_joint", "head_pitch_joint",
)


def pose_document(name: str = "studio_pose", base: float = 0.2) -> dict[str, object]:
    return {
        "format_version": 2, "units": "radians",
        "poses": {name: {
            "description": "A Studio-authored powered pose.",
            "tags": ["powered", "idle_anchor"], "idle_profile": "home",
            "default_lighting": "warm_idle_breathe",
            "positions": {joint: base if joint == JOINTS[0] else 0.0 for joint in JOINTS},
        }},
    }


def motion_document(name: str = "studio_motion") -> dict[str, object]:
    return {"format_version": 2, "motion": {
        "name": name, "description": "A continuous Studio motion.",
        "space": "absolute", "style": "attentive",
        "keyframes": [
            {"pose": "studio_pose", "duration": 0.7, "arrival": "through", "marker": "notice"},
            {"pose": "home", "duration": 0.8, "arrival": "settle"},
        ],
    }}


def relative_motion_document(name: str = "studio_idle") -> dict[str, object]:
    return {"format_version": 2, "motion": {
        "name": name, "description": "A returning relative motion.",
        "space": "anchor_relative", "style": "living_idle", "return_to_anchor": True,
        "keyframes": [
            {"offsets": {"head_pitch_joint": 0.02}, "duration": 0.6, "arrival": "through"},
            {"offsets": {}, "duration": 0.7, "arrival": "settle"},
        ],
    }}


def scene_document(name: str = "studio_scene") -> dict[str, object]:
    return {"format_version": 2, "scene": {
        "name": name, "description": "Parallel motion, light, and sound.",
        "motion": [{"at": 0.0, "play": "studio_motion"}],
        "lighting": [{"on_marker": "notice", "effect": "acknowledge_pulse"}],
        "audio": [{"on_marker": "notice", "cue": "notice_warm"}],
        "finish": {"anchor": "final_pose", "lighting": "pose_default"},
    }}


def pcm_wav(*, channels: int = 1, width: int = 2, rate: int = 24_000, frames: int = 480) -> bytes:
    output = io.BytesIO()
    with wave.open(output, "wb") as wav:
        wav.setnchannels(channels); wav.setsampwidth(width); wav.setframerate(rate)
        wav.writeframes(b"\0" * frames * channels * width)
    return output.getvalue()


class FakeOrionClient:
    def __init__(self) -> None:
        self.commands: list[str] = []
        self.motion = self.scene = self.speech = None
        self.reject_reload = False
        self.mode, self.torque_enabled = "holding", True
        self.character = {"enabled": False, "state": "off", "active_anchor": None,
                          "active_clip": None, "next_idle_category": None}

    def request(self, command: str) -> dict[str, object]:
        self.commands.append(command)
        if command == "status":
            return {"schema_version": 3, "robot": "orion", "build_revision": "test-revision",
                    "mode": self.mode, "torque_enabled": self.torque_enabled,
                    "motion": self.motion, "last_motion": None, "joints": []}
        if command == "scene status": return {"ok": True, "scene": self.scene, "last_scene": None}
        if command == "speech status": return {"ok": True, "speech": self.speech, "last_speech": None}
        if command == "character status": return {"ok": True, "character": self.character}
        if command == "pose list": return {"ok": True, "poses": ["home", "rest"]}
        if command == "motion list": return {"ok": True, "motions": ["look_at_left_expressive"]}
        if command == "scene list": return {"ok": True, "scenes": ["acknowledge_left"]}
        if command == "joint limits":
            return {"ok": True, "joints": [
                {"name": name, "lower_rad": -1.5, "upper_rad": 1.5} for name in JOINTS
            ]}
        if command in {"scene reload", "asset reload"}:
            return ({"ok": False, "error": "catalog rejected"} if self.reject_reload
                    else {"ok": True, "command": command.replace(" ", "_")})
        if command == "configure":
            self.mode, self.torque_enabled = "configured", False; return {"ok": True, "mode": self.mode}
        if command == "enable":
            self.mode, self.torque_enabled = "holding", True; return {"ok": True, "mode": self.mode}
        if command == "disable":
            self.mode, self.torque_enabled = "configured", False; return {"ok": True, "mode": self.mode}
        if command.startswith("goto "):
            self.motion = {"run_id": 4, "state": "executing"}; return {"ok": True, "run_id": 4, "state": "executing"}
        if command.startswith("play "): return {"ok": True, "run_id": 5, "state": "executing"}
        if command.startswith("scene start "): return {"ok": True, "run_id": 6, "state": "executing"}
        if command.startswith("scene preview "): return {"ok": True, "run_id": 8, "state": "executing", "persisted": False}
        if command.startswith("speech start "): return {"ok": True, "run_id": 7, "state": "synthesizing"}
        if command.startswith("speech file "):
            self.speech = {"run_id": 9, "state": "playing"}; return {"ok": True, "run_id": 9, "state": "queued"}
        if command == "character start":
            self.character = {**self.character, "enabled": True, "state": "starting"}; return {"ok": True, "character": self.character}
        if command == "character stop":
            self.character = {**self.character, "enabled": False, "state": "off"}; return {"ok": True, "character": self.character}
        if command.startswith("character state "):
            self.character = {**self.character, "state": command.rsplit(" ", 1)[1]}; return {"ok": True, "character": self.character}
        if command in {"stop", "scene stop", "speech stop"}: return {"ok": True}
        return {"ok": False, "error": f"unexpected fake command: {command}"}


class GatewayContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.client = FakeOrionClient(); self.gateway = OrionGateway(self.client)

    def test_capabilities_are_v2_and_calibration_owned(self) -> None:
        capabilities = self.gateway.capabilities()["capabilities"]
        self.assertEqual((capabilities["pose_format_version"], capabilities["motion_format_version"],
                          capabilities["scene_format_version"]), (2, 2, 2))
        self.assertEqual(len(capabilities["joint_limits"]), 5)
        self.assertEqual(capabilities["hardware_profile"]["variant"], "7.4 V STS3215")
        self.assertAlmostEqual(capabilities["hardware_profile"]["maximum_no_load_speed_rad_s"], 5.4454272662)

    def test_status_and_character_operations_are_deterministic(self) -> None:
        self.assertEqual(self.gateway.status()["character"]["state"], "off")
        for payload, expected in (
            ({"operation": "character_start"}, "character start"),
            ({"operation": "character_state", "state": "listening"}, "character state listening"),
            ({"operation": "character_state", "state": "thinking"}, "character state thinking"),
            ({"operation": "character_stop"}, "character stop"),
        ):
            self.assertEqual(self.gateway.submit(payload)[0], HTTPStatus.ACCEPTED)
            self.assertEqual(self.client.commands[-1], expected)
        with self.assertRaisesRegex(GatewayError, "neutral, listening, or thinking"):
            self.gateway.submit({"operation": "character_state", "state": "dancing"})

    def test_preview_accepts_v2_and_rejects_v1_clearly(self) -> None:
        document = scene_document("preview_scene")
        status, response = self.gateway.submit({"operation": "preview_scene", "document": document})
        self.assertEqual(status, HTTPStatus.ACCEPTED); self.assertFalse(response["result"]["persisted"])
        self.assertEqual(json.loads(self.client.commands[-1].removeprefix("scene preview ")), document)
        with self.assertRaisesRegex(GatewayError, "v2 required"):
            self.gateway.submit({"operation": "preview_scene", "document": {"format_version": 1, "scene": {}}})

    def test_v2_validators_reject_unknown_fields_and_nonreturning_relative_motion(self) -> None:
        pose = pose_document(); pose["poses"]["studio_pose"]["mystery"] = True
        with self.assertRaises(GatewayError): self.gateway._validate_pose_document(pose)
        motion = relative_motion_document(); motion["motion"]["keyframes"][-1]["offsets"] = {"head_pitch_joint": 0.01}
        with self.assertRaisesRegex(GatewayError, "finish at zero offsets"): self.gateway._validate_motion_document(motion)
        scene = scene_document(); scene["scene"]["lighting"][0]["unknown"] = 1
        with self.assertRaisesRegex(GatewayError, "invalid fields"): self.gateway._validate_scene_document(scene)

    def test_unsaved_motion_document_uses_the_real_rust_compiler_without_persisting(self) -> None:
        root = Path(__file__).resolve().parents[2]
        compiler = root / "runtime/target/debug/orion-trajectory"
        if not compiler.is_file():
            self.skipTest("Build runtime all-targets before the gateway integration test.")
        gateway = OrionGateway(
            self.client,
            root,
            calibration_file=root / "simulation/mujoco/config/servo_calibration.json",
            trajectory_compiler=compiler,
        )
        document = motion_document("studio_preview")
        document["motion"]["keyframes"][0]["pose"] = "attentive"
        result = gateway.compile_trajectory_preview({
            "document": document,
            "start_pose": "home",
        })
        self.assertEqual(result["compiler"], "orion-runtime")
        self.assertEqual(result["motion_name"], "studio_preview")
        self.assertEqual(result["control_rate_hz"], 50)
        self.assertFalse((root / "motion/motions/studio_preview.yaml").exists())

    def test_publishes_immutable_v2_assets(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / "motion").mkdir(); (root / "scenes").mkdir()
            gateway = OrionGateway(self.client, root)
            self.assertEqual(gateway.publish_pose(pose_document())[0], HTTPStatus.CREATED)
            self.assertEqual(gateway.publish_motion(motion_document())[0], HTTPStatus.CREATED)
            self.assertEqual(gateway.publish_motion(relative_motion_document())[0], HTTPStatus.CREATED)
            self.assertEqual(gateway.publish_scene(scene_document())[0], HTTPStatus.CREATED)
            self.assertTrue((root / "motion/user/poses/studio_pose.yaml").is_file())
            self.assertTrue((root / "motion/motions/user/studio_motion.yaml").is_file())
            self.assertTrue((root / "scenes/user/studio_scene.yaml").is_file())
            self.assertEqual(gateway.publish_pose(pose_document())[0], HTTPStatus.OK)
            with self.assertRaisesRegex(GatewayError, "already exists"): gateway.publish_pose(pose_document(base=0.3))

    def test_pose_is_checked_against_live_joint_ranges_before_write(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / "motion").mkdir(); gateway = OrionGateway(self.client, root)
            with self.assertRaisesRegex(GatewayError, "must stay between"): gateway.publish_pose(pose_document("unsafe_pose", 2.0))
            self.assertFalse((root / "motion/user/poses/unsafe_pose.yaml").exists())

    def test_speech_spooling_validation_status_and_rejection_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            spool = Path(directory); gateway = OrionGateway(self.client, speech_spool=spool)
            status, result = gateway.upload_speech(pcm_wav(), "studio-request-1")
            self.assertEqual((status, result["run_id"]), (HTTPStatus.ACCEPTED, 9))
            files = list(spool.glob("*.wav")); self.assertEqual(len(files), 1)
            self.assertEqual(self.client.commands[-1], f"speech file {files[0].stem}")
            self.assertEqual(gateway.speech_run_status(9)["state"], "playing")

        class RejectingClient(FakeOrionClient):
            def request(self, command: str) -> dict[str, object]:
                return ({"ok": False, "error": "playback unavailable"}
                        if command.startswith("speech file ") else super().request(command))

        with tempfile.TemporaryDirectory() as directory:
            spool = Path(directory); gateway = OrionGateway(RejectingClient(), speech_spool=spool)
            with self.assertRaisesRegex(GatewayError, "playback unavailable"): gateway.upload_speech(pcm_wav(), "request")
            self.assertEqual(list(spool.iterdir()), [])

        for body in (b"not wav", pcm_wav(channels=2), pcm_wav(width=1), pcm_wav(rate=16_000)):
            with self.subTest(bytes=len(body)):
                with self.assertRaises(GatewayError): self.gateway.upload_speech(body, "request")
        with self.assertRaises(GatewayError): self.gateway.upload_speech(b"x" * (8 * 1024 * 1024 + 1), "request")
        with self.assertRaisesRegex(GatewayError, "no longer than 120 seconds"):
            self.gateway.upload_speech(pcm_wav(frames=24_000 * 121), "request")


class HttpAuthenticationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.fake = FakeOrionClient(); self.temporary = tempfile.TemporaryDirectory()
        root = Path(self.temporary.name); (root / "scenes").mkdir()
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(
            OrionGateway(self.fake, root), "a" * 32, ["tauri://localhost"]))
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True); self.thread.start()
        self.base_url = f"http://127.0.0.1:{self.server.server_port}"

    def tearDown(self) -> None:
        self.server.shutdown(); self.server.server_close(); self.thread.join(timeout=2); self.temporary.cleanup()

    def request(self, path: str, *, document: object | None = None, authorized: bool = True):
        headers = {"Origin": "tauri://localhost"}
        if authorized: headers["Authorization"] = f"Bearer {'a' * 32}"
        data = None
        if document is not None:
            data = json.dumps(document).encode(); headers["Content-Type"] = "application/json"
        return urllib.request.Request(f"{self.base_url}{path}", data=data, headers=headers)

    def test_authentication_precedes_routing_and_v1_routes_are_gone(self) -> None:
        with self.assertRaises(urllib.error.HTTPError) as context:
            urllib.request.urlopen(self.request("/api/v2/status", authorized=False))
        self.assertEqual(context.exception.code, HTTPStatus.UNAUTHORIZED)
        with self.assertRaises(urllib.error.HTTPError) as context:
            urllib.request.urlopen(self.request("/api/v1/status"))
        self.assertEqual(context.exception.code, HTTPStatus.NOT_FOUND)

    def test_serves_authenticated_v2_status_and_scene_publish(self) -> None:
        with urllib.request.urlopen(self.request("/api/v2/status")) as response:
            body = json.load(response); self.assertEqual(response.headers["Access-Control-Allow-Origin"], "tauri://localhost")
        self.assertEqual(body["api_version"], 2); self.assertEqual(body["character"]["state"], "off")
        with urllib.request.urlopen(self.request("/api/v2/scenes", document=scene_document("http_scene"))) as response:
            published = json.load(response); self.assertEqual(response.status, HTTPStatus.CREATED)
        self.assertEqual(published["name"], "http_scene")


if __name__ == "__main__":
    unittest.main()
