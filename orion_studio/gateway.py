#!/usr/bin/env python3
"""Small authenticated HTTP adapter for Orion's private Unix command socket."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import re
import secrets
import shutil
import socket
import stat
import subprocess
import tempfile
import threading
import wave
from io import BytesIO
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import urlparse


API_VERSION = 2
DEFAULT_SOCKET = "/tmp/oriond.sock"
DEFAULT_SPEECH_SPOOL = Path("/tmp/orion-speech-spool")
DEFAULT_CALIBRATION = Path("~/.config/orion/servo_calibration.json")
MAX_BODY_BYTES = 262_144
MAX_SPEECH_BYTES = 8 * 1024 * 1024
MAX_PREVIEW_SCENE_BYTES = 3_000
MAX_RESPONSE_BYTES = 1_048_576
NAME_PATTERN = re.compile(r"^[A-Za-z0-9_-]+$")
JOINT_NAMES = (
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
)
DEFAULT_ALLOWED_ORIGINS = (
    "http://localhost:1420",
    "http://127.0.0.1:1420",
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
)


class GatewayError(Exception):
    def __init__(self, status: HTTPStatus, code: str, message: str):
        super().__init__(message)
        self.status = status
        self.code = code


class UnixOrionClient:
    """One-command/one-response client for the Pi-local oriond socket."""

    def __init__(self, socket_path: str = DEFAULT_SOCKET, timeout_seconds: float = 2.0):
        self.socket_path = socket_path
        self.timeout_seconds = timeout_seconds

    def request(self, command: str) -> dict[str, Any]:
        if not command or "\n" in command or "\r" in command or "\0" in command:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_command", "Invalid local command.")

        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
                client.settimeout(self.timeout_seconds)
                client.connect(self.socket_path)
                client.sendall(command.encode("utf-8") + b"\n")
                client.shutdown(socket.SHUT_WR)
                chunks: list[bytes] = []
                received = 0
                while True:
                    chunk = client.recv(65_536)
                    if not chunk:
                        break
                    received += len(chunk)
                    if received > MAX_RESPONSE_BYTES:
                        raise GatewayError(
                            HTTPStatus.BAD_GATEWAY,
                            "runtime_response_too_large",
                            "oriond returned an unexpectedly large response.",
                        )
                    chunks.append(chunk)
        except GatewayError:
            raise
        except OSError as error:
            raise GatewayError(
                HTTPStatus.SERVICE_UNAVAILABLE,
                "runtime_unavailable",
                f"Could not reach oriond: {error}",
            ) from error

        try:
            value = json.loads(b"".join(chunks).decode("utf-8").strip())
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise GatewayError(
                HTTPStatus.BAD_GATEWAY,
                "invalid_runtime_response",
                "oriond returned invalid JSON.",
            ) from error
        if not isinstance(value, dict):
            raise GatewayError(
                HTTPStatus.BAD_GATEWAY,
                "invalid_runtime_response",
                "oriond returned a non-object response.",
            )
        return value


class OrionGateway:
    """Allowlisted semantic translation; this class never controls hardware."""

    def __init__(
        self,
        client: Any,
        project_root: Path | None = None,
        speech_spool: Path = DEFAULT_SPEECH_SPOOL,
        calibration_file: Path = DEFAULT_CALIBRATION,
        trajectory_compiler: Path | None = None,
    ):
        self.client = client
        self.project_root = project_root
        self.speech_spool = speech_spool
        self.calibration_file = calibration_file.expanduser()
        self.trajectory_compiler = trajectory_compiler
        self.scene_write_lock = threading.Lock()

    def status(self) -> dict[str, Any]:
        runtime = self.client.request("status")
        scenes = self._checked("scene status")
        speech = self._checked("speech status")
        character = self._checked("character status")
        return {
            "api_version": API_VERSION,
            "runtime": runtime,
            "scene": {
                "active": scenes.get("scene"),
                "last": scenes.get("last_scene"),
            },
            "speech": {
                "active": speech.get("speech"),
                "last": speech.get("last_speech"),
            },
            "character": character.get("character"),
        }

    def capabilities(self) -> dict[str, Any]:
        poses = self._checked("pose list")
        motions = self._checked("motion list")
        scenes = self._checked("scene list")
        limits = self._checked("joint limits")
        return {
            "api_version": API_VERSION,
            "capabilities": {
                "goto": poses.get("poses", []),
                "motion": motions.get("motions", []),
                "scene": scenes.get("scenes", []),
                "pose_format_version": 2,
                "motion_format_version": 2,
                "scene_format_version": 2,
                "scene_publish": {"format_version": 2, "max_body_bytes": MAX_BODY_BYTES},
                "scene_preview": {
                    "format_version": 2,
                    "max_body_bytes": MAX_PREVIEW_SCENE_BYTES,
                    "persisted": False,
                },
                "scene_library": {"read": True, "create": True, "update": "revision"},
                "joint_limits": limits.get("joints", []),
                "pose_library": {"read": True, "create": True, "update": False},
                "motion_library": {"read": True, "create": True, "update": False},
                "movement_lifecycle": ["prepare", "release"],
                "speech": {"format": "pcm16_mono_24000_hz", "max_bytes": MAX_SPEECH_BYTES, "max_seconds": 120},
                "character_states": ["neutral", "listening", "thinking"],
                "hardware_profile": {
                    "variant": "7.4 V STS3215",
                    "encoder_counts_per_revolution": 4096,
                    "maximum_no_load_speed_rpm": 52,
                    "maximum_no_load_speed_rad_s": 5.4454272662,
                    "rated_torque_kg_cm": 5.0,
                    "stall_torque_kg_cm": 19.5,
                    "runtime_control_hz": 50,
                    "native_profile_registers": True,
                },
                "cancel": ["movement", "scene", "speech"],
            },
        }

    def submit(self, payload: Any) -> tuple[HTTPStatus, dict[str, Any]]:
        if not isinstance(payload, dict):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_request", "Expected a JSON object.")
        operation = payload.get("operation")
        if operation == "goto":
            name = self._name(payload.get("name"), "pose")
            duration = payload.get("duration_seconds", 3.0)
            if isinstance(duration, bool) or not isinstance(duration, (int, float)):
                raise GatewayError(
                    HTTPStatus.BAD_REQUEST, "invalid_duration", "duration_seconds must be a number."
                )
            duration = float(duration)
            if not 0.1 <= duration <= 60.0:
                raise GatewayError(
                    HTTPStatus.BAD_REQUEST,
                    "invalid_duration",
                    "duration_seconds must be between 0.1 and 60.0.",
                )
            response = self._checked(f"goto {name} {duration:.6f}")
        elif operation == "motion":
            name = self._name(payload.get("name"), "motion")
            response = self._checked(f"play {name}")
        elif operation == "scene":
            name = self._name(payload.get("name"), "scene")
            response = self._checked(f"scene start {name}")
        elif operation == "preview_scene":
            if set(payload) != {"operation", "document"}:
                raise GatewayError(
                    HTTPStatus.BAD_REQUEST,
                    "invalid_scene_preview",
                    "Scene preview requires only operation and document fields.",
                )
            document = payload.get("document")
            self._validate_scene_document(document)
            try:
                encoded = json.dumps(
                    document,
                    separators=(",", ":"),
                    ensure_ascii=False,
                    allow_nan=False,
                ).encode("utf-8")
            except (TypeError, ValueError) as error:
                raise GatewayError(
                    HTTPStatus.BAD_REQUEST,
                    "invalid_scene_preview",
                    f"Could not encode the scene preview: {error}",
                ) from error
            if len(encoded) > MAX_PREVIEW_SCENE_BYTES:
                raise GatewayError(
                    HTTPStatus.REQUEST_ENTITY_TOO_LARGE,
                    "scene_preview_too_large",
                    f"Scene preview cannot exceed {MAX_PREVIEW_SCENE_BYTES} UTF-8 bytes.",
                )
            response = self._checked(f"scene preview {encoded.decode('utf-8')}")
        elif operation == "prepare_movement":
            response = self._prepare_movement()
        elif operation == "release_movement":
            response = self._release_movement()
        elif operation == "speech":
            text = payload.get("text")
            if not isinstance(text, str) or not text.strip():
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_speech", "Speech text is required.")
            text = text.strip()
            if len(text.encode("utf-8")) > 2_000 or any(char in text for char in "\r\n\0"):
                raise GatewayError(
                    HTTPStatus.BAD_REQUEST,
                    "invalid_speech",
                    "Speech must be one line and no more than 2000 UTF-8 bytes.",
                )
            response = self._checked(f"speech start {text}")
        elif operation == "character_start":
            response = self._checked("character start")
        elif operation == "character_stop":
            response = self._checked("character stop")
        elif operation == "character_state":
            state = payload.get("state")
            if state not in {"neutral", "listening", "thinking"}:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_character_state", "Character state must be neutral, listening, or thinking.")
            response = self._checked(f"character state {state}")
        elif operation == "cancel":
            response = self._cancel(payload)
        else:
            raise GatewayError(
                HTTPStatus.BAD_REQUEST,
                "unsupported_operation",
                "Supported operations are goto, motion, scene, preview_scene, speech, character_start, character_stop, character_state, prepare_movement, release_movement, and cancel.",
            )

        return HTTPStatus.ACCEPTED, {
            "api_version": API_VERSION,
            "accepted": True,
            "operation": operation,
            "result": response,
        }

    def upload_speech(self, body: bytes, studio_request_id: str) -> tuple[HTTPStatus, dict[str, Any]]:
        if not studio_request_id or len(studio_request_id) > 128 or any(char in studio_request_id for char in "\r\n\0"):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_voice_request_id", "Studio voice request ID is required.")
        self._validate_speech_wav(body)
        identifier = secrets.token_urlsafe(24)
        try:
            self.speech_spool.mkdir(parents=True, exist_ok=True, mode=0o700)
            path = self.speech_spool / f"{identifier}.wav"
            temporary = self.speech_spool / f".{identifier}.{secrets.token_hex(8)}.tmp"
            descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            try:
                with os.fdopen(descriptor, "wb") as output:
                    output.write(body)
                    output.flush()
                    os.fsync(output.fileno())
                os.replace(temporary, path)
            finally:
                temporary.unlink(missing_ok=True)
        except OSError as error:
            raise GatewayError(HTTPStatus.INTERNAL_SERVER_ERROR, "speech_spool_failed", f"Could not spool speech: {error}") from error
        try:
            runtime = self._checked(f"speech file {identifier}")
        except GatewayError:
            path.unlink(missing_ok=True)
            raise
        return HTTPStatus.ACCEPTED, {
            "api_version": API_VERSION,
            "accepted": True,
            "studio_voice_request_id": studio_request_id,
            "run_id": runtime.get("run_id"),
            "state": runtime.get("state", "queued"),
        }

    def speech_run_status(self, run_id: int) -> dict[str, Any]:
        if isinstance(run_id, bool) or run_id <= 0:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_run_id", "Speech run ID must be positive.")
        status = self._checked("speech status")
        for key in ("speech", "last_speech"):
            speech = status.get(key)
            if isinstance(speech, dict) and speech.get("run_id") == run_id:
                state = speech.get("state")
                if state == "synthesizing": state = "queued"
                return {"api_version": API_VERSION, "run_id": run_id, "state": state, "error": speech.get("error")}
        raise GatewayError(HTTPStatus.NOT_FOUND, "speech_run_not_found", f"Speech run {run_id} is not active or the most recent result.")

    def compile_trajectory_preview(self, payload: Any) -> dict[str, Any]:
        """Return the Rust compiler's calibrated 50 Hz sample document."""

        if self.project_root is None:
            raise GatewayError(HTTPStatus.NOT_IMPLEMENTED, "trajectory_preview_unavailable", "The gateway has no Orion project root.")
        if not isinstance(payload, dict) or not set(payload) <= {"motion", "document", "start_pose", "anchor_pose"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_trajectory_preview", "Trajectory preview accepts one motion name or v2 document, start_pose, and optional anchor_pose.")
        if ("motion" in payload) == ("document" in payload):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_trajectory_preview", "Provide exactly one motion name or v2 motion document.")
        document = payload.get("document")
        motion = self._validate_motion_document(document) if document is not None else self._name(payload.get("motion"), "motion")
        start_pose = self._name(payload.get("start_pose"), "pose")
        anchor_value = payload.get("anchor_pose")
        anchor_pose = self._name(anchor_value, "pose") if anchor_value is not None else None
        temporary: tempfile.TemporaryDirectory[str] | None = None
        try:
            motions_directory = self.project_root / "motion/motions"
            if document is not None:
                temporary = tempfile.TemporaryDirectory(prefix="orion-studio-motion-")
                motions_directory = Path(temporary.name)
                (motions_directory / f"{motion}.yaml").write_text(
                    json.dumps(document, separators=(",", ":"), ensure_ascii=False, allow_nan=False),
                    encoding="utf-8",
                )
            command = [
                str(self._trajectory_compiler_path()),
                "--motion", motion,
                "--start-pose", start_pose,
                "--pose-file", str(self.project_root / "motion/config/poses.yaml"),
                "--motions-directory", str(motions_directory),
                "--calibration", str(self.calibration_file),
                "--control-rate-hz", "50",
            ]
            if anchor_pose is not None:
                command.extend(("--anchor-pose", anchor_pose))
            completed = subprocess.run(
                command,
                cwd=self.project_root,
                check=False,
                capture_output=True,
                text=True,
                timeout=15,
            )
        except (OSError, ValueError, subprocess.TimeoutExpired) as error:
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "trajectory_compiler_unavailable", f"Could not run the Rust trajectory compiler: {error}") from error
        finally:
            if temporary is not None:
                temporary.cleanup()
        if completed.returncode != 0:
            detail = completed.stderr.strip() or "The requested motion could not be compiled."
            raise GatewayError(HTTPStatus.UNPROCESSABLE_ENTITY, "trajectory_compile_failed", detail[:1000])
        try:
            document = json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            raise GatewayError(HTTPStatus.BAD_GATEWAY, "invalid_trajectory_compiler_response", "The Rust trajectory compiler returned invalid JSON.") from error
        if not isinstance(document, dict) or document.get("format_version") != 2 or document.get("compiler") != "orion-runtime":
            raise GatewayError(HTTPStatus.BAD_GATEWAY, "invalid_trajectory_compiler_response", "The Rust trajectory compiler did not return a v2 document.")
        return document

    def _trajectory_compiler_path(self) -> Path:
        candidates = [
            self.trajectory_compiler,
            self.project_root / "runtime/target/release/orion-trajectory" if self.project_root else None,
            self.project_root / "runtime/target/debug/orion-trajectory" if self.project_root else None,
            Path("/usr/local/bin/orion-trajectory"),
        ]
        configured = os.environ.get("ORION_TRAJECTORY_COMPILER")
        if configured:
            candidates.insert(0, Path(configured))
        discovered = shutil.which("orion-trajectory")
        if discovered:
            candidates.append(Path(discovered))
        for candidate in candidates:
            if candidate is not None and candidate.is_file() and os.access(candidate, os.X_OK):
                return candidate
        raise GatewayError(HTTPStatus.NOT_IMPLEMENTED, "trajectory_compiler_unavailable", "Install the orion-trajectory Rust binary with the runtime deployment.")

    @staticmethod
    def _validate_speech_wav(body: bytes) -> None:
        if not body or len(body) > MAX_SPEECH_BYTES:
            raise GatewayError(HTTPStatus.REQUEST_ENTITY_TOO_LARGE, "invalid_speech_size", "Speech WAV must be between 1 byte and 8 MiB.")
        try:
            with wave.open(BytesIO(body), "rb") as wav:
                channels = wav.getnchannels()
                sample_width = wav.getsampwidth()
                sample_rate = wav.getframerate()
                frames = wav.getnframes()
                compression = wav.getcomptype()
        except (wave.Error, EOFError) as error:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_speech_wav", "Speech body must be a valid RIFF/WAV file.") from error
        duration = frames / sample_rate if sample_rate else float("inf")
        if (channels, sample_width, sample_rate, compression) != (1, 2, 24_000, "NONE") or not 0 < duration <= 120.0:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_speech_format", "Speech WAV must be PCM16 mono at 24 kHz and no longer than 120 seconds.")

    def _prepare_movement(self) -> dict[str, Any]:
        status = self.client.request("status")
        mode = status.get("mode")
        if mode == "observe":
            self._checked("configure")
            mode = "configured"
        if mode == "configured":
            return self._checked("enable")
        if mode == "holding":
            return {"ok": True, "command": "prepare_movement", "mode": "holding", "already_prepared": True}
        raise GatewayError(
            HTTPStatus.CONFLICT,
            "movement_busy",
            f"Orion cannot prepare movement while runtime mode is '{mode}'.",
        )

    def _release_movement(self) -> dict[str, Any]:
        status = self.client.request("status")
        scenes = self._checked("scene status")
        mode = status.get("mode")
        if mode == "moving" or status.get("motion") is not None or scenes.get("scene") is not None:
            raise GatewayError(
                HTTPStatus.CONFLICT,
                "movement_busy",
                "Cancel the active scene or movement before releasing torque.",
            )
        if status.get("torque_enabled") is True:
            return self._checked("disable")
        return {"ok": True, "command": "release_movement", "mode": mode, "already_released": True}

    def publish_scene(self, document: Any) -> tuple[HTTPStatus, dict[str, Any]]:
        with self.scene_write_lock:
            return self._publish_scene_locked(document)

    def _publish_scene_locked(self, document: Any) -> tuple[HTTPStatus, dict[str, Any]]:
        if self.project_root is None:
            raise GatewayError(
                HTTPStatus.NOT_IMPLEMENTED,
                "scene_publish_unavailable",
                "The gateway was not configured with an Orion project root.",
            )
        name = self._validate_scene_document(document)
        scenes_directory = self.project_root / "scenes"
        for extension in ("yaml", "yml"):
            if (scenes_directory / f"{name}.{extension}").exists():
                raise GatewayError(
                    HTTPStatus.CONFLICT,
                    "built_in_scene",
                    f"'{name}' is a built-in scene and cannot be replaced.",
                )

        user_directory = scenes_directory / "user"
        try:
            user_directory.mkdir(parents=True, exist_ok=True)
            encoded = self._encode_scene_document(document)
        except (OSError, ValueError) as error:
            raise GatewayError(
                HTTPStatus.INTERNAL_SERVER_ERROR,
                "scene_write_failed",
                f"Could not prepare the user scene: {error}",
            ) from error

        path = user_directory / f"{name}.yaml"
        if path.exists():
            try:
                identical = path.read_bytes() == encoded
            except OSError as error:
                raise GatewayError(
                    HTTPStatus.INTERNAL_SERVER_ERROR,
                    "scene_read_failed",
                    f"Could not read existing user scene '{name}': {error}",
                ) from error
            if not identical:
                raise GatewayError(
                    HTTPStatus.CONFLICT,
                    "user_scene_exists",
                    f"A different user scene named '{name}' already exists; choose a new name.",
                )
            reload_result = self._checked("scene reload")
            return HTTPStatus.OK, {
                "api_version": API_VERSION,
                "published": True,
                "already_present": True,
                "name": name,
                "revision": self._revision(encoded),
                "relative_path": f"scenes/user/{name}.yaml",
                "reload": reload_result,
            }

        created = False
        try:
            descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            created = True
            with os.fdopen(descriptor, "wb") as output:
                output.write(encoded)
                output.flush()
                os.fsync(output.fileno())
            reload_result = self._checked("scene reload")
        except GatewayError:
            if created:
                path.unlink(missing_ok=True)
            raise
        except OSError as error:
            if created:
                path.unlink(missing_ok=True)
            raise GatewayError(
                HTTPStatus.INTERNAL_SERVER_ERROR,
                "scene_write_failed",
                f"Could not publish user scene '{name}': {error}",
            ) from error

        return HTTPStatus.CREATED, {
            "api_version": API_VERSION,
            "published": True,
            "already_present": False,
            "name": name,
            "revision": self._revision(encoded),
            "relative_path": f"scenes/user/{name}.yaml",
            "reload": reload_result,
        }

    def list_user_scenes(self) -> dict[str, Any]:
        directory = self._user_scene_directory()
        if not directory.exists():
            return {"api_version": API_VERSION, "scenes": []}

        entries: list[dict[str, Any]] = []
        try:
            paths = sorted(
                path for path in directory.iterdir()
                if path.suffix in {".yaml", ".yml"}
            )
        except OSError as error:
            raise GatewayError(
                HTTPStatus.INTERNAL_SERVER_ERROR,
                "scene_library_read_failed",
                f"Could not read the Pi user scene library: {error}",
            ) from error

        for path in paths:
            data = self._read_user_scene_file(path)
            name = self._name(path.stem, "scene")
            entries.append({
                "name": name,
                "revision": self._revision(data),
                "bytes": len(data),
                "relative_path": f"scenes/user/{path.name}",
            })
        return {"api_version": API_VERSION, "scenes": entries}

    def read_user_scene(self, name: Any) -> dict[str, Any]:
        name = self._name(name, "scene")
        path = self._existing_user_scene_path(name)
        data = self._read_user_scene_file(path)
        try:
            yaml = data.decode("utf-8")
        except UnicodeDecodeError as error:
            raise GatewayError(
                HTTPStatus.INTERNAL_SERVER_ERROR,
                "invalid_scene_encoding",
                f"User scene '{name}' is not valid UTF-8.",
            ) from error
        return {
            "api_version": API_VERSION,
            "name": name,
            "revision": self._revision(data),
            "relative_path": f"scenes/user/{path.name}",
            "yaml": yaml,
        }

    def update_user_scene(self, name: Any, payload: Any) -> tuple[HTTPStatus, dict[str, Any]]:
        with self.scene_write_lock:
            return self._update_user_scene_locked(name, payload)

    def _update_user_scene_locked(self, name: Any, payload: Any) -> tuple[HTTPStatus, dict[str, Any]]:
        name = self._name(name, "scene")
        if not isinstance(payload, dict) or set(payload) != {"expected_revision", "document"}:
            raise GatewayError(
                HTTPStatus.BAD_REQUEST,
                "invalid_scene_update",
                "Expected expected_revision and document fields.",
            )
        expected = payload.get("expected_revision")
        if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
            raise GatewayError(
                HTTPStatus.BAD_REQUEST,
                "invalid_scene_revision",
                "A valid scene revision is required.",
            )
        document = payload.get("document")
        document_name = self._validate_scene_document(document)
        if document_name != name:
            raise GatewayError(
                HTTPStatus.BAD_REQUEST,
                "scene_name_mismatch",
                "The URL scene name must match the document scene name.",
            )

        path = self._existing_user_scene_path(name)
        previous = self._read_user_scene_file(path)
        actual = self._revision(previous)
        if actual != expected:
            raise GatewayError(
                HTTPStatus.CONFLICT,
                "scene_revision_conflict",
                f"User scene '{name}' changed on the Pi; reload it before saving.",
            )
        encoded = self._encode_scene_document(document)
        if encoded == previous:
            return HTTPStatus.OK, {
                "api_version": API_VERSION,
                "updated": False,
                "name": name,
                "revision": actual,
                "relative_path": f"scenes/user/{path.name}",
            }

        try:
            self._atomic_replace(path, encoded)
            reload_result = self._checked("scene reload")
        except GatewayError as publish_error:
            try:
                self._atomic_replace(path, previous)
                self._checked("scene reload")
            except (GatewayError, OSError) as rollback_error:
                raise GatewayError(
                    HTTPStatus.INTERNAL_SERVER_ERROR,
                    "scene_rollback_failed",
                    f"Could not restore user scene '{name}' after a rejected update: {rollback_error}",
                ) from rollback_error
            raise publish_error
        except OSError as error:
            raise GatewayError(
                HTTPStatus.INTERNAL_SERVER_ERROR,
                "scene_write_failed",
                f"Could not update user scene '{name}': {error}",
            ) from error

        return HTTPStatus.OK, {
            "api_version": API_VERSION,
            "updated": True,
            "name": name,
            "revision": self._revision(encoded),
            "relative_path": f"scenes/user/{path.name}",
            "reload": reload_result,
        }

    def list_user_poses(self) -> dict[str, Any]:
        return self._list_user_assets(self._user_pose_directory(), "motion/user/poses")

    def read_user_pose(self, name: Any) -> dict[str, Any]:
        return self._read_user_asset(
            self._user_pose_directory(),
            "motion/user/poses",
            self._name(name, "pose"),
        )

    def publish_pose(self, document: Any) -> tuple[HTTPStatus, dict[str, Any]]:
        name = self._validate_pose_document(document)
        limits = self._checked("joint limits").get("joints", [])
        self._validate_pose_limits(document, limits)
        return self._publish_immutable_asset(
            kind="pose",
            name=name,
            document=document,
            directory=self._user_pose_directory(),
            relative_directory="motion/user/poses",
            existing_names=self._checked("pose list").get("poses", []),
        )

    def list_user_motions(self) -> dict[str, Any]:
        return self._list_user_assets(self._user_motion_directory(), "motion/motions/user")

    def read_user_motion(self, name: Any) -> dict[str, Any]:
        return self._read_user_asset(
            self._user_motion_directory(),
            "motion/motions/user",
            self._name(name, "motion"),
        )

    def publish_motion(self, document: Any) -> tuple[HTTPStatus, dict[str, Any]]:
        name = self._validate_motion_document(document)
        return self._publish_immutable_asset(
            kind="motion",
            name=name,
            document=document,
            directory=self._user_motion_directory(),
            relative_directory="motion/motions/user",
            existing_names=self._checked("motion list").get("motions", []),
        )

    def _publish_immutable_asset(
        self,
        *,
        kind: str,
        name: str,
        document: Any,
        directory: Path,
        relative_directory: str,
        existing_names: Any,
    ) -> tuple[HTTPStatus, dict[str, Any]]:
        with self.scene_write_lock:
            try:
                directory.mkdir(parents=True, exist_ok=True)
                encoded = self._encode_scene_document(document)
            except OSError as error:
                raise GatewayError(
                    HTTPStatus.INTERNAL_SERVER_ERROR,
                    "asset_write_failed",
                    f"Could not prepare the user {kind}: {error}",
                ) from error
            path = directory / f"{name}.yaml"
            relative_path = f"{relative_directory}/{name}.yaml"
            if path.exists():
                current = self._read_user_scene_file(path)
                if current != encoded:
                    raise GatewayError(
                        HTTPStatus.CONFLICT,
                        f"user_{kind}_exists",
                        f"A different user {kind} named '{name}' already exists; choose a new name.",
                    )
                reload_result = self._checked("asset reload")
                return HTTPStatus.OK, {
                    "api_version": API_VERSION,
                    "published": True,
                    "already_present": True,
                    "name": name,
                    "revision": self._revision(encoded),
                    "relative_path": relative_path,
                    "reload": reload_result,
                }
            if isinstance(existing_names, list) and name in existing_names:
                raise GatewayError(
                    HTTPStatus.CONFLICT,
                    f"built_in_{kind}",
                    f"'{name}' is an existing Orion {kind} and cannot be replaced.",
                )

            created = False
            try:
                descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                created = True
                with os.fdopen(descriptor, "wb") as output:
                    output.write(encoded)
                    output.flush()
                    os.fsync(output.fileno())
                reload_result = self._checked("asset reload")
            except GatewayError:
                if created:
                    path.unlink(missing_ok=True)
                raise
            except OSError as error:
                if created:
                    path.unlink(missing_ok=True)
                raise GatewayError(
                    HTTPStatus.INTERNAL_SERVER_ERROR,
                    "asset_write_failed",
                    f"Could not publish user {kind} '{name}': {error}",
                ) from error
            return HTTPStatus.CREATED, {
                "api_version": API_VERSION,
                "published": True,
                "already_present": False,
                "name": name,
                "revision": self._revision(encoded),
                "relative_path": relative_path,
                "reload": reload_result,
            }

    def _list_user_assets(self, directory: Path, relative_directory: str) -> dict[str, Any]:
        if not directory.exists():
            return {"api_version": API_VERSION, "assets": []}
        try:
            paths = sorted(path for path in directory.iterdir() if path.suffix in {".yaml", ".yml"})
        except OSError as error:
            raise GatewayError(
                HTTPStatus.INTERNAL_SERVER_ERROR,
                "asset_library_read_failed",
                f"Could not read the Pi user asset library: {error}",
            ) from error
        assets = []
        for path in paths:
            data = self._read_user_scene_file(path)
            assets.append({
                "name": self._name(path.stem, "asset"),
                "revision": self._revision(data),
                "bytes": len(data),
                "relative_path": f"{relative_directory}/{path.name}",
            })
        return {"api_version": API_VERSION, "assets": assets}

    def _read_user_asset(self, directory: Path, relative_directory: str, name: str) -> dict[str, Any]:
        path = self._existing_named_yaml(directory, name, "asset")
        data = self._read_user_scene_file(path)
        try:
            yaml = data.decode("utf-8")
        except UnicodeDecodeError as error:
            raise GatewayError(
                HTTPStatus.INTERNAL_SERVER_ERROR,
                "invalid_asset_encoding",
                f"User asset '{name}' is not valid UTF-8.",
            ) from error
        return {
            "api_version": API_VERSION,
            "name": name,
            "revision": self._revision(data),
            "relative_path": f"{relative_directory}/{path.name}",
            "yaml": yaml,
        }

    def _user_scene_directory(self) -> Path:
        if self.project_root is None:
            raise GatewayError(
                HTTPStatus.NOT_IMPLEMENTED,
                "scene_library_unavailable",
                "The gateway was not configured with an Orion project root.",
            )
        return self.project_root / "scenes" / "user"

    def _user_pose_directory(self) -> Path:
        if self.project_root is None:
            raise GatewayError(
                HTTPStatus.NOT_IMPLEMENTED,
                "pose_library_unavailable",
                "The gateway was not configured with an Orion project root.",
            )
        return self.project_root / "motion" / "user" / "poses"

    def _user_motion_directory(self) -> Path:
        if self.project_root is None:
            raise GatewayError(
                HTTPStatus.NOT_IMPLEMENTED,
                "motion_library_unavailable",
                "The gateway was not configured with an Orion project root.",
            )
        return self.project_root / "motion" / "motions" / "user"

    def _existing_user_scene_path(self, name: str) -> Path:
        return self._existing_named_yaml(self._user_scene_directory(), name, "user scene")

    @staticmethod
    def _existing_named_yaml(directory: Path, name: str, label: str) -> Path:
        matches = [directory / f"{name}.{extension}" for extension in ("yaml", "yml")]
        matches = [path for path in matches if path.exists()]
        if not matches:
            raise GatewayError(
                HTTPStatus.NOT_FOUND,
                "user_asset_not_found",
                f"{label.title()} '{name}' does not exist on the Pi.",
            )
        if len(matches) != 1:
            raise GatewayError(
                HTTPStatus.CONFLICT,
                "duplicate_user_asset",
                f"{label.title()} '{name}' has duplicate YAML files on the Pi.",
            )
        return matches[0]

    @staticmethod
    def _read_user_scene_file(path: Path) -> bytes:
        try:
            if path.is_symlink() or not path.is_file():
                raise GatewayError(
                    HTTPStatus.CONFLICT,
                    "unsafe_scene_file",
                    f"User scene '{path.name}' must be a regular file.",
                )
            if path.stat().st_size > MAX_BODY_BYTES:
                raise GatewayError(
                    HTTPStatus.REQUEST_ENTITY_TOO_LARGE,
                    "scene_file_too_large",
                    f"User scene '{path.name}' is too large.",
                )
            return path.read_bytes()
        except GatewayError:
            raise
        except OSError as error:
            raise GatewayError(
                HTTPStatus.INTERNAL_SERVER_ERROR,
                "scene_read_failed",
                f"Could not read user scene '{path.name}': {error}",
            ) from error

    @staticmethod
    def _encode_scene_document(document: Any) -> bytes:
        try:
            return (json.dumps(document, indent=2, allow_nan=False) + "\n").encode("utf-8")
        except (TypeError, ValueError) as error:
            raise GatewayError(
                HTTPStatus.BAD_REQUEST,
                "invalid_scene",
                f"Could not encode the scene document: {error}",
            ) from error

    @staticmethod
    def _revision(data: bytes) -> str:
        return hashlib.sha256(data).hexdigest()

    @staticmethod
    def _atomic_replace(path: Path, data: bytes) -> None:
        temporary = path.with_name(f".{path.name}.{secrets.token_hex(8)}.tmp")
        descriptor: int | None = None
        try:
            descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "wb") as output:
                descriptor = None
                output.write(data)
                output.flush()
                os.fsync(output.fileno())
            os.replace(temporary, path)
        finally:
            if descriptor is not None:
                os.close(descriptor)
            temporary.unlink(missing_ok=True)

    @classmethod
    def _validate_pose_document(cls, document: Any) -> str:
        if not isinstance(document, dict) or set(document) != {"format_version", "units", "poses"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_pose", "Expected a versioned pose document.")
        if document.get("format_version") != 2 or document.get("units") != "radians":
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_pose", "Pose format_version must be 2 with radian units (v2 required).")
        poses = document.get("poses")
        if not isinstance(poses, dict) or len(poses) != 1:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_pose", "A user pose file must contain exactly one pose.")
        name, pose = next(iter(poses.items()))
        name = cls._name(name, "pose")
        allowed = {"description", "tags", "idle_profile", "default_lighting", "positions"}
        if not isinstance(pose, dict) or not set(pose) <= allowed or "positions" not in pose:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_pose", "Pose contains unsupported v2 fields or omits positions.")
        if not isinstance(pose.get("description"), str):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_pose", "Pose description must be text.")
        positions = pose.get("positions")
        if not isinstance(positions, dict) or set(positions) != set(JOINT_NAMES):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_pose", "Pose positions must contain every Orion joint exactly once.")
        for joint, value in positions.items():
            if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_pose", f"Pose position for {joint} must be finite.")
        return name

    @classmethod
    def _validate_pose_limits(cls, document: dict[str, Any], limits: Any) -> None:
        if not isinstance(limits, list):
            raise GatewayError(HTTPStatus.BAD_GATEWAY, "invalid_joint_limits", "oriond returned invalid joint limits.")
        ranges: dict[str, tuple[float, float]] = {}
        for limit in limits:
            if not isinstance(limit, dict):
                continue
            name = limit.get("name")
            lower = limit.get("lower_rad")
            upper = limit.get("upper_rad")
            if (
                name in JOINT_NAMES
                and isinstance(lower, (int, float))
                and not isinstance(lower, bool)
                and isinstance(upper, (int, float))
                and not isinstance(upper, bool)
                and math.isfinite(lower)
                and math.isfinite(upper)
                and lower < upper
            ):
                ranges[name] = (float(lower), float(upper))
        if set(ranges) != set(JOINT_NAMES):
            raise GatewayError(HTTPStatus.BAD_GATEWAY, "invalid_joint_limits", "oriond omitted Orion joint limits.")
        pose = next(iter(document["poses"].values()))
        for name, value in pose["positions"].items():
            lower, upper = ranges[name]
            if not lower <= float(value) <= upper:
                raise GatewayError(
                    HTTPStatus.BAD_REQUEST,
                    "pose_out_of_range",
                    f"Pose position for {name} must stay between {lower:.6f} and {upper:.6f} radians.",
                )

    @classmethod
    def _validate_motion_document(cls, document: Any) -> str:
        if not isinstance(document, dict) or set(document) != {"format_version", "motion"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Expected a versioned motion document.")
        if document.get("format_version") != 2 or not isinstance(document.get("motion"), dict):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Motion format_version must be 2 (v2 required).")
        motion = document["motion"]
        allowed = {"name", "description", "space", "style", "return_to_anchor", "keyframes"}
        required = {"name", "description", "space", "style", "keyframes"}
        if not required <= set(motion) or not set(motion) <= allowed:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Motion contains unsupported or missing v2 fields.")
        name = cls._name(motion.get("name"), "motion")
        if not isinstance(motion.get("description"), str):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Motion description must be text.")
        space = motion.get("space")
        if space not in {"absolute", "anchor_relative"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Motion space must be absolute or anchor_relative.")
        if motion.get("style") not in {"living_idle", "attentive", "expressive_turn", "speaking_calm", "speaking_emphatic", "thinking", "quick_reaction", "return_home"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Motion style is not in Orion's character style library.")
        if space == "anchor_relative" and motion.get("return_to_anchor") is not True:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Anchor-relative motion must return_to_anchor.")
        if space == "absolute" and motion.get("return_to_anchor") not in {None, False}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Absolute motion cannot return_to_anchor.")
        keyframes = motion.get("keyframes")
        if not isinstance(keyframes, list) or not keyframes:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "A motion must contain at least one keyframe.")
        for keyframe in keyframes:
            allowed_keyframe = {"pose", "offsets", "duration", "arrival", "hold", "marker"}
            if not isinstance(keyframe, dict) or not set(keyframe) <= allowed_keyframe or not {"duration", "arrival"} <= set(keyframe):
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Motion keyframe contains invalid v2 fields.")
            if space == "absolute":
                if "offsets" in keyframe: raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Absolute keyframes use pose only.")
                cls._name(keyframe.get("pose"), "pose")
            else:
                if "pose" in keyframe or not isinstance(keyframe.get("offsets", {}), dict):
                    raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Relative keyframes use offset mappings only.")
                if not set(keyframe.get("offsets", {})) <= set(JOINT_NAMES):
                    raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Relative keyframe contains an unknown joint.")
            duration = cls._number(keyframe.get("duration"), "Keyframe duration", minimum=0.000001)
            hold = cls._number(keyframe.get("hold", 0.0), "Keyframe hold", minimum=0.0)
            if keyframe.get("arrival") not in {"through", "settle"} or keyframe.get("arrival") == "through" and hold > 0:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Through keyframes cannot hold; arrival must be through or settle.")
            if duration > 300.0 or hold > 300.0:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Keyframe timing cannot exceed 300 seconds.")
        if keyframes[-1].get("arrival") != "settle":
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Final motion keyframe must settle.")
        if space == "anchor_relative" and any(abs(float(value)) > 1e-12 for value in keyframes[-1].get("offsets", {}).values()):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Relative motion must finish at zero offsets.")
        return name

    @classmethod
    def _validate_scene_document(cls, document: Any) -> str:
        if not isinstance(document, dict) or set(document) != {"format_version", "scene"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Expected a versioned scene document.")
        if document.get("format_version") != 2 or not isinstance(document.get("scene"), dict):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Scene format_version must be 2 (v2 required).")
        scene = document["scene"]
        if set(scene) != {"name", "description", "motion", "lighting", "audio", "finish"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Scene must contain v2 parallel motion, lighting, audio, and finish tracks.")
        name = cls._name(scene.get("name"), "scene")
        if not isinstance(scene.get("description"), str):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Scene description must be text.")
        motion = scene.get("motion")
        lighting = scene.get("lighting")
        audio = scene.get("audio")
        if not all(isinstance(track, list) for track in (motion, lighting, audio)) or not (motion or lighting or audio):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Scene tracks must be lists with at least one event overall.")
        previous_at = -1.0
        for event in motion:
            if not isinstance(event, dict) or set(event) != {"at", "play"}:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Motion track events require at and play.")
            at = cls._number(event.get("at"), "Motion time", minimum=0.0)
            if at < previous_at: raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Motion track times must be ordered.")
            previous_at = at
            cls._name(event.get("play"), "motion")
        effects = {"warm_idle_breathe", "attentive_focus", "thinking_drift", "speaking_energy", "acknowledge_pulse", "curious_sweep", "delight_spark", "settle_glow", "off"}
        for event in lighting:
            if not isinstance(event, dict) or not set(event) <= {"at", "on_marker", "effect", "intensity", "duration", "transition", "palette"}:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Lighting track event contains invalid fields.")
            if ("at" in event) == ("on_marker" in event): raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Lighting event requires exactly one trigger.")
            if "at" in event: cls._number(event["at"], "Lighting time", minimum=0.0)
            else: cls._name(event["on_marker"], "marker")
            if event.get("effect") not in effects: raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Unknown Orion lighting effect.")
            cls._number(event.get("intensity", 1.0), "Lighting intensity", minimum=0.0)
            cls._number(event.get("duration", 0.8), "Lighting duration", minimum=0.0)
            cls._number(event.get("transition", 0.0), "Lighting transition", minimum=0.0)
        for event in audio:
            if not isinstance(event, dict) or not set(event) <= {"at", "on_marker", "cue"} or "cue" not in event:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Audio track event contains invalid fields.")
            if ("at" in event) == ("on_marker" in event): raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Audio event requires exactly one trigger.")
            if "at" in event: cls._number(event["at"], "Audio time", minimum=0.0)
            else: cls._name(event["on_marker"], "marker")
            cls._name(event.get("cue"), "audio cue")
        if scene.get("finish") != {"anchor": "final_pose", "lighting": "pose_default"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Scene finish must restore final_pose and pose_default.")
        return name

    @staticmethod
    def _number(value: Any, label: str, minimum: float) -> float:
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < minimum:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", f"{label} is invalid.")
        return float(value)

    def _cancel(self, payload: dict[str, Any]) -> dict[str, Any]:
        kind = payload.get("kind")
        run_id = payload.get("run_id")
        if kind not in {"movement", "scene", "speech"}:
            raise GatewayError(
                HTTPStatus.BAD_REQUEST,
                "invalid_cancel_kind",
                "Cancel kind must be movement, scene, or speech.",
            )
        if isinstance(run_id, bool) or not isinstance(run_id, int) or run_id <= 0:
            raise GatewayError(
                HTTPStatus.BAD_REQUEST, "invalid_run_id", "Cancel requires a positive run_id."
            )

        snapshot = self.status()
        if kind == "movement":
            if snapshot["scene"]["active"] is not None:
                raise GatewayError(
                    HTTPStatus.CONFLICT,
                    "scene_owns_movement",
                    "Cancel the active scene instead of its internal movement.",
                )
            active = snapshot["runtime"].get("motion")
            command = "stop"
        else:
            active = snapshot[kind]["active"]
            command = f"{kind} stop"
        if not isinstance(active, dict) or active.get("run_id") != run_id:
            raise GatewayError(
                HTTPStatus.CONFLICT,
                "run_not_active",
                f"{kind} run {run_id} is not active; no cancellation was sent.",
            )
        return self._checked(command)

    def _checked(self, command: str) -> dict[str, Any]:
        response = self.client.request(command)
        if response.get("ok") is False:
            raise GatewayError(
                HTTPStatus.CONFLICT,
                "runtime_rejected",
                str(response.get("error", "oriond rejected the operation.")),
            )
        return response

    @staticmethod
    def _name(value: Any, label: str) -> str:
        if not isinstance(value, str) or not NAME_PATTERN.fullmatch(value):
            raise GatewayError(
                HTTPStatus.BAD_REQUEST,
                f"invalid_{label}_name",
                f"A valid named Orion {label} is required.",
            )
        return value


def make_handler(gateway: OrionGateway, token: str, allowed_origins: str | list[str] | tuple[str, ...]):
    origin_allowlist = {allowed_origins} if isinstance(allowed_origins, str) else set(allowed_origins)

    class GatewayHandler(BaseHTTPRequestHandler):
        server_version = "OrionStudioGateway/2"

        def do_OPTIONS(self) -> None:
            self.send_response(HTTPStatus.NO_CONTENT)
            self._cors_headers()
            self.send_header("Access-Control-Allow-Headers", "Authorization, Content-Type, X-Orion-Voice-Request-ID")
            self.send_header("Access-Control-Allow-Methods", "GET, POST, PUT, OPTIONS")
            self.end_headers()

        def do_GET(self) -> None:
            self._handle(self._get)

        def do_POST(self) -> None:
            self._handle(self._post)

        def do_PUT(self) -> None:
            self._handle(self._put)

        def _get(self) -> tuple[HTTPStatus, dict[str, Any]]:
            path = urlparse(self.path).path
            if path == "/api/v2/status":
                return HTTPStatus.OK, gateway.status()
            if path == "/api/v2/capabilities":
                return HTTPStatus.OK, gateway.capabilities()
            if path.startswith("/api/v2/speech/"):
                value = path.removeprefix("/api/v2/speech/")
                if not value.isdigit():
                    raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_run_id", "Speech run ID must be numeric.")
                return HTTPStatus.OK, gateway.speech_run_status(int(value))
            if path == "/api/v2/scenes":
                return HTTPStatus.OK, gateway.list_user_scenes()
            if path.startswith("/api/v2/scenes/"):
                return HTTPStatus.OK, gateway.read_user_scene(path.removeprefix("/api/v2/scenes/"))
            if path == "/api/v2/poses":
                return HTTPStatus.OK, gateway.list_user_poses()
            if path.startswith("/api/v2/poses/"):
                return HTTPStatus.OK, gateway.read_user_pose(path.removeprefix("/api/v2/poses/"))
            if path == "/api/v2/motions":
                return HTTPStatus.OK, gateway.list_user_motions()
            if path.startswith("/api/v2/motions/"):
                return HTTPStatus.OK, gateway.read_user_motion(path.removeprefix("/api/v2/motions/"))
            raise GatewayError(HTTPStatus.NOT_FOUND, "not_found", "Unknown Orion Studio endpoint.")

        def _post(self) -> tuple[HTTPStatus, dict[str, Any]]:
            path = urlparse(self.path).path
            if path == "/api/v2/speech":
                request_id = self.headers.get("X-Orion-Voice-Request-ID", "")
                return gateway.upload_speech(self._read_speech_wav(), request_id)
            if path not in {"/api/v2/operations", "/api/v2/trajectory", "/api/v2/scenes", "/api/v2/poses", "/api/v2/motions"}:
                raise GatewayError(HTTPStatus.NOT_FOUND, "not_found", "Unknown Orion Studio endpoint.")
            payload = self._read_json()
            if path == "/api/v2/trajectory":
                return HTTPStatus.OK, gateway.compile_trajectory_preview(payload)
            if path == "/api/v2/scenes":
                return gateway.publish_scene(payload)
            if path == "/api/v2/poses":
                return gateway.publish_pose(payload)
            if path == "/api/v2/motions":
                return gateway.publish_motion(payload)
            return gateway.submit(payload)

        def _put(self) -> tuple[HTTPStatus, dict[str, Any]]:
            path = urlparse(self.path).path
            if not path.startswith("/api/v2/scenes/"):
                raise GatewayError(HTTPStatus.NOT_FOUND, "not_found", "Unknown Orion Studio endpoint.")
            name = path.removeprefix("/api/v2/scenes/")
            return gateway.update_user_scene(name, self._read_json())

        def _read_json(self) -> Any:
            length_text = self.headers.get("Content-Length")
            try:
                length = int(length_text or "")
            except ValueError as error:
                raise GatewayError(HTTPStatus.LENGTH_REQUIRED, "missing_length", "Content-Length is required.") from error
            if not 0 < length <= MAX_BODY_BYTES:
                raise GatewayError(HTTPStatus.REQUEST_ENTITY_TOO_LARGE, "invalid_body_size", "Request body is empty or too large.")
            try:
                return json.loads(self.rfile.read(length))
            except json.JSONDecodeError as error:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_json", "Request body is not valid JSON.") from error

        def _read_speech_wav(self) -> bytes:
            length_text = self.headers.get("Content-Length")
            try:
                length = int(length_text or "")
            except ValueError as error:
                raise GatewayError(HTTPStatus.LENGTH_REQUIRED, "missing_length", "Content-Length is required.") from error
            if not 0 < length <= MAX_SPEECH_BYTES:
                raise GatewayError(HTTPStatus.REQUEST_ENTITY_TOO_LARGE, "invalid_speech_size", "Speech WAV is empty or exceeds 8 MiB.")
            return self.rfile.read(length)

        def _handle(self, action) -> None:
            try:
                authorization = self.headers.get("Authorization", "")
                supplied = authorization[7:] if authorization.startswith("Bearer ") else ""
                if not secrets.compare_digest(supplied, token):
                    raise GatewayError(HTTPStatus.UNAUTHORIZED, "unauthorized", "A valid Studio token is required.")
                status, body = action()
            except GatewayError as error:
                status = error.status
                body = {
                    "api_version": API_VERSION,
                    "error": {"code": error.code, "message": str(error)},
                }
            self._write_json(status, body)

        def _write_json(self, status: HTTPStatus, body: dict[str, Any]) -> None:
            encoded = json.dumps(body, separators=(",", ":")).encode("utf-8")
            self.send_response(status)
            self._cors_headers()
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(encoded)

        def _cors_headers(self) -> None:
            origin = self.headers.get("Origin")
            if origin in origin_allowlist:
                self.send_header("Access-Control-Allow-Origin", origin)
                self.send_header("Vary", "Origin")

        def log_message(self, message: str, *args: Any) -> None:
            print(f"orion-studio-gateway: {self.address_string()} - {message % args}")

    return GatewayHandler


def read_token(path: Path) -> str:
    try:
        mode = stat.S_IMODE(path.stat().st_mode)
        token = path.read_text(encoding="utf-8").strip()
    except OSError as error:
        raise SystemExit(f"Could not read Studio token file '{path}': {error}") from error
    if mode & 0o077:
        raise SystemExit(f"Studio token file '{path}' must not be accessible by group or others.")
    if len(token) < 32:
        raise SystemExit("Studio token must contain at least 32 characters.")
    return token


def create_token(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    token = secrets.token_urlsafe(32)
    try:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            output.write(token + "\n")
    except FileExistsError as error:
        raise SystemExit(f"Refusing to replace existing token file '{path}'.") from error
    print(token)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    token = commands.add_parser("create-token", help="Create a development bearer token.")
    token.add_argument("--token-file", type=Path, required=True)
    serve = commands.add_parser("serve", help="Run the source-tree Studio gateway.")
    serve.add_argument("--socket", default=DEFAULT_SOCKET)
    serve.add_argument("--bind", default="127.0.0.1")
    serve.add_argument("--port", type=int, default=7447)
    serve.add_argument("--token-file", type=Path, required=True)
    serve.add_argument("--calibration", type=Path, default=DEFAULT_CALIBRATION)
    serve.add_argument("--trajectory-compiler", type=Path)
    serve.add_argument(
        "--project-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="Orion source checkout containing scenes/ (default: inferred from gateway.py).",
    )
    serve.add_argument(
        "--allow-origin",
        action="append",
        dest="allowed_origins",
        default=list(DEFAULT_ALLOWED_ORIGINS),
        help="Allowed Studio web origin; may be repeated.",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if args.command == "create-token":
        create_token(args.token_file)
        return
    token = read_token(args.token_file)
    project_root = args.project_root.resolve()
    if not (project_root / "AGENTS.md").is_file() or not (project_root / "scenes").is_dir():
        raise SystemExit(f"Not an Orion project root: '{project_root}'.")
    gateway = OrionGateway(
        UnixOrionClient(args.socket),
        project_root,
        calibration_file=args.calibration,
        trajectory_compiler=args.trajectory_compiler,
    )
    server = ThreadingHTTPServer(
        (args.bind, args.port), make_handler(gateway, token, args.allowed_origins)
    )
    print(f"orion-studio-gateway: serving http://{args.bind}:{args.port} -> {args.socket}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
