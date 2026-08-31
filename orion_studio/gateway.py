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
import socket
import stat
import threading
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import urlparse


API_VERSION = 1
DEFAULT_SOCKET = "/tmp/oriond.sock"
MAX_BODY_BYTES = 262_144
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

    def __init__(self, client: Any, project_root: Path | None = None):
        self.client = client
        self.project_root = project_root
        self.scene_write_lock = threading.Lock()

    def status(self) -> dict[str, Any]:
        runtime = self.client.request("status")
        scenes = self._checked("scene status")
        speech = self._checked("speech status")
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
                "scene_publish": {"format_version": 1, "max_body_bytes": MAX_BODY_BYTES},
                "scene_preview": {
                    "format_version": 1,
                    "max_body_bytes": MAX_PREVIEW_SCENE_BYTES,
                    "persisted": False,
                },
                "scene_library": {"read": True, "create": True, "update": "revision"},
                "joint_limits": limits.get("joints", []),
                "pose_library": {"read": True, "create": True, "update": False},
                "motion_library": {"read": True, "create": True, "update": False},
                "movement_lifecycle": ["prepare", "release"],
                "speech": {"max_text_bytes": 2_000},
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
        elif operation == "cancel":
            response = self._cancel(payload)
        else:
            raise GatewayError(
                HTTPStatus.BAD_REQUEST,
                "unsupported_operation",
                "Supported operations are goto, motion, scene, preview_scene, speech, prepare_movement, release_movement, and cancel.",
            )

        return HTTPStatus.ACCEPTED, {
            "api_version": API_VERSION,
            "accepted": True,
            "operation": operation,
            "result": response,
        }

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
        if document.get("format_version") != 1 or document.get("units") != "radians":
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_pose", "Pose format_version must be 1 with radian units.")
        poses = document.get("poses")
        if not isinstance(poses, dict) or len(poses) != 1:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_pose", "A user pose file must contain exactly one pose.")
        name, pose = next(iter(poses.items()))
        name = cls._name(name, "pose")
        if not isinstance(pose, dict) or set(pose) != {"description", "positions"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_pose", "Pose fields must be description and positions.")
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
        if document.get("format_version") != 1 or not isinstance(document.get("motion"), dict):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Motion format_version must be 1.")
        motion = document["motion"]
        if set(motion) != {"name", "description", "keyframes"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Motion fields must be name, description, and keyframes.")
        name = cls._name(motion.get("name"), "motion")
        if not isinstance(motion.get("description"), str):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Motion description must be text.")
        keyframes = motion.get("keyframes")
        if not isinstance(keyframes, list) or not keyframes:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "A motion must contain at least one keyframe.")
        for keyframe in keyframes:
            if not isinstance(keyframe, dict) or set(keyframe) != {"pose", "duration", "hold"}:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Motion keyframes require pose, duration, and hold.")
            cls._name(keyframe.get("pose"), "pose")
            duration = cls._number(keyframe.get("duration"), "Keyframe duration", minimum=0.000001)
            hold = cls._number(keyframe.get("hold"), "Keyframe hold", minimum=0.0)
            if duration > 300.0 or hold > 300.0:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_motion", "Keyframe timing cannot exceed 300 seconds.")
        return name

    @classmethod
    def _validate_scene_document(cls, document: Any) -> str:
        if not isinstance(document, dict) or set(document) != {"format_version", "scene"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Expected a versioned scene document.")
        if document.get("format_version") != 1 or not isinstance(document.get("scene"), dict):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Scene format_version must be 1.")
        scene = document["scene"]
        if set(scene) != {"name", "description", "timeline"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Scene fields must be name, description, and timeline.")
        name = cls._name(scene.get("name"), "scene")
        if not isinstance(scene.get("description"), str):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Scene description must be text.")
        timeline = scene.get("timeline")
        if not isinstance(timeline, list) or not timeline:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Scene timeline must contain events.")

        previous_at = 0.0
        for index, event in enumerate(timeline):
            if not isinstance(event, dict):
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Each timeline event must be an object.")
            at = event.get("at")
            if isinstance(at, bool) or not isinstance(at, (int, float)) or not math.isfinite(at) or at < previous_at:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Timeline times must be finite, non-negative, and ordered.")
            previous_at = float(at)
            kind = event.get("type")
            if kind == "play_motion":
                if set(event) != {"at", "type", "motion"}:
                    raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Invalid play_motion event fields.")
                cls._name(event.get("motion"), "motion")
            elif kind == "goto_pose":
                if set(event) != {"at", "type", "pose", "duration_seconds"}:
                    raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Invalid goto_pose event fields.")
                cls._name(event.get("pose"), "pose")
                cls._number(event.get("duration_seconds"), "Pose duration", minimum=0.000001)
            elif kind == "light":
                if set(event) != {"at", "type", "red", "green", "blue", "white", "transition_seconds"}:
                    raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Invalid light event fields.")
                for channel in ("red", "green", "blue", "white"):
                    value = event.get(channel)
                    if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= 255:
                        raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", f"Light {channel} must be an integer from 0 to 255.")
                cls._number(event.get("transition_seconds"), "Light transition", minimum=0.0)
            elif kind == "audio":
                if set(event) != {"at", "type", "cue"}:
                    raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Invalid audio event fields.")
                cls._name(event.get("cue"), "audio cue")
            else:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", f"Unsupported scene event type at index {index}.")
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
        server_version = "OrionStudioGateway/0.1"

        def do_OPTIONS(self) -> None:
            self.send_response(HTTPStatus.NO_CONTENT)
            self._cors_headers()
            self.send_header("Access-Control-Allow-Headers", "Authorization, Content-Type")
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
            if path == "/api/v1/status":
                return HTTPStatus.OK, gateway.status()
            if path == "/api/v1/capabilities":
                return HTTPStatus.OK, gateway.capabilities()
            if path == "/api/v1/scenes":
                return HTTPStatus.OK, gateway.list_user_scenes()
            if path.startswith("/api/v1/scenes/"):
                return HTTPStatus.OK, gateway.read_user_scene(path.removeprefix("/api/v1/scenes/"))
            if path == "/api/v1/poses":
                return HTTPStatus.OK, gateway.list_user_poses()
            if path.startswith("/api/v1/poses/"):
                return HTTPStatus.OK, gateway.read_user_pose(path.removeprefix("/api/v1/poses/"))
            if path == "/api/v1/motions":
                return HTTPStatus.OK, gateway.list_user_motions()
            if path.startswith("/api/v1/motions/"):
                return HTTPStatus.OK, gateway.read_user_motion(path.removeprefix("/api/v1/motions/"))
            raise GatewayError(HTTPStatus.NOT_FOUND, "not_found", "Unknown Orion Studio endpoint.")

        def _post(self) -> tuple[HTTPStatus, dict[str, Any]]:
            path = urlparse(self.path).path
            if path not in {"/api/v1/operations", "/api/v1/scenes", "/api/v1/poses", "/api/v1/motions"}:
                raise GatewayError(HTTPStatus.NOT_FOUND, "not_found", "Unknown Orion Studio endpoint.")
            payload = self._read_json()
            if path == "/api/v1/scenes":
                return gateway.publish_scene(payload)
            if path == "/api/v1/poses":
                return gateway.publish_pose(payload)
            if path == "/api/v1/motions":
                return gateway.publish_motion(payload)
            return gateway.submit(payload)

        def _put(self) -> tuple[HTTPStatus, dict[str, Any]]:
            path = urlparse(self.path).path
            if not path.startswith("/api/v1/scenes/"):
                raise GatewayError(HTTPStatus.NOT_FOUND, "not_found", "Unknown Orion Studio endpoint.")
            name = path.removeprefix("/api/v1/scenes/")
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
    gateway = OrionGateway(UnixOrionClient(args.socket), project_root)
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
