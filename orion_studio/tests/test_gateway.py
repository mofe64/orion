from __future__ import annotations

import json
import sys
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from gateway import GatewayError, OrionGateway, make_handler  # noqa: E402


class FakeOrionClient:
    def __init__(self):
        self.commands: list[str] = []
        self.motion = None
        self.scene = None
        self.speech = None
        self.reject_reload = False
        self.reject_reload_count = 0

    def request(self, command: str):
        self.commands.append(command)
        if command == "status":
            return {
                "schema_version": 3,
                "robot": "orion",
                "build_revision": "test-revision",
                "mode": "holding",
                "torque_enabled": True,
                "motion": self.motion,
                "last_motion": None,
                "joints": [],
            }
        if command == "scene status":
            return {"ok": True, "scene": self.scene, "last_scene": None}
        if command == "speech status":
            return {"ok": True, "speech": self.speech, "last_speech": None}
        if command == "pose list":
            return {"ok": True, "poses": ["home", "rest"]}
        if command == "motion list":
            return {"ok": True, "motions": ["look_at_left"]}
        if command == "scene list":
            return {"ok": True, "scenes": ["acknowledge_left"]}
        if command == "scene reload":
            if self.reject_reload or self.reject_reload_count > 0:
                if self.reject_reload_count > 0:
                    self.reject_reload_count -= 1
                return {"ok": False, "error": "invalid scene catalog"}
            return {"ok": True, "command": "scene_reload", "scenes": ["acknowledge_left"]}
        if command.startswith("goto "):
            self.motion = {"run_id": 4, "name": "home", "state": "executing"}
            return {"ok": True, "command": "goto", "run_id": 4, "state": "executing"}
        if command.startswith("play "):
            return {"ok": True, "command": "play", "run_id": 5, "state": "executing"}
        if command.startswith("scene start "):
            return {"ok": True, "command": "scene_start", "run_id": 6, "state": "executing"}
        if command.startswith("speech start "):
            return {"ok": True, "command": "speech_start", "run_id": 7, "state": "synthesizing"}
        if command == "stop":
            self.motion = None
            return {"ok": True, "command": "stop"}
        if command == "scene stop":
            self.scene = None
            return {"ok": True, "command": "scene_stop"}
        if command == "speech stop":
            self.speech = None
            return {"ok": True, "command": "speech_stop"}
        return {"ok": False, "error": "unexpected fake command"}


