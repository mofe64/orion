#!/usr/bin/env python3
"""Small authenticated HTTP adapter for Orion's private Unix command socket."""

from __future__ import annotations

from contextlib import contextmanager, nullcontext

import argparse
import hashlib
import http.client
import ipaddress
import json
import logging
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
import time
import wave
from io import BytesIO
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Callable
from urllib.parse import urlparse


from system_metrics import SystemMetrics, SystemdProperties
from home_assistant import HomeAssistant, HomeError

API_VERSION = 2
# Resolve once so a running gateway stays with its release if a launcher symlink changes.
GATEWAY_PROGRAM = Path(__file__).resolve()
DEFAULT_SOCKET = "/tmp/oriond.sock"
DEFAULT_SPEECH_SPOOL = Path("/tmp/orion-speech-spool")
DEFAULT_CALIBRATION = Path("~/.config/orion/servo_calibration.json")
MAX_BODY_BYTES = 262_144
MAX_SPEECH_BYTES = 8 * 1024 * 1024
MAX_PREVIEW_SCENE_BYTES = 3_000
MAX_RESPONSE_BYTES = 1_048_576
# Includes recovery and closing the voice interval; coordinator keeps its gate
# for 65 s. The cross-process timing test protects that ordering.
AIM_TOTAL_SECONDS = 58.
AIM_RECOVERY_SECONDS = 14.
AIM_VOICE_END_SECONDS = 2.
FIND_TIMEOUT_SECONDS = 24.  # frame wait (5) + child (15) + reap/IPC margin
LOG = logging.getLogger(__name__)
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


SPEECH_CUES = frozenset({"agree", "disagree", "happy", "curious", "thinking", "surprised", "sympathy", "sorry", "shy", "unsure", "laugh"})


def validate_speech_cues(value: str | None) -> str:
    if not value:
        return ""
    if not re.fullmatch(r"[a-z_]{2,16}(,[a-z_]{2,16}){0,3}", value) or any(name not in SPEECH_CUES for name in value.split(",")):
        raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_speech_cues", "Expected at most four canonical speech cues.")
    return value


class GatewayError(Exception):
    def __init__(self, status: HTTPStatus, code: str, message: str):
        super().__init__(message)
        self.status = status
        self.code = code


def installed_release() -> Path:
    root = next((parent for parent in GATEWAY_PROGRAM.parents if (parent / "release.json").exists()), None)
    if root is None:
        raise RuntimeError(f"No installed Orion release found above {GATEWAY_PROGRAM} (missing release.json)")
    manifest = root / "release.json"
    try:
        metadata = json.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise RuntimeError(f"Invalid Orion release metadata at {manifest}: {error}") from error
    revision = metadata.get("revision") if isinstance(metadata, dict) else None
    if not isinstance(revision, str) or not revision.strip():
        raise RuntimeError(f"Missing revision in {manifest}")
    return root


class PairingCodes:
    """One short-lived code shared by all HTTP handlers, independent of audio work."""

    def __init__(self, token: str, speak: Callable[[str], None], clock: Callable[[], float] = time.monotonic):
        self.token = token
        self.speak = speak
        self.clock = clock
        self.lock = threading.Lock()
        self.code: str | None = None
        self.expires_at = 0.0
        self.attempts = 0
        self.last_request: float | None = None

    def request_code(self) -> dict[str, Any]:
        with self.lock:
            now = self.clock()
            if self.last_request is not None and now - self.last_request < 15:
                raise GatewayError(HTTPStatus.TOO_MANY_REQUESTS, "pairing_busy", "Wait 15 seconds before asking the lamp for a new code.")
            self.last_request = now
            code = f"{secrets.randbelow(10**6):06d}"
            self.code = code
            self.expires_at = now + 300
            self.attempts = 0

        # The journal fallback is available even if synthesis or playback fails.
        print(f"Studio pairing code: {code}", flush=True)
        words = ("zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine")
        digits = ", ".join(words[int(digit)] for digit in code)
        response: dict[str, Any] = {"api_version": API_VERSION, "expires_in_seconds": 300}
        try:
            self.speak(f"Studio pairing code. {digits}. Again, {digits}.")
        except Exception as error:
            print(f"Studio pairing code: {code}; speech failed: {error}", flush=True)
            response.update(spoken=False, message="The pairing code is in the gateway log.")
        return response

    def exchange(self, code: Any) -> dict[str, Any]:
        if not isinstance(code, str) or not re.fullmatch(r"[0-9]{6}", code):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_pairing_code", "Enter exactly 6 digits for the pairing code.")
        with self.lock:
            if self.code is None or self.clock() >= self.expires_at:
                self.code = None
                raise GatewayError(HTTPStatus.GONE, "pairing_code_expired", "Ask the lamp for a new code.")
            if not secrets.compare_digest(code, self.code):
                self.attempts += 1
                remaining = 5 - self.attempts
                if remaining == 0:
                    self.code = None
                    message = "That code is wrong. The code was discarded. Ask the lamp for a new code."
                else:
                    message = f"That code is wrong. {remaining} {'try' if remaining == 1 else 'tries'} remain."
                raise GatewayError(HTTPStatus.FORBIDDEN, "pairing_code_wrong", message)
            self.code = None
            return {"api_version": API_VERSION, "token": self.token}


class UnixOrionClient:
    """One-command/one-response client for the Pi-local oriond socket."""

    def __init__(self, socket_path: str = DEFAULT_SOCKET, timeout_seconds: float = 2.0):
        self.socket_path = socket_path
        self.timeout_seconds = timeout_seconds

    def request(self, command: str, *, deadline=None) -> dict[str, Any]:
        if not command or "\n" in command or "\r" in command or "\0" in command:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_command", "Invalid local command.")

        try:
            until = min(deadline if deadline is not None else math.inf,
                        time.monotonic() + self.timeout_seconds)
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
                def remaining():
                    timeout = until - time.monotonic()
                    if timeout <= 0:
                        raise TimeoutError('Runtime request deadline exceeded')
                    client.settimeout(timeout)
                remaining()
                client.connect(self.socket_path)
                remaining()
                client.sendall(command.encode("utf-8") + b"\n")
                client.shutdown(socket.SHUT_WR)
                chunks: list[bytes] = []
                received = 0
                while True:
                    remaining()
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


class CameraViewers:
    """Shared viewing lease, rest monitor and minimum snapshot indication."""
    def __init__(self, request, require_awake=lambda: None, clock=time.monotonic):
        self.request = request
        self.require_awake = require_awake
        self.clock = clock
        self.lock = threading.Lock()
        self.clients = 0
        self.healthy = False
        self.stop = threading.Event()
        self.snapshot_until = 0.

    @contextmanager
    def watching(self, snapshot=False):
        with self.lock:
            if self.clients >= 8:
                raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_busy", "Too many camera viewers.")
            self.require_awake()
            if self.clients and not self.healthy:
                raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_indicator_unavailable", "Orion's camera viewer indicator is unavailable.")
            if self.clients == 0:
                try:
                    self.request("camera-viewer 3")
                except GatewayError as error:
                    raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_indicator_unavailable",
                        "Orion's camera viewer indicator is unavailable.") from error
                self.healthy = True
                self.stop = threading.Event()
                threading.Thread(target=self._renew, args=(self.stop,), daemon=True).start()
            self.clients += 1
        try:
            yield self
        finally:
            with self.lock:
                self.clients -= 1
                if snapshot and self.healthy:
                    try:
                        # Renew after delivery, so even a slow snapshot leaves
                        # three seconds of indication. Other viewers cannot
                        # clear this lease early when they disconnect.
                        self.request("camera-viewer 3")
                    except GatewayError:
                        self.healthy = False
                    finally:
                        # Even if renewal fails, do not explicitly clear the
                        # existing lease immediately after delivering a frame.
                        self.snapshot_until = self.clock() + 3
                if self.clients == 0:
                    self.stop.set()
                    self.healthy = False
                    if self.clock() >= self.snapshot_until:
                        try:
                            self.request("camera-viewer 0")
                        except GatewayError:
                            pass  # The runtime lease expires if cleanup cannot reach it.

    def _renew(self, stop):
        renew_at = self.clock() + 1
        while not stop.wait(.1):
            with self.lock:
                if stop.is_set():
                    return
                try:
                    # One shared status poll, independent of frame rate and
                    # viewer count. Rest or unavailable telemetry ends feeds.
                    self.require_awake()
                    if self.clock() >= renew_at:
                        self.request("camera-viewer 3")
                        renew_at = self.clock() + 1
                except GatewayError:
                    self.healthy = False
                    return

    def require_viewing(self):
        with self.lock:
            if not self.clients or not self.healthy:
                raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_not_viewed",
                    "A healthy camera viewer is required for face boxes.")