class GatewayTests(unittest.TestCase):
    def setUp(self):
        self.client = FakeOrionClient()
        self.gateway = OrionGateway(self.client)

    def test_discovers_names_from_oriond(self):
        capabilities = self.gateway.capabilities()["capabilities"]
        self.assertEqual(capabilities["goto"], ["home", "rest"])
        self.assertEqual(capabilities["motion"], ["look_at_left"])
        self.assertEqual(capabilities["scene"], ["acknowledge_left"])

    def test_translates_only_named_semantic_operations(self):
        status, response = self.gateway.submit(
            {"operation": "goto", "name": "home", "duration_seconds": 2.5}
        )
        self.assertEqual(status, 202)
        self.assertEqual(response["result"]["run_id"], 4)
        self.assertEqual(self.client.commands[-1], "goto home 2.500000")

        with self.assertRaises(GatewayError):
            self.gateway.submit({"operation": "joint_stream", "positions": [0, 1]})
        self.assertNotIn("joint_stream", self.client.commands)

    def test_rejects_a_stale_cancel_without_stopping_newer_motion(self):
        self.client.motion = {"run_id": 9, "name": "home", "state": "executing"}
        with self.assertRaises(GatewayError) as context:
            self.gateway.submit({"operation": "cancel", "kind": "movement", "run_id": 8})
        self.assertEqual(context.exception.code, "run_not_active")
        self.assertNotEqual(self.client.commands[-1], "stop")

        self.gateway.submit({"operation": "cancel", "kind": "movement", "run_id": 9})
        self.assertEqual(self.client.commands[-1], "stop")

    def test_rejects_newlines_in_speech(self):
        with self.assertRaises(GatewayError):
            self.gateway.submit({"operation": "speech", "text": "hello\nstop"})

    def test_publishes_only_new_or_identical_user_scenes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scenes").mkdir()
            gateway = OrionGateway(self.client, root)
            document = {
                "format_version": 1,
                "scene": {
                    "name": "studio_wave",
                    "description": "A test scene.",
                    "timeline": [
                        {"at": 0, "type": "goto_pose", "pose": "home", "duration_seconds": 1.5}
                    ],
                },
            }
            status, result = gateway.publish_scene(document)
            self.assertEqual(status, 201)
            self.assertFalse(result["already_present"])
            self.assertTrue((root / "scenes/user/studio_wave.yaml").is_file())
            self.assertEqual(self.client.commands[-1], "scene reload")

            status, result = gateway.publish_scene(document)
            self.assertEqual(status, 200)
            self.assertTrue(result["already_present"])

            changed = json.loads(json.dumps(document))
            changed["scene"]["description"] = "Different content."
            with self.assertRaises(GatewayError) as context:
                gateway.publish_scene(changed)
            self.assertEqual(context.exception.code, "user_scene_exists")

    def test_lists_reads_and_revision_updates_user_scenes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scenes").mkdir()
            gateway = OrionGateway(self.client, root)
            document = {
                "format_version": 1,
                "scene": {
                    "name": "studio_wave",
                    "description": "First version.",
                    "timeline": [
                        {"at": 0, "type": "play_motion", "motion": "look_at_left"}
                    ],
                },
            }
            _, published = gateway.publish_scene(document)

            library = gateway.list_user_scenes()
            self.assertEqual(len(library["scenes"]), 1)
            self.assertEqual(library["scenes"][0]["name"], "studio_wave")
            self.assertEqual(library["scenes"][0]["revision"], published["revision"])

            loaded = gateway.read_user_scene("studio_wave")
            self.assertEqual(json.loads(loaded["yaml"]), document)
            self.assertEqual(loaded["revision"], published["revision"])

            changed = json.loads(json.dumps(document))
            changed["scene"]["description"] = "Second version."
            status, updated = gateway.update_user_scene(
                "studio_wave",
                {"expected_revision": loaded["revision"], "document": changed},
            )
            self.assertEqual(status, 200)
            self.assertTrue(updated["updated"])
            self.assertNotEqual(updated["revision"], loaded["revision"])
            self.assertEqual(
                json.loads(gateway.read_user_scene("studio_wave")["yaml"]),
                changed,
            )

            with self.assertRaises(GatewayError) as context:
                gateway.update_user_scene(
                    "studio_wave",
                    {"expected_revision": loaded["revision"], "document": document},
                )
            self.assertEqual(context.exception.code, "scene_revision_conflict")

    def test_user_scene_library_refuses_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            user = root / "scenes/user"
            user.mkdir(parents=True)
            outside = root / "outside.yaml"
            outside.write_text("not a scene", encoding="utf-8")
            (user / "linked.yaml").symlink_to(outside)
            gateway = OrionGateway(self.client, root)
            with self.assertRaises(GatewayError) as context:
                gateway.list_user_scenes()
            self.assertEqual(context.exception.code, "unsafe_scene_file")

    def test_revision_update_restores_previous_scene_when_reload_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scenes").mkdir()
            gateway = OrionGateway(self.client, root)
            document = {
                "format_version": 1,
                "scene": {
                    "name": "rollback_scene",
                    "description": "Known good.",
                    "timeline": [{"at": 0, "type": "audio", "cue": "acknowledge"}],
                },
            }
            _, published = gateway.publish_scene(document)
            path = root / "scenes/user/rollback_scene.yaml"
            previous = path.read_bytes()
            changed = json.loads(json.dumps(document))
            changed["scene"]["timeline"][0]["cue"] = "missing_cue"
            self.client.reject_reload_count = 1

            with self.assertRaises(GatewayError) as context:
                gateway.update_user_scene(
                    "rollback_scene",
                    {"expected_revision": published["revision"], "document": changed},
                )
            self.assertEqual(context.exception.code, "runtime_rejected")
            self.assertEqual(path.read_bytes(), previous)
            self.assertEqual(self.client.commands[-1], "scene reload")

    def test_rejects_invalid_scene_documents_before_writing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scenes").mkdir()
            gateway = OrionGateway(self.client, root)
            with self.assertRaises(GatewayError):
                gateway.publish_scene(
                    {
                        "format_version": 1,
                        "scene": {
                            "name": "unsafe",
                            "description": "Invalid raw event.",
                            "timeline": [{"at": 0, "type": "joint_stream", "positions": []}],
                        },
                    }
                )
            self.assertFalse((root / "scenes/user/unsafe.yaml").exists())

    def test_rolls_back_a_new_file_when_oriond_rejects_reload(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scenes").mkdir()
            self.client.reject_reload = True
            gateway = OrionGateway(self.client, root)
            document = {
                "format_version": 1,
                "scene": {
                    "name": "rejected_scene",
                    "description": "Runtime will reject this catalog.",
                    "timeline": [{"at": 0, "type": "play_motion", "motion": "look_at_left"}],
                },
            }
            with self.assertRaises(GatewayError):
                gateway.publish_scene(document)
            self.assertFalse((root / "scenes/user/rejected_scene.yaml").exists())


class HttpAuthenticationTests(unittest.TestCase):
    def setUp(self):
        self.fake = FakeOrionClient()
        self.temporary = tempfile.TemporaryDirectory()
        self.project_root = Path(self.temporary.name)
        (self.project_root / "scenes").mkdir()
        self.server = ThreadingHTTPServer(
            ("127.0.0.1", 0),
            make_handler(
                OrionGateway(self.fake, self.project_root),
                "a" * 32,
                ["http://localhost:1420", "tauri://localhost"],
            ),
        )
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.base_url = f"http://127.0.0.1:{self.server.server_port}"

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        self.temporary.cleanup()

    def test_requires_bearer_token(self):
        with self.assertRaises(urllib.error.HTTPError) as context:
            urllib.request.urlopen(f"{self.base_url}/api/v1/status")
        self.assertEqual(context.exception.code, 401)
        body = json.loads(context.exception.read())
        self.assertEqual(body["error"]["code"], "unauthorized")

    def test_returns_status_to_authenticated_studio(self):
        request = urllib.request.Request(
            f"{self.base_url}/api/v1/status",
            headers={
                "Authorization": f"Bearer {'a' * 32}",
                "Origin": "tauri://localhost",
            },
        )
        with urllib.request.urlopen(request) as response:
            body = json.load(response)
            self.assertEqual(response.headers["Access-Control-Allow-Origin"], "tauri://localhost")
        self.assertEqual(body["api_version"], 1)
        self.assertEqual(body["runtime"]["mode"], "holding")

    def test_serves_authenticated_scene_create_read_and_revision_update(self):
        authorization = {"Authorization": f"Bearer {'a' * 32}"}
        document = {
            "format_version": 1,
            "scene": {
                "name": "http_scene",
                "description": "Created through HTTP.",
                "timeline": [{"at": 0, "type": "audio", "cue": "acknowledge"}],
            },
        }
        create = urllib.request.Request(
            f"{self.base_url}/api/v1/scenes",
            data=json.dumps(document).encode(),
            headers={**authorization, "Content-Type": "application/json"},
        )
        with urllib.request.urlopen(create) as response:
            created = json.load(response)
        self.assertEqual(response.status, 201)

        listing = urllib.request.Request(
            f"{self.base_url}/api/v1/scenes",
            headers=authorization,
        )
        with urllib.request.urlopen(listing) as response:
            library = json.load(response)
        self.assertEqual(library["scenes"][0]["revision"], created["revision"])

        read = urllib.request.Request(
            f"{self.base_url}/api/v1/scenes/http_scene",
            headers=authorization,
        )
        with urllib.request.urlopen(read) as response:
            loaded = json.load(response)
        self.assertEqual(json.loads(loaded["yaml"]), document)

        changed = json.loads(json.dumps(document))
        changed["scene"]["description"] = "Updated through HTTP."
        update = urllib.request.Request(
            f"{self.base_url}/api/v1/scenes/http_scene",
            data=json.dumps({
                "expected_revision": loaded["revision"],
                "document": changed,
            }).encode(),
            headers={**authorization, "Content-Type": "application/json"},
            method="PUT",
        )
        with urllib.request.urlopen(update) as response:
            updated = json.load(response)
        self.assertTrue(updated["updated"])
        self.assertNotEqual(updated["revision"], loaded["revision"])


if __name__ == "__main__":
    unittest.main()