class OrionGateway:
    """Allowlisted semantic translation; this class never controls hardware."""

    def __init__(
        self,
        client: Any,
        project_root: Path | None = None,
        speech_spool: Path = DEFAULT_SPEECH_SPOOL,
        calibration_file: Path | None = None,
        trajectory_compiler: Path | None = None,
        hardware: str = "v1",
        vision_socket: str = "/tmp/orion-vision.sock",
        camera_preference_file: Path | None = None,
        tracking_preference_file: Path | None = None,
    ):
        if hardware not in ("v1", "v2"):
            raise ValueError("Hardware must be v1 or v2")
        self.system_metrics = SystemMetrics()
        self.home = HomeAssistant()
        self.hardware = hardware
        self.client = client
        self.project_root = project_root
        self.speech_spool = speech_spool
        self.calibration_file = (calibration_file or (GATEWAY_PROGRAM.parent.parent / "hardware/profiles/v2-joints.json"
                                 if hardware == "v2" else DEFAULT_CALIBRATION)).expanduser()
        self.trajectory_compiler = trajectory_compiler
        self.scene_write_lock = threading.Lock()
        self.vision_socket = vision_socket
        self.camera_preference_file = camera_preference_file or Path.home() / ".config/orion/camera.json"
        self.tracking_preference_file = tracking_preference_file or self.camera_preference_file.with_name('tracking.json')
        self.behaviours_file = self.camera_preference_file.with_name('behaviours.json')
        self.behaviours_lock = threading.Lock()
        self.camera_viewers = CameraViewers(self._checked, self.require_camera_awake)

    def home_request(self, method, path, payload=None):
        try:
            if path == 'status' and method == 'GET':
                result = self.home.status()
                properties = SystemdProperties()
                try:
                    state = properties('orion-home-assistant-install.service').get('ActiveState')
                    result['installing'] = state == 'activating'
                    result['install_failed'] = state == 'failed'
                finally:
                    properties.close()
            elif path == 'install' and method == 'POST':
                try:
                    subprocess.run(['systemctl', '--no-ask-password', 'start', '--no-block', 'orion-home-assistant-install.service'],
                        check=True, capture_output=True, timeout=5)
                except (OSError, subprocess.SubprocessError):
                    raise HomeError('Home installation could not start. Update Orion to install the Home service.', 503) from None
                result = {'installing': True}
            elif path == 'onboarding' and method == 'POST':
                result = self.home.onboarding(payload)
            elif path == 'devices' and method == 'GET':
                result = self.home.devices()
            elif path == 'discovered' and method == 'GET':
                result = self.home.discovered()
            elif path == 'integrations' and method == 'GET':
                result = self.home.rest('/api/config/config_entries/flow_handlers')
            elif path == 'access' and method == 'GET':
                result = self.home.ws('homeassistant/expose_entity/list')
            elif path == 'access' and method == 'POST':
                result = self.home.expose(payload)
            elif path == 'control' and method == 'POST':
                if not isinstance(payload, dict):
                    raise HomeError('Choose a home entity.', 400)
                entities = [entity for area in self.home.devices()['areas'] for entity in area['entities']]
                entity = next((entity for entity in entities if entity['entity_id'] == payload.get('entity_id')), None)
                if entity is None:
                    raise HomeError('That home entity is unavailable.', 404)
                result = self.home.control(entity, payload)
            elif path == 'flows' and method == 'POST':
                result = self.home.flow(method, payload=payload)
            elif path.startswith('flows/') and method in ('GET', 'POST', 'DELETE'):
                result = self.home.flow(method, path.removeprefix('flows/'), payload)
            else:
                raise HomeError('Unknown Home endpoint.', 404)
            return self._home_public(result)
        except HomeError as error:
            raise GatewayError(HTTPStatus(error.status), 'home_unavailable', str(error)) from None
        except (OSError, ValueError, KeyError, TypeError):
            raise GatewayError(HTTPStatus.BAD_GATEWAY, 'home_unavailable', 'Home Assistant could not complete this request.') from None

    def _home_public(self, value):
        credentials = self.home.credentials()
        secrets_to_remove = [secret for secret in credentials.values() if isinstance(secret, str) and secret]
        def clean(item):
            if isinstance(item, str):
                for secret in secrets_to_remove:
                    item = item.replace(secret, '[private]')
                return item
            if isinstance(item, dict):
                return {clean(key): clean(child) for key, child in item.items()}
            if isinstance(item, list):
                return [clean(child) for child in item]
            return item
        return clean(value)

    def require_camera_awake(self):
        try:
            rest = self._checked("character status").get("rest")
        except GatewayError as error:
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_indicator_unavailable",
                "Could not verify Orion's camera viewer indicator.") from error
        state = rest.get("state") if isinstance(rest, dict) else None
        if state in ("going_to_rest", "resting", "waking", "fault"):
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_resting", "Camera viewing is unavailable while Orion rests or wakes.")
        if state not in ("awake", "disabled"):
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_indicator_unavailable",
                "Could not verify Orion's rest state.")

    def camera_is_enabled(self):
        try:
            return json.loads(self.camera_preference_file.read_text()).get("camera_enabled") is True
        except FileNotFoundError:
            return True
        except (OSError, ValueError, AttributeError):
            return False

    def set_camera_enabled(self, payload):
        if self.hardware != "v2":
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_unavailable", "This Orion has no camera.")
        if not isinstance(payload, dict) or type(payload.get("camera_enabled")) is not bool:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_camera_setting", "camera_enabled must be true or false.")
        path = self.camera_preference_file
        temporary = None
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, prefix=".camera-", delete=False) as output:
                temporary = Path(output.name)
                json.dump({"camera_enabled": payload["camera_enabled"]}, output)
                output.flush()
                os.fsync(output.fileno())
            temporary.replace(path)
        except OSError as error:
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_setting_unavailable", "Could not save camera privacy setting.") from error
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
        return {"api_version": API_VERSION, "camera_enabled": payload["camera_enabled"]}

    BEHAVIOURS = ('object_glances', 'mutual_gaze', 'arrival_notice', 'new_object_peek', 'presence_defers_rest',
                  'mind_idle_rhythm', 'listen_lean_in', 'thinking_beats', 'wake_sleep_acts', 'posture_drift', 'act_sounds')
    BEHAVIOUR_DEFAULTS = dict.fromkeys(BEHAVIOURS, False) | dict.fromkeys(
        ('mind_idle_rhythm', 'listen_lean_in', 'thinking_beats', 'wake_sleep_acts', 'act_sounds'), True)

    def get_behaviours(self):
        if self.hardware != 'v2':
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, 'camera_unavailable', 'This Orion has no camera.')
        off = self.BEHAVIOUR_DEFAULTS.copy()
        try:
            value = json.loads(self.behaviours_file.read_text())
            if not isinstance(value, dict) or any(type(item) is not bool for item in value.values()):
                value = off
        except (OSError, ValueError):
            value = off
        return {'api_version': API_VERSION, 'behaviours': {key: value.get(key, self.BEHAVIOUR_DEFAULTS[key]) for key in self.BEHAVIOURS}}

    def set_behaviours(self, payload):
        if (not isinstance(payload, dict) or not payload or any(
                key not in self.BEHAVIOURS or type(value) is not bool for key, value in payload.items())):
            raise GatewayError(HTTPStatus.BAD_REQUEST, 'invalid_behaviour_setting', 'Supply known behaviour names with boolean values.')
        with self.behaviours_lock:
            saved = self.get_behaviours()['behaviours']
            saved.update(payload)
            path, temporary = self.behaviours_file, None
            try:
                path.parent.mkdir(parents=True, exist_ok=True)
                with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, prefix='.behaviours-', delete=False) as output:
                    temporary = Path(output.name)
                    json.dump(saved, output)
                    output.flush()
                    os.fsync(output.fileno())
                temporary.replace(path)
            except OSError as error:
                raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, 'behaviour_setting_unavailable',
                    'Could not save character behaviours.') from error
            finally:
                if temporary is not None:
                    temporary.unlink(missing_ok=True)
            return {'api_version': API_VERSION, 'behaviours': saved}

    def tracking_mode(self):
        try:
            value = json.loads(self.tracking_preference_file.read_text()).get('tracking_mode')
            return value if value in ('off', 'idle', 'faces', 'hands') else 'off'
        except FileNotFoundError:
            return 'faces'
        except (OSError, ValueError, AttributeError):
            return 'off'

    def get_tracking_mode(self):
        if self.hardware != 'v2':
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, 'camera_unavailable', 'This Orion has no camera.')
        return {'api_version': API_VERSION, 'tracking_mode': self.tracking_mode(),
                'camera_enabled': self.camera_is_enabled()}

    def set_tracking_mode(self, payload):
        if self.hardware != 'v2':
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, 'camera_unavailable', 'This Orion has no camera.')
        if not isinstance(payload, dict) or payload.get('tracking_mode') not in ('off', 'idle', 'faces', 'hands'):
            raise GatewayError(HTTPStatus.BAD_REQUEST, 'invalid_tracking_mode',
                'tracking_mode must be off, idle, faces or hands.')
        path = self.tracking_preference_file
        temporary = None
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, prefix='.tracking-', delete=False) as output:
                temporary = Path(output.name)
                json.dump({'tracking_mode': payload['tracking_mode']}, output)
                output.flush()
                os.fsync(output.fileno())
            temporary.replace(path)
        except OSError as error:
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, 'tracking_setting_unavailable',
                'Could not save tracking mode.') from error
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
        return self.get_tracking_mode()

    def open_camera(self, path):
        if self.hardware != "v2":
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_unavailable", "This Orion has no camera.")
        if not self.camera_is_enabled():
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_off", "Camera is disabled.")
        if path in ("/stream", "/frame.jpg"):
            self.require_camera_awake()
        if path == "/detections":
            self.require_camera_awake()
            self.camera_viewers.require_viewing()
        connection = http.client.HTTPConnection("localhost", timeout=2)
        connection.sock = socket.socket(socket.AF_UNIX)
        connection.sock.settimeout(2)
        try:
            connection.sock.connect(self.vision_socket)
            connection.request("GET", path)
            return connection, connection.getresponse()
        except (OSError, http.client.HTTPException) as error:
            connection.close()
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_unavailable", "Orion's camera service is unavailable.") from error

    def find_object(self, category, session=None, *, deadline=None):
        if self.hardware != 'v2' or not self.camera_is_enabled():
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, 'camera_off', 'Camera is unavailable or disabled.')
        until = min(deadline if deadline is not None else math.inf,
                    time.monotonic() + FIND_TIMEOUT_SECONDS)
        connection = http.client.HTTPConnection('localhost', timeout=FIND_TIMEOUT_SECONDS)
        connection.sock = socket.socket(socket.AF_UNIX)
        transport = connection.sock
        response = None
        def remaining():
            timeout = until - time.monotonic()
            if timeout <= 0:
                raise TimeoutError('Object lookup deadline exceeded')
            transport.settimeout(timeout)
        try:
            remaining()
            connection.sock.connect(self.vision_socket)
            payload = {'category': category}
            if session is not None:
                payload['session'] = session
            remaining()
            connection.request('POST', '/find', json.dumps(payload), {'Content-Type': 'application/json'})
            remaining()
            response = connection.getresponse()
            # read1 performs at most one raw read, so a dribbling peer cannot
            # renew the socket timeout indefinitely with partial JSON.
            encoded = bytearray()
            while not response.isclosed() and len(encoded) <= 32768:
                remaining()
                chunk = response.read1(32769 - len(encoded))
                if not chunk:
                    break
                encoded.extend(chunk)
            if len(encoded) > 32768:
                raise ValueError('Oversized object response')
            value = json.loads(encoded)
            if response.status != 200:
                error = value.get('error') if isinstance(value, dict) else None
                message = error.get('message') if isinstance(error, dict) else None
                raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, 'find_unavailable',
                    message if isinstance(message, str) else 'Object lookup was refused.')
            if (not isinstance(value, dict) or value.get('status') not in ('found', 'ambiguous', 'not_found')
                    or not isinstance(value.get('candidates'), list) or len(value['candidates']) > 16
                    or any(not isinstance(candidate, dict) for candidate in value['candidates'])):
                raise ValueError('Invalid object response')
            return {'status': value['status'], 'candidates': value['candidates']}
        except (OSError, http.client.HTTPException, ValueError) as error:
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, 'find_unavailable', 'Object lookup failed.') from error
        finally:
            if response is not None:
                response.close()
            connection.close()

    def aim_light(self, payload):
        if (set(payload) - {'operation', 'category', 'session_id'}
                or not isinstance(payload.get('category'), str) or not 1 <= len(payload['category']) <= 32):
            raise GatewayError(HTTPStatus.BAD_REQUEST, 'invalid_target', 'Expected one COCO category.')
        session = payload.get('session_id')
        if session is not None and (not isinstance(session, str) or re.fullmatch('[0-9a-f]{32}', session) is None):
            raise GatewayError(HTTPStatus.BAD_REQUEST, 'invalid_session', 'Invalid voice session.')
        if self.hardware != 'v2' or not self.camera_is_enabled():
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, 'camera_off', 'Camera is unavailable or disabled.')
        token = secrets.token_hex(16)
        mode = self.get_tracking_mode()['tracking_mode']
        total_deadline = time.monotonic() + AIM_TOTAL_SECONDS
        deadline = total_deadline - AIM_RECOVERY_SECONDS
        recovery_deadline = total_deadline - AIM_VOICE_END_SECONDS
        begun = False
        interval = False
        primary_error = None
        def checked(command, until=deadline):
            if time.monotonic() >= until:
                raise GatewayError(HTTPStatus.GATEWAY_TIMEOUT, 'aim_timeout', 'Aiming timed out.')
            return self._checked(command, deadline=until)
        def work(action, until=deadline, **fields):
            return checked('aim-light ' + json.dumps({'action': action, 'token': token, **fields}), until)
        def find():
            guard()
            # Do not admit inference that could outlive the normal-work budget.
            if deadline - time.monotonic() < FIND_TIMEOUT_SECONDS:
                raise GatewayError(HTTPStatus.GATEWAY_TIMEOUT, 'aim_timeout', 'No time remains for another lookup.')
            return self.find_object(payload['category'], session, deadline=deadline)
        def guard():
            if not self.camera_is_enabled() or self.get_tracking_mode()['tracking_mode'] != mode:
                raise GatewayError(HTTPStatus.CONFLICT, 'camera_changed', 'Camera or tracking changed during aiming.')
            voice = checked('voice status')
            if ((session is not None and voice.get('vision_session') != session)
                    or (session is None and voice['voice'].get('session'))):
                raise GatewayError(HTTPStatus.CONFLICT, 'voice_active', 'The voice-safe aiming interval ended.')
            if time.monotonic() >= deadline:
                raise GatewayError(HTTPStatus.GATEWAY_TIMEOUT, 'aim_timeout', 'Aiming timed out.')
        def wait_movement(result, check=True):
            if result.get('run_id') is None:
                return
            until = min(time.monotonic() + 10, deadline if check else recovery_deadline)
            while time.monotonic() < until:
                if check:
                    guard()
                status = checked('status', until)
                motion = next((item for item in (status.get('motion'), status.get('last_motion'))
                               if item and item.get('run_id') == result['run_id']), None)
                if not motion:
                    raise GatewayError(HTTPStatus.CONFLICT, 'aim_interrupted', 'Aiming lost movement ownership.')
                if motion['state'] == 'completed':
                    return
                if motion['state'] not in ('executing', 'settling'):
                    raise GatewayError(HTTPStatus.CONFLICT, 'aim_interrupted', 'Aiming did not complete.')
                time.sleep(min(.05, max(0, until - time.monotonic())))
            raise GatewayError(HTTPStatus.GATEWAY_TIMEOUT, 'aim_timeout', 'Aiming did not settle.')
        try:
            if session is not None:
                interval = True
                checked(f'voice {session} vision_begin')
            guard()
            begun = True
            work('begin')
            found = find()
            for index in range(3):
                if found['status'] != 'not_found':
                    break
                guard()
                wait_movement(work('scan', index=index))
                found = find()
            guard()
            if found['status'] != 'found':
                return {'status': found['status'], 'candidates': found['candidates'], 'ok': True, 'moved': False}
            if len(found['candidates']) != 1:
                raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, 'invalid_find', 'Expected one object candidate.')
            point = found['candidates'][0].get('point')
            if point is None:
                return {'ok': True, 'status': 'clamped', 'moved': False,
                        'reason': 'The object ray does not meet the desk in front of the camera.'}
            if (not isinstance(point, list) or len(point) != 3
                    or any(type(v) not in (int, float) or not math.isfinite(v) for v in point)):
                raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, 'invalid_find', 'Invalid desk point.')
            result = work('target', point=point, lean=.8, **{'class': payload['category']})
            if result['status'] == 'unreachable':
                return {'ok': True, 'status': 'clamped', 'moved': False, 'reason': 'The desk point is unreachable.'}
            wait_movement(result)
            if work('status').get('committed') is not True:
                raise GatewayError(HTTPStatus.CONFLICT, 'aim_interrupted', 'The aimed pose was not accepted.')
            work('end')
            begun = False
            checked('routines ' + json.dumps({'action': 'set_mode', 'mode': 'lamp'}))
            return {**result, 'moved': True, 'category': payload['category']}
        except BaseException as error:
            primary_error = error
            raise
        finally:
            cleanup_error = None
            recovery_hold_until = min(recovery_deadline,
                time.monotonic() + AIM_RECOVERY_SECONDS - AIM_VOICE_END_SECONDS)
            try:
                if begun:
                    # The owner token makes recovery inert after Stop, rest,
                    # speech, or another foreground action revokes this request.
                    wait_movement(work('end', until=recovery_deadline), check=False)
            except Exception as error:
                cleanup_error = error
                LOG.warning('Aiming recovery failed: %s', error)
                # End has an eight-second runtime lease, including failed
                # scans. If its reply or status is lost, retain voice ownership
                # through the reserved recovery window before releasing it.
                while time.monotonic() < recovery_hold_until:
                    time.sleep(max(0, min(.05, recovery_hold_until - time.monotonic())))
            finally:
                if interval:
                    try:
                        checked(f'voice {session} vision_end', total_deadline)
                    except Exception as error:
                        cleanup_error = cleanup_error or error
                        LOG.warning('Closing aiming voice interval failed: %s', error)
            if primary_error is None and cleanup_error is not None:
                raise cleanup_error

    def status(self) -> dict[str, Any]:
        runtime = self.client.request("status")
        scenes = self._checked("scene status")
        speech = self._checked("speech status")
        character = self._checked("character status")
        return {
            "api_version": API_VERSION,
            "runtime": runtime,
            "mind": runtime.get("mind"),
            "scene": {
                "active": scenes.get("scene"),
                "last": scenes.get("last_scene"),
            },
            "speech": {
                "active": speech.get("speech"),
                "last": speech.get("last_speech"),
            },
            "character": character.get("character"),
            "rest": character.get("rest"),
            "routines": character.get("routines"),
        }

    def runtime_logs(self) -> dict[str, Any]:
        journal = shutil.which("journalctl")
        if not journal:
            raise GatewayError(HTTPStatus.NOT_IMPLEMENTED, "logs_unavailable", "Service logs are unavailable on this gateway.")
        try:
            result = subprocess.run([journal, "--no-pager", "--output=short-iso", "--lines=200",
                "-u", "oriond.service", "-u", "orion-studio-gateway.service", "-u", "orion-listener.service"],
                capture_output=True, text=True, timeout=3, check=False)
        except (OSError, subprocess.TimeoutExpired) as error:
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "logs_unavailable", "Could not read Orion service logs.") from error
        if result.returncode != 0:
            raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "logs_unavailable", "The gateway cannot read service logs. Check its journal permissions.")
        return {"api_version": API_VERSION, "lines": result.stdout[-100000:].splitlines()[-200:]}

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
                "scene_library": {"read": True, "create": True, "update": "revision", "delete": True},
                "joint_limits": limits.get("joints", []),
                "pose_library": {"read": True, "create": True, "update": False},
                "motion_library": {"read": True, "create": True, "update": False},
                "movement_lifecycle": ["prepare", "release"],
                "speech": {"format": "pcm16_mono_24000_hz", "max_bytes": MAX_SPEECH_BYTES, "max_seconds": 120, "streaming": "ordered_wav_chunks_v1", "max_chunk_seconds": 2},
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

    @staticmethod
    def _validate_routines(request):
        fields = {
            "list": set(), "stop": set(), "set_mode": {"mode"},
            "set_sound": {"kind", "sound"}, "timer": {"seconds", "label"},
            "alarm": {"due_unix", "label", "repeat", "briefing", "sunrise_minutes"},
            "reminder": {"due_unix", "seconds", "label", "repeat"},
            "start_light_routine": {"routine", "minutes"}, "stop_light_routine": set(),
            "cancel": {"id", "series"}, "announced": {"id"}, "announce_failed": {"id"},
            "dismiss_announcement": {"id"}, "snooze": {"minutes"},
        }
        action = request.get("action")
        if not isinstance(action, str) or action not in fields or set(request) - {"action"} - fields[action]:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_routines", "Unknown action or routines fields.")
        if action == "cancel" and "series" in request and type(request["series"]) is not bool:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_routines", "Series must be true or false.")
        if action == "alarm" and "briefing" in request and type(request["briefing"]) is not bool:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_routines", "Briefing must be true or false.")
        if action in {"alarm", "reminder"}:
            triggers = ("due_unix", "repeat") if action == "alarm" else ("due_unix", "seconds", "repeat")
            if sum(request.get(key) is not None for key in triggers) != 1:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_routines", "Choose exactly one alert trigger.")
            label = request.get("label")
            limit = 200 if action == "reminder" else 80
            if not isinstance(label, str) or len(label) > limit or (action == "reminder" and not label.strip()) or any(ord(c) < 32 for c in label):
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_routines", "Use printable alert text within the length limit.")
            repeat = request.get("repeat")
            if repeat is not None:
                import re
                if (not isinstance(repeat, dict) or set(repeat) != {"days", "time"}
                        or not isinstance(repeat["days"], list) or not 1 <= len(repeat["days"]) <= 7
                        or any(not isinstance(day, str) or day not in {"mon", "tue", "wed", "thu", "fri", "sat", "sun"} for day in repeat["days"])
                        or len(set(repeat["days"])) != len(repeat["days"])
                        or not isinstance(repeat["time"], str) or re.fullmatch(r"(?:[01]\d|2[0-3]):[0-5]\d", repeat["time"]) is None):
                    raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_routines", "Use unique repeat days and HH:MM time.")
        if action == "alarm" and request.get("sunrise_minutes") is not None:
            if type(request["sunrise_minutes"]) is not int or not 5 <= request["sunrise_minutes"] <= 60:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_routines", "Sunrise must be 5–60 minutes.")
        if action == "start_light_routine":
            bounds = {"wind_down": (10, 60), "focus": (5, 120)}.get(request.get("routine")) if isinstance(request.get("routine"), str) else None
            if bounds is None or type(request.get("minutes")) is not int or not bounds[0] <= request["minutes"] <= bounds[1]:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_routines", "Invalid light routine duration.")
        if action == "snooze" and request.get("minutes") is not None:
            if type(request["minutes"]) is not int or not 1 <= request["minutes"] <= 60:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_routines", "Snooze must be 1–60 minutes.")

    def submit(self, payload: Any) -> tuple[HTTPStatus, dict[str, Any]]:
        if not isinstance(payload, dict):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_request", "Expected a JSON object.")
        operation = payload.get("operation")
        if operation == 'aim_light':
            response = self.aim_light(payload)
        elif operation == "goto":
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
            if document.get("studio"):
                for path, data in self._owned_scene_assets(document).items():
                    if not path.is_file() or path.read_bytes() != data:
                        raise GatewayError(HTTPStatus.CONFLICT, "publish_scene_first", "Publish this scene to Orion before playing its custom poses on the robot.")
                document = {key: value for key, value in document.items() if key != "studio"}
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
        elif operation == "home":
            if set(payload) != {'operation', 'request'} or not isinstance(payload['request'], dict):
                raise GatewayError(HTTPStatus.BAD_REQUEST, 'invalid_home', 'Expected a home request.')
            try:
                response = self._home_public(self.home.voice(payload['request']))
            except HomeError as error:
                raise GatewayError(HTTPStatus(error.status), 'home_refused', str(error)) from None
            except (OSError, ValueError, KeyError, TypeError):
                raise GatewayError(HTTPStatus.BAD_GATEWAY, 'home_unavailable', 'Home Assistant could not complete this request.') from None
        elif operation == "routines":
            if set(payload) != {"operation", "request"} or not isinstance(payload["request"], dict):
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_routines", "Expected a routines request object.")
            self._validate_routines(payload["request"])
            try:
                body = json.dumps(payload["request"], ensure_ascii=True, separators=(",", ":"), allow_nan=False)
            except (ValueError, TypeError) as error:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_routines", "Use finite numbers in the routines request.") from error
            if len(body) > 1024:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_routines", "Routines request is too large.")
            response = self._checked("routines " + body)
        elif operation == "sleep":
            if set(payload) != {"operation", "session_id"}:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_sleep", "Sleep needs the current voice session.")
            session = self._name(payload.get("session_id"), "voice session")
            response = self._checked(f"sleep {session}")
        elif operation == "rest":
            response = self._checked("character rest")
        elif operation == "lamp_effect":
            settings = payload.get("settings")
            if not isinstance(settings, dict) or not settings or set(settings) - {"brightness", "effect", "colors", "transition_seconds"}:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_lamp", "Invalid lamp settings.")
            transition = settings.get("transition_seconds")
            if transition is not None and (type(transition) not in {int, float} or not 0 <= transition <= 3600):
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_lamp", "Transition must be 0–3600 seconds.")
            brightness = settings.get("brightness")
            if brightness is not None and (type(brightness) not in {int, float} or not 0 <= brightness <= 1):
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_lamp", "Brightness must be 0–1.")
            effect = settings.get("effect")
            if effect is not None and (not isinstance(effect, str) or effect not in {"solid", "warm_idle_breathe", "attentive_focus", "thinking_drift", "speaking_energy", "acknowledge_pulse", "curious_sweep", "delight_spark", "settle_glow", "off"}):
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_lamp", "Unknown lamp effect.")
            colors = settings.get("colors")
            if colors is not None and (not isinstance(colors, list) or not 1 <= len(colors) <= 2 or any(not isinstance(c, list) or len(c) != 4 or any(type(v) is not int or not 0 <= v <= 255 for v in c) for c in colors)):
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_lamp", "Expected one or two RGBW colors.")
            response = self._checked("lamp-effect " + json.dumps(settings, allow_nan=False))
        elif operation == "lamp_status":
            if set(payload) != {"operation"}:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_lamp", "Lamp status takes no parameters.")
            response = self._checked("lamp-status")
        elif operation == "lamp":
            channels = payload.get("rgbw")
            if (not isinstance(channels, list) or len(channels) != 4
                    or any(type(value) is not int or not 0 <= value <= 255 for value in channels)):
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_lamp", "rgbw must contain four integers from 0 to 255.")
            response = self._checked("lamp " + " ".join(map(str, channels)))
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
                "Supported operations are goto, motion, scene, preview_scene, speech, rest, sleep, routines, lamp, lamp_effect, lamp_status, character_start, character_stop, character_state, prepare_movement, release_movement, and cancel.",
            )

        return HTTPStatus.ACCEPTED, {
            "api_version": API_VERSION,
            "accepted": True,
            "operation": operation,
            "result": response,
        }

    def speak(self, text: str) -> None:
        root = installed_release()
        # Keep the venv path intact: its executable can be a symlink to a base interpreter.
        python = root / "speech/.venv/bin/python"
        if not python.is_file():
            raise RuntimeError(f"Missing speech Python environment: {python}")
        environment = os.environ.copy()
        keys = {"ORION_PIPER_MODEL_DIR", "ORION_TTS_THREADS"}
        try:
            lines = Path("~/.config/orion/voice-stack.env").expanduser().read_text(encoding="utf-8").splitlines()
        except FileNotFoundError:
            lines = []
        for line in lines:
            key, separator, value = line.strip().partition("=")
            if separator and key.strip() in keys:
                environment[key.strip()] = value.strip().strip("\"'")
        model = "piper-alba-medium"
        if "ORION_PIPER_MODEL_DIR" in environment:
            environment["ORION_PIPER_MODEL_DIR"] = str(Path(environment["ORION_PIPER_MODEL_DIR"]).expanduser())
        script = (
            "import importlib.util, pathlib, sys\n"
            "expected = pathlib.Path(sys.argv[3]).resolve()\n"
            "assert pathlib.Path(sys.prefix).resolve() == expected, 'Python is not using the release environment'\n"
            "spec = importlib.util.find_spec('orion_speech_worker.piper')\n"
            "assert spec and spec.origin and pathlib.Path(spec.origin).resolve().is_relative_to(expected), 'Piper is not installed in the release environment'\n"
            "from orion_speech_worker.piper import PiperAlbaSynthesizer\n"
            "for chunk in PiperAlbaSynthesizer(sys.argv[1]).stream(sys.argv[2]):\n"
            "    sys.stdout.buffer.write(chunk.pcm)\n"
        )
        try:
            result = subprocess.run([str(python), "-I", "-c", script, model, text, str(root / "speech/.venv")],
                                    cwd=root, env=environment, capture_output=True, timeout=30, check=True)
        except subprocess.CalledProcessError as error:
            detail = error.stderr.decode("utf-8", errors="replace").strip() if error.stderr else str(error)
            raise RuntimeError(f"Piper synthesis failed: {detail}") from error
        output = BytesIO()
        with wave.open(output, "wb") as wav:
            wav.setnchannels(1)
            wav.setsampwidth(2)
            wav.setframerate(24_000)
            wav.writeframes(result.stdout)
        self.upload_speech(output.getvalue(), "pairing-code")

    def upload_speech(self, body: bytes, studio_request_id: str, *, streaming=False, run_id=None, sequence=None, cues=None) -> tuple[HTTPStatus, dict[str, Any]]:
        if not studio_request_id or len(studio_request_id) > 128 or any(char in studio_request_id for char in "\r\n\0"):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_voice_request_id", "Studio voice request ID is required.")
        if run_id is not None and (type(run_id) is not int or run_id < 1 or type(sequence) is not int or sequence < 1 or not streaming):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_sequence", "A stream run and positive sequence are required.")
        cues = validate_speech_cues(cues)
        if cues and not streaming:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_speech_cues", "Reaction cues require streamed speech.")
        self._validate_speech_wav(body)
        if streaming and len(body) > 100_000:
            raise GatewayError(HTTPStatus.REQUEST_ENTITY_TOO_LARGE, "chunk_too_large", "Speech chunks may contain at most two seconds of audio.")
        voice_session = studio_request_id.removeprefix("voice:") if studio_request_id.startswith("voice:") else None
        if voice_session is not None and not re.fullmatch(r"[a-f0-9]{32}", voice_session):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_voice_session", "Invalid voice session ID.")
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
        cue_suffix = f" cues={cues}" if cues else ""
        try:
            if run_id is not None:
                runtime = self._checked(f"speech append {run_id} {sequence} {identifier}{cue_suffix}")
            else:
                suffix = f" {voice_session}" if voice_session is not None else ""
                runtime = self._checked(f"speech {'stream' if streaming else 'file'} {identifier}{suffix}{cue_suffix}")
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
                return {"api_version": API_VERSION, "run_id": run_id, "state": state, "error": speech.get("error"), "first_playback_ms": speech.get("first_playback_ms"), "elapsed_ms": speech.get("elapsed_ms"), "buffered_ms": speech.get("buffered_ms")}
        raise GatewayError(HTTPStatus.NOT_FOUND, "speech_run_not_found", f"Speech run {run_id} is not active or the most recent result.")

    def compile_trajectory_preview(self, payload: Any) -> dict[str, Any]:
        """Return the Rust compiler's calibrated 50 Hz sample document."""

        if self.project_root is None:
            raise GatewayError(HTTPStatus.NOT_IMPLEMENTED, "trajectory_preview_unavailable", "The gateway has no Orion project root.")
        if not isinstance(payload, dict) or not set(payload) <= {"motion", "document", "start_pose", "anchor_pose", "poses"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_trajectory_preview", "Trajectory preview accepts one motion name or v2 document, start_pose, and optional anchor_pose.")
        if ("motion" in payload) == ("document" in payload):
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_trajectory_preview", "Provide exactly one motion name or v2 motion document.")
        document = payload.get("document")
        motion = self._validate_motion_document(document) if document is not None else self._name(payload.get("motion"), "motion")
        start_pose = self._name(payload.get("start_pose"), "pose")
        anchor_value = payload.get("anchor_pose")
        anchor_pose = self._name(anchor_value, "pose") if anchor_value is not None else None
        poses_document = payload.get("poses")
        if poses_document is not None:
            if not isinstance(poses_document, dict) or not isinstance(poses_document.get("poses"), dict) or not poses_document["poses"]:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_preview_poses", "Preview poses are invalid.")
            for name, pose in poses_document["poses"].items():
                self._validate_pose_document({"format_version": 2, "units": "radians", "poses": {name: pose}})
        temporary: tempfile.TemporaryDirectory[str] | None = None
        try:
            motions_directory = self.project_root / ("motion/motions/v2" if self.hardware == "v2" else "motion/motions/v1")
            if document is not None or poses_document is not None:
                temporary = tempfile.TemporaryDirectory(prefix="orion-studio-motion-")
            if document is not None:
                motions_directory = Path(temporary.name)
                (motions_directory / f"{motion}.yaml").write_text(
                    json.dumps(document, separators=(",", ":"), ensure_ascii=False, allow_nan=False),
                    encoding="utf-8",
                )
            pose_file = self.project_root / ("motion/config/v2/poses.yaml" if self.hardware == "v2" else "motion/config/v1/poses.yaml")
            if poses_document is not None:
                pose_file = Path(temporary.name) / "preview-poses.json"
                pose_file.write_text(json.dumps(poses_document, allow_nan=False), encoding="utf-8")
            command = [
                str(self._trajectory_compiler_path()),
                "--hardware", self.hardware,
                "--motion", motion,
                "--start-pose", start_pose,
                "--pose-file", str(pose_file),
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
        # The catalog contains assets; the compiler belongs to this app release.
        candidates = [
            self.trajectory_compiler,
            GATEWAY_PROGRAM.parent.parent / "runtime/target/release/orion-trajectory",
            GATEWAY_PROGRAM.parent.parent / "runtime/target/debug/orion-trajectory",
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

    def _owned_scene_assets(self, document: Any) -> dict[Path, bytes]:
        self._validate_scene_document(document)
        if self.project_root is None:
            raise GatewayError(HTTPStatus.NOT_IMPLEMENTED, "scene_publish_unavailable", "Orion project root is unavailable.")
        studio = document.get("studio", {})
        if not isinstance(studio, dict) or not set(studio) <= {"poses", "motions"}:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene_assets", "Scene assets are invalid.")
        owner = document["scene"]["name"]
        result = {}
        for kind, directory in (("poses", self._user_pose_directory() / "_scene_owned"), ("motions", self._user_motion_directory() / "_scene_owned")):
            entries = studio.get(kind, {})
            if not isinstance(entries, dict):
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene_assets", "Scene assets must be named objects.")
            for name, value in entries.items():
                self._name(name, kind)
                suffix = "pose" if kind == "poses" else "movement"
                if not re.fullmatch(re.escape(owner) + "_custom_" + suffix + r"_[1-9][0-9]*", name) or not isinstance(value, dict) or value.get("owner_scene") != owner:
                    raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene_owner", "Custom assets must belong to this scene.")
                if kind == "poses":
                    asset = {"format_version": 2, "units": "radians", "poses": {name: {key: value[key] for key in ("description", "tags", "idle_profile", "default_lighting", "positions") if key in value}}}
                    self._validate_pose_document(asset)
                    self._validate_pose_limits(asset, self._checked("joint limits").get("joints"))
                else:
                    asset = {"format_version": 2, "motion": {key: value[key] for key in ("name", "description", "space", "style", "return_to_anchor", "keyframes") if key in value}}
                    self._validate_motion_document(asset)
                result[directory / f"{name}.yaml"] = self._encode_scene_document(asset)
        return result

    @contextmanager
    def _scene_asset_transaction(self, document: Any):
        assets = self._owned_scene_assets(document)
        backups = {}
        try:
            for path, data in assets.items():
                path.parent.mkdir(parents=True, exist_ok=True)
                if path.is_symlink():
                    raise GatewayError(HTTPStatus.CONFLICT, "invalid_scene_asset", "Scene asset cannot be a symbolic link.")
                backups[path] = path.read_bytes() if path.exists() else None
                self._atomic_replace(path, data)
            yield
        except Exception:
            for path, data in backups.items():
                if data is None: path.unlink(missing_ok=True)
                else: self._atomic_replace(path, data)
            raise

    def publish_scene(self, document: Any) -> tuple[HTTPStatus, dict[str, Any]]:
        with self.scene_write_lock:
            with self._scene_asset_transaction(document):
                result = self._publish_scene_locked(document)
                return result

    def _publish_scene_locked(self, document: Any) -> tuple[HTTPStatus, dict[str, Any]]:
        if self.project_root is None:
            raise GatewayError(
                HTTPStatus.NOT_IMPLEMENTED,
                "scene_publish_unavailable",
                "The gateway was not configured with an Orion project root.",
            )
        name = self._validate_scene_document(document)
        scenes_directory = self.project_root / f"scenes/{self.hardware}"
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
            reload_result = self._checked("asset reload" if document.get("studio") else "scene reload")
            return HTTPStatus.OK, {
                "api_version": API_VERSION,
                "published": True,
                "already_present": True,
                "name": name,
                "revision": self._revision(encoded),
                "relative_path": f"scenes/{self.hardware}/user/{name}.yaml",
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
            reload_result = self._checked("asset reload" if document.get("studio") else "scene reload")
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
            "relative_path": f"scenes/{self.hardware}/user/{name}.yaml",
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
                "relative_path": str(path.relative_to(self.project_root)),
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
            "relative_path": str(path.relative_to(self.project_root)),
            "yaml": yaml,
        }

    def delete_user_scene(self, name: Any, payload: Any) -> tuple[HTTPStatus, dict[str, Any]]:
        with self.scene_write_lock:
            name = self._name(name, "scene")
            if not isinstance(payload, dict) or set(payload) != {"expected_revision"}:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene_delete", "A scene revision is required.")
            path = self._existing_user_scene_path(name)
            previous = self._read_user_scene_file(path)
            if self._revision(previous) != payload["expected_revision"]:
                raise GatewayError(HTTPStatus.CONFLICT, "scene_revision_conflict", "Scene changed on Orion; reload it before deleting.")
            try:
                stored = json.loads(previous)
            except (ValueError, UnicodeDecodeError):
                stored = None
            owned = self._owned_scene_assets(stored) if stored and stored.get("studio") else {}
            backups = {asset: asset.read_bytes() for asset in owned if asset.exists() and not asset.is_symlink()}
            try:
                path.unlink()
                for asset in backups: asset.unlink()
                self._checked("asset reload" if owned else "scene reload")
            except (GatewayError, OSError):
                self._atomic_replace(path, previous)
                for asset, data in backups.items(): self._atomic_replace(asset, data)
                raise
            return HTTPStatus.OK, {"api_version": API_VERSION, "name": name, "deleted": True}

    def update_user_scene(self, name: Any, payload: Any) -> tuple[HTTPStatus, dict[str, Any]]:
        with self.scene_write_lock:
            with self._scene_asset_transaction(payload.get("document") if isinstance(payload, dict) else None):
                result = self._update_user_scene_locked(name, payload)
                return result

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
                "relative_path": str(path.relative_to(self.project_root)),
            }

        try:
            self._atomic_replace(path, encoded)
            reload_result = self._checked("asset reload" if document.get("studio") else "scene reload")
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
            "relative_path": str(path.relative_to(self.project_root)),
            "reload": reload_result,
        }

    def list_user_poses(self) -> dict[str, Any]:
        return self._list_user_assets(self._user_pose_directory(), ("motion/user/poses/v2" if self.hardware == "v2" else "motion/user/poses/v1"))

    def read_user_pose(self, name: Any) -> dict[str, Any]:
        return self._read_user_asset(
            self._user_pose_directory(),
            ("motion/user/poses/v2" if self.hardware == "v2" else "motion/user/poses/v1"),
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
            relative_directory=("motion/user/poses/v2" if self.hardware == "v2" else "motion/user/poses/v1"),
            existing_names=self._checked("pose list").get("poses", []),
        )

    def list_user_motions(self) -> dict[str, Any]:
        return self._list_user_assets(self._user_motion_directory(), ("motion/motions/v2/user" if self.hardware == "v2" else "motion/motions/v1/user"))

    def read_user_motion(self, name: Any) -> dict[str, Any]:
        return self._read_user_asset(
            self._user_motion_directory(),
            ("motion/motions/v2/user" if self.hardware == "v2" else "motion/motions/v1/user"),
            self._name(name, "motion"),
        )

    def publish_motion(self, document: Any) -> tuple[HTTPStatus, dict[str, Any]]:
        name = self._validate_motion_document(document)
        return self._publish_immutable_asset(
            kind="motion",
            name=name,
            document=document,
            directory=self._user_motion_directory(),
            relative_directory=("motion/motions/v2/user" if self.hardware == "v2" else "motion/motions/v1/user"),
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
        return self.project_root / f"scenes/{self.hardware}/user"

    def _user_pose_directory(self) -> Path:
        if self.project_root is None:
            raise GatewayError(
                HTTPStatus.NOT_IMPLEMENTED,
                "pose_library_unavailable",
                "The gateway was not configured with an Orion project root.",
            )
        return self.project_root / ("motion/user/poses/v2" if self.hardware == "v2" else "motion/user/poses/v1")

    def _user_motion_directory(self) -> Path:
        if self.project_root is None:
            raise GatewayError(
                HTTPStatus.NOT_IMPLEMENTED,
                "motion_library_unavailable",
                "The gateway was not configured with an Orion project root.",
            )
        return self.project_root / ("motion/motions/v2/user" if self.hardware == "v2" else "motion/motions/v1/user")

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
        if not isinstance(document, dict) or not {"format_version", "scene"} <= set(document) or not set(document) <= {"format_version", "scene", "studio"}:
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
        effects = {"warm_idle_breathe", "attentive_focus", "thinking_drift", "speaking_energy", "acknowledge_pulse", "curious_sweep", "delight_spark", "settle_glow", "off", "constant", "pulse", "breathe", "fade"}
        for event in lighting:
            if not isinstance(event, dict) or not set(event) <= {"at", "on_marker", "effect", "intensity", "duration", "transition", "palette", "colors", "period", "levels"}:
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Lighting track event contains invalid fields.")
            if ("at" in event) == ("on_marker" in event): raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Lighting event requires exactly one trigger.")
            if "at" in event: cls._number(event["at"], "Lighting time", minimum=0.0)
            else: cls._name(event["on_marker"], "marker")
            if event.get("effect") not in effects: raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Unknown Orion lighting effect.")
            cls._number(event.get("intensity", 1.0), "Lighting intensity", minimum=0.0)
            if event.get("effect") in {"constant", "pulse", "breathe", "fade"}:
                colors = event.get("colors")
                count = 1 if event["effect"] == "constant" else 2
                if not isinstance(colors, list) or len(colors) != count or any(not isinstance(color, str) or (color != "warm_white" and not re.fullmatch(r"#[0-9a-fA-F]{6}", color)) for color in colors):
                    raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Choose a color for every effect stage.")
            cls._number(event.get("period", 2.0), "Effect cycle", minimum=0.1)
            levels = event.get("levels", [])
            if not isinstance(levels, list) or len(levels) > 2 or any(type(level) not in (int, float) or not math.isfinite(level) or not 0 <= level <= 1 for level in levels):
                raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_scene", "Stage brightness must be between zero and one.")
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

    def _checked(self, command: str, *, deadline=None) -> dict[str, Any]:
        response = self.client.request(command, **({'deadline': deadline} if deadline is not None else {}))
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


def voice_service_directory() -> Path:
    return Path("~/.local/share/orion/studio-service").expanduser()


def voice_service_request(request: dict[str, Any]) -> Any:
    """Bridge allowlisted controls to the private headless owner, never to Codex directly."""
    if not isinstance(request, dict) or request.get("method") not in {
        "status", "observe", "history", "behaviours", "start_saved", "load_settings", "save_settings", "model_locations", "profile", "lists", "geocode", "microphone",
    }:
        raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_voice_request", "Unsupported voice control.")
    try:
        connection = json.loads((voice_service_directory() / "connection.json").read_text())
        host, port = connection["address"].rsplit(":", 1)
        if connection["protocol"] != 1 or not ipaddress.ip_address(host).is_loopback:
            raise ValueError("Invalid voice service address")
        payload = json.dumps({"protocol": 1, "token": connection["token"], "request": request}).encode() + b"\n"
        with socket.create_connection((host, int(port)), timeout=2) as client:
            client.settimeout(135)
            client.sendall(payload)
            with client.makefile("rb") as reader:
                data = reader.readline(MAX_RESPONSE_BYTES + 1)
        if len(data) > MAX_RESPONSE_BYTES or not data.endswith(b"\n"):
            raise ValueError("Invalid voice service response")
        result = json.loads(data)
        if result.get("ok") is not True:
            raise GatewayError(HTTPStatus.BAD_REQUEST, "voice_request_failed", result.get("error", "Voice request failed."))
        value = result["result"]
        if request["method"] == "start_saved":
            # The remote client uses the gateway's token, not a loopback-only credential.
            value = {"url": "/api/v2/voice/events", "asrModel": value["asrModel"]}
        return value
    except GatewayError:
        raise
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "voice_unavailable", "Orion's onboard voice service is unavailable.") from error


def make_handler(gateway: OrionGateway, token: str, allowed_origins: str | list[str] | tuple[str, ...]):
    origin_allowlist = {allowed_origins} if isinstance(allowed_origins, str) else set(allowed_origins)
    pairing_codes = PairingCodes(token, lambda text: gateway.speak(text))

    class GatewayHandler(BaseHTTPRequestHandler):
        server_version = "OrionStudioGateway/2"

        def do_OPTIONS(self) -> None:
            self.send_response(HTTPStatus.NO_CONTENT)
            self._cors_headers()
            self.send_header("Access-Control-Allow-Headers", "Authorization, Content-Type, X-Orion-Voice-Request-ID, X-Orion-Speech-Cues")
            self.send_header("Access-Control-Allow-Methods", "GET, POST, PUT, DELETE, OPTIONS")
            self.end_headers()

        def do_GET(self) -> None:
            self._handle(self._get)

        def do_POST(self) -> None:
            self._handle(self._post)

        def do_DELETE(self) -> None:
            self._handle(self._delete)

        def _delete(self) -> tuple[HTTPStatus, dict[str, Any]]:
            path = urlparse(self.path).path
            if path.startswith('/api/v2/home/'):
                return HTTPStatus.OK, gateway.home_request('DELETE', path.removeprefix('/api/v2/home/'))
            if not path.startswith("/api/v2/scenes/"):
                raise GatewayError(HTTPStatus.NOT_FOUND, "not_found", "Unknown Orion Studio endpoint.")
            return gateway.delete_user_scene(path.removeprefix("/api/v2/scenes/"), self._read_json())

        def do_PUT(self) -> None:
            self._handle(self._put)

        def _get(self) -> tuple[HTTPStatus, dict[str, Any]] | None:
            path = urlparse(self.path).path
            if path.startswith('/api/v2/home/'):
                return HTTPStatus.OK, gateway.home_request('GET', path.removeprefix('/api/v2/home/'))
            if path == '/api/v2/system/metrics':
                return HTTPStatus.OK, gateway.system_metrics.snapshot()
            if path == '/api/v2/behaviours':
                return HTTPStatus.OK, gateway.get_behaviours()
            if path == '/api/v2/tracking/mode':
                return HTTPStatus.OK, gateway.get_tracking_mode()
            camera_paths = {"/api/v2/camera/stream": "/stream", "/api/v2/camera/snapshot": "/frame.jpg",
                            "/api/v2/camera/observations": "/observations",
                            "/api/v2/camera/detections": "/detections"}
            if path in camera_paths:
                return self._proxy_camera(camera_paths[path])
            if path == "/api/v2/voice/status":
                installed = (voice_service_directory() / "installed").exists()
                return HTTPStatus.OK, {"onboard": installed}
            if path == "/api/v2/voice/events":
                return HTTPStatus.OK, voice_service_request({"method": "observe"})
            if path == "/api/v2/debug/logs":
                return HTTPStatus.OK, gateway.runtime_logs()
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
            if path.startswith('/api/v2/home/'):
                return HTTPStatus.OK, gateway.home_request('POST', path.removeprefix('/api/v2/home/'), self._read_json())
            if path == '/api/v2/behaviours':
                return HTTPStatus.OK, gateway.set_behaviours(self._read_json())
            if path == '/api/v2/tracking/mode':
                return HTTPStatus.OK, gateway.set_tracking_mode(self._read_json())
            if path == "/api/v2/camera/enabled":
                return HTTPStatus.OK, gateway.set_camera_enabled(self._read_json())
            if path == "/api/v2/pair/code":
                return HTTPStatus.OK, pairing_codes.request_code()
            if path == "/api/v2/pair/token":
                payload = self._read_json()
                return HTTPStatus.OK, pairing_codes.exchange(payload.get("code") if isinstance(payload, dict) else None)
            if path == "/api/v2/voice/request":
                return HTTPStatus.OK, voice_service_request(self._read_json())
            if path == "/api/v2/speech/stream":
                return gateway.upload_speech(self._read_speech_wav(), self.headers.get("X-Orion-Voice-Request-ID", ""), streaming=True, cues=self.headers.get("X-Orion-Speech-Cues"))
            match = re.fullmatch(r"/api/v2/speech/([1-9][0-9]*)/chunks/([1-9][0-9]*)", path)
            if match:
                return gateway.upload_speech(self._read_speech_wav(), self.headers.get("X-Orion-Voice-Request-ID", ""), streaming=True, run_id=int(match[1]), sequence=int(match[2]), cues=self.headers.get("X-Orion-Speech-Cues"))
            match = re.fullmatch(r"/api/v2/speech/([1-9][0-9]*)/end", path)
            if match:
                sequence = self._read_json().get("sequence")
                if type(sequence) is not int or sequence < 1:
                    raise GatewayError(HTTPStatus.BAD_REQUEST, "invalid_sequence", "A positive sequence is required.")
                return HTTPStatus.OK, gateway._checked(f"speech end {match[1]} {sequence}")
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
                pairing_request = self.command == "POST" and urlparse(self.path).path in {"/api/v2/pair/code", "/api/v2/pair/token"}
                if not pairing_request and not secrets.compare_digest(supplied, token):
                    raise GatewayError(HTTPStatus.UNAUTHORIZED, "unauthorized", "A valid Studio token is required.")
                result = action()
                if result is None:  # Binary/stream responses have already been written.
                    return
                status, body = result
            except GatewayError as error:
                status = error.status
                body = {
                    "api_version": API_VERSION,
                    "error": {"code": error.code, "message": str(error)},
                }
            self._write_json(status, body)

        def _proxy_camera(self, path):
            connection, response = gateway.open_camera(path)
            try:
                content_type = response.getheader("Content-Type", "application/json")
                if path != "/stream" or response.status != 200:
                    snapshot = path == "/frame.jpg" and response.status == 200
                    lease = gateway.camera_viewers.watching(snapshot=True) if snapshot else nullcontext()
                    with lease as viewers:
                        body = response.read(MAX_RESPONSE_BYTES + 1)
                        if len(body) > MAX_RESPONSE_BYTES:
                            raise GatewayError(HTTPStatus.BAD_GATEWAY, "camera_response_too_large", "Camera response exceeds 1 MiB.")
                        if snapshot:
                            gateway.require_camera_awake()
                            if not gateway.camera_is_enabled():
                                raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_off", "Camera is disabled.")
                            if not viewers.healthy:
                                raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_indicator_unavailable", "Orion's camera viewer indicator is unavailable.")
                        if path == "/detections":
                            gateway.require_camera_awake()
                            gateway.camera_viewers.require_viewing()
                            if not gateway.camera_is_enabled():
                                raise GatewayError(HTTPStatus.SERVICE_UNAVAILABLE, "camera_off", "Camera is disabled.")
                        self.send_response(response.status)
                        self._cors_headers()
                        self.send_header("Content-Type", content_type)
                        self.send_header("Content-Length", str(len(body)))
                        self.send_header("Cache-Control", "no-store")
                        stamp = response.getheader("X-Capture-Boottime-Ns")
                        if stamp:
                            self.send_header("X-Capture-Boottime-Ns", stamp)
                        self.end_headers()
                        self.wfile.write(body)
                    return
                # WebKit cancels x-mixed-replace fetch loads that are not image
                # elements. Generic multipart keeps the same JPEG framing and
                # lets authenticated fetch clients read the bytes themselves.
                content_type = re.sub(r"^multipart/x-mixed-replace(?=\s*;)", "multipart/mixed", content_type, flags=re.IGNORECASE)
                with gateway.camera_viewers.watching() as viewers:
                    self.send_response(HTTPStatus.OK)
                    self._cors_headers()
                    self.send_header("Content-Type", content_type)
                    self.send_header("Cache-Control", "no-store")
                    self.end_headers()
                    self.connection.settimeout(2)
                    while viewers.healthy and gateway.camera_is_enabled():
                        chunk = response.read1(65536)
                        if not chunk or not viewers.healthy or not gateway.camera_is_enabled():
                            break
                        self.wfile.write(chunk)
                        self.wfile.flush()
            except (OSError, http.client.HTTPException):
                self.close_connection = True
            finally:
                connection.close()

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
    print(f"Studio token file: {path}")


class GatewayHTTPServer(ThreadingHTTPServer):
    """IPv4 for IPv4 binds; IPv6 with IPv4 support for the wildcard ``::``."""

    def __init__(self, server_address, handler):
        self.address_family = socket.AF_INET6 if ":" in server_address[0] else socket.AF_INET
        super().__init__(server_address, handler)

    def server_bind(self) -> None:
        if self.address_family == socket.AF_INET6:
            # Set this explicitly: the OS default differs between platforms.
            # A specific IPv6 address remains IPv6-only; only :: accepts IPv4 too.
            dual_stack = self.server_address[0] == "::"
            self.socket.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, int(not dual_stack))
        super().server_bind()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    token = commands.add_parser("create-token", help="Create a development bearer token.")
    token.add_argument("--token-file", type=Path, required=True)
    serve = commands.add_parser("serve", help="Run the Studio gateway.")
    serve.add_argument("--hardware", choices=("v1", "v2"), default="v1")
    serve.add_argument("--socket", default=DEFAULT_SOCKET)
    serve.add_argument("--vision-socket", default="/tmp/orion-vision.sock")
    serve.add_argument("--camera-preference-file", type=Path, default=Path.home() / ".config/orion/camera.json")
    serve.add_argument('--tracking-preference-file', type=Path,
        help='Tracking preference (default: tracking.json beside camera preference)')
    serve.add_argument("--bind", default="127.0.0.1", help="Listen address; :: accepts IPv4 and IPv6 (default: IPv4 loopback).")
    serve.add_argument("--port", type=int, default=7447)
    serve.add_argument("--token-file", type=Path, required=True)
    serve.add_argument("--calibration", type=Path)
    serve.add_argument("--trajectory-compiler", type=Path)
    serve.add_argument(
        "--project-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="Asset catalog containing scenes/<hardware> and motion/config/<hardware>/poses.yaml (default: inferred from gateway.py).",
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
    if not (project_root / f"motion/config/{args.hardware}/poses.yaml").is_file() or not (project_root / f"scenes/{args.hardware}").is_dir():
        raise SystemExit(f"Not an Orion project root: '{project_root}'.")
    gateway = OrionGateway(
        UnixOrionClient(args.socket),
        project_root,
        calibration_file=args.calibration,
        trajectory_compiler=args.trajectory_compiler,
        hardware=args.hardware,
        vision_socket=args.vision_socket,
        camera_preference_file=args.camera_preference_file,
        tracking_preference_file=args.tracking_preference_file,
    )
    gateway.system_metrics.start()
    server = GatewayHTTPServer(
        (args.bind, args.port), make_handler(gateway, token, args.allowed_origins)
    )
    host = f"[{args.bind}]" if ":" in args.bind else args.bind
    print(f"orion-studio-gateway: serving http://{host}:{args.port} -> {args.socket}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
        gateway.system_metrics.close()


if __name__ == "__main__":
    main()
