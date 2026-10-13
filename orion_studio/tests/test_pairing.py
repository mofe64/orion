from __future__ import annotations

import io
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest
import venv
import wave
from concurrent.futures import ThreadPoolExecutor
from contextlib import chdir
from http import HTTPStatus
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from gateway import GatewayError, GatewayHTTPServer, OrionGateway, PairingCodes, make_handler  # noqa: E402


class PairingCodesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.clock = Mock(return_value=0.0)
        self.speak = Mock()
        self.pairing = PairingCodes("a" * 32, self.speak, self.clock)
        self.random = self.enterContext(patch("gateway.secrets.randbelow", return_value=482190))
        self.output = self.enterContext(patch("sys.stdout", new_callable=io.StringIO))

    def assert_error(self, code: str, status: HTTPStatus, operation, *args) -> GatewayError:
        with self.assertRaises(GatewayError) as context:
            operation(*args)
        self.assertEqual((context.exception.code, context.exception.status), (code, status))
        return context.exception

    def test_code_has_six_digits_including_leading_zeroes_and_speaks_words(self) -> None:
        self.random.return_value = 1
        self.assertEqual(self.pairing.request_code(), {"api_version": 2, "expires_in_seconds": 300})
        self.random.assert_called_once_with(10**6)
        self.assertEqual(self.output.getvalue(), "Studio pairing code: 000001\n")
        self.speak.assert_called_once_with(
            "Studio pairing code. zero, zero, zero, zero, zero, one. Again, zero, zero, zero, zero, zero, one.")
        self.assertEqual(self.pairing.exchange("000001"), {"api_version": 2, "token": "a" * 32})

    def test_no_code_and_expiry_at_five_minutes(self) -> None:
        error = self.assert_error("pairing_code_expired", HTTPStatus.GONE, self.pairing.exchange, "482190")
        self.assertEqual(str(error), "Ask the lamp for a new code.")
        self.pairing.request_code()
        self.clock.return_value = 299.999
        self.assert_error("pairing_code_wrong", HTTPStatus.FORBIDDEN, self.pairing.exchange, "111111")
        self.clock.return_value = 300
        self.assert_error("pairing_code_expired", HTTPStatus.GONE, self.pairing.exchange, "482190")

    def test_rate_limit_is_recorded_before_speaking_and_does_not_replace_code(self) -> None:
        def speak(_text):
            self.assert_error("pairing_busy", HTTPStatus.TOO_MANY_REQUESTS, self.pairing.request_code)
            self.assertTrue(self.pairing.lock.acquire(blocking=False))
            self.pairing.lock.release()
        self.speak.side_effect = speak
        self.assertEqual(self.pairing.request_code(), {"api_version": 2, "expires_in_seconds": 300})
        self.clock.return_value = 14.999
        self.assert_error("pairing_busy", HTTPStatus.TOO_MANY_REQUESTS, self.pairing.request_code)
        self.random.assert_called_once()
        self.speak.side_effect = None
        self.clock.return_value = 15
        self.pairing.request_code()
        self.assertEqual(self.speak.call_count, 2)

    def test_wrong_code_reports_remaining_tries_and_fifth_attempt_discards_it(self) -> None:
        self.pairing.request_code()
        for remaining in (4, 3, 2, 1, 0):
            error = self.assert_error("pairing_code_wrong", HTTPStatus.FORBIDDEN, self.pairing.exchange, "111111")
            self.assertIn(f"{remaining} {'try' if remaining == 1 else 'tries'} remain" if remaining else "code was discarded", str(error))
        self.assert_error("pairing_code_expired", HTTPStatus.GONE, self.pairing.exchange, "482190")

    def test_replacement_resets_attempts_and_invalidates_old_code(self) -> None:
        self.pairing.request_code()
        self.assert_error("pairing_code_wrong", HTTPStatus.FORBIDDEN, self.pairing.exchange, "111111")
        self.clock.return_value = 15
        self.random.return_value = 123456
        self.pairing.request_code()
        error = self.assert_error("pairing_code_wrong", HTTPStatus.FORBIDDEN, self.pairing.exchange, "482190")
        self.assertIn("4 tries remain", str(error))
        self.assertEqual(self.pairing.exchange("123456")["token"], "a" * 32)

    def test_correct_code_is_single_use_even_with_concurrent_exchanges(self) -> None:
        self.pairing.request_code()
        def exchange(_index):
            try:
                return self.pairing.exchange("482190")
            except GatewayError as error:
                return error.code
        with ThreadPoolExecutor(max_workers=2) as executor:
            results = list(executor.map(exchange, range(2)))
        self.assertCountEqual(results, [{"api_version": 2, "token": "a" * 32}, "pairing_code_expired"])

    def test_only_six_ascii_digits_are_accepted_without_spending_attempts(self) -> None:
        self.pairing.request_code()
        for value in (None, 482190, "12345", "1234567", "１２３４５６", "12345\n", " 482190", "abcdef"):
            with self.subTest(value=value):
                self.assert_error("invalid_pairing_code", HTTPStatus.BAD_REQUEST, self.pairing.exchange, value)
        self.assertEqual(self.pairing.attempts, 0)
        self.assertEqual(self.pairing.exchange("482190")["token"], "a" * 32)

    def test_speak_failure_keeps_code_valid_and_journal_fallback(self) -> None:
        self.speak.side_effect = RuntimeError("speaker unavailable")
        response = self.pairing.request_code()
        self.assertFalse(response["spoken"])
        self.assertIn("gateway log", response["message"])
        self.assertIn("Studio pairing code: 482190", self.output.getvalue())
        self.assertIn("Studio pairing code: 482190; speech failed: speaker unavailable", self.output.getvalue())
        self.assertEqual(self.pairing.exchange("482190")["token"], "a" * 32)


class PairingHttpTests(unittest.TestCase):
    def setUp(self) -> None:
        self.gateway = Mock(spec=OrionGateway)
        self.server = GatewayHTTPServer(("127.0.0.1", 0), make_handler(self.gateway, "a" * 32, ["tauri://localhost"]))
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def tearDown(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)

    def request(self, method: str, path: str, payload=None):
        connection = HTTPConnection("127.0.0.1", self.server.server_port, timeout=3)
        try:
            connection.request(method, path, body=json.dumps(payload) if payload is not None else None,
                               headers={"Origin": "tauri://localhost", "Content-Type": "application/json"})
            response = connection.getresponse()
            return response.status, json.load(response), response.getheader("Access-Control-Allow-Origin")
        finally:
            connection.close()

    def test_pairing_posts_need_no_bearer_but_other_paths_and_methods_do(self) -> None:
        with patch("gateway.secrets.randbelow", return_value=482190):
            status, body, origin = self.request("POST", "/api/v2/pair/code")
        self.assertEqual((status, body, origin), (HTTPStatus.OK, {"api_version": 2, "expires_in_seconds": 300}, "tauri://localhost"))
        self.gateway.speak.assert_called_once_with(
            "Studio pairing code. four, eight, two, one, nine, zero. Again, four, eight, two, one, nine, zero.")
        status, body, _ = self.request("POST", "/api/v2/pair/token", {"code": "482190"})
        self.assertEqual((status, body), (HTTPStatus.OK, {"api_version": 2, "token": "a" * 32}))
        for method, path in (("GET", "/api/v2/status"), ("POST", "/api/v2/operations"),
                             ("POST", "/api/v2/pair/code/"), ("POST", "/api/v2/pair/token/extra"),
                             ("GET", "/api/v2/pair/token")):
            with self.subTest(method=method, path=path):
                self.assertEqual(self.request(method, path)[0], HTTPStatus.UNAUTHORIZED)

    def test_nonobject_payload_returns_invalid_code(self) -> None:
        status, body, _ = self.request("POST", "/api/v2/pair/token", ["123456"])
        self.assertEqual((status, body["error"]["code"]), (HTTPStatus.BAD_REQUEST, "invalid_pairing_code"))


class PairingSpeechTests(unittest.TestCase):
    def installed_release(self, home: Path) -> tuple[Path, Path]:
        release = (home / "release").resolve()
        program = release / "orion_studio/gateway.py"
        program.parent.mkdir(parents=True)
        program.touch()
        (release / "release.json").write_text('{"revision":"fixture"}')
        python = release / "speech/.venv/bin/python"
        python.parent.mkdir(parents=True)
        base_python = home / "base/bin/python"
        base_python.parent.mkdir(parents=True)
        base_python.touch()
        python.symlink_to(base_python)
        self.enterContext(patch("gateway.GATEWAY_PROGRAM", program.resolve()))
        return release, python

    def test_subprocess_uses_voice_environment_and_uploads_pcm_as_runtime_wav(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            release, python = self.installed_release(home)
            environment_file = home / ".config/orion/voice-stack.env"
            environment_file.parent.mkdir(parents=True)
            environment_file.write_text(
                '# Pi voice settings\n'
                "ORION_PIPER_MODEL_DIR=~/models/piper\nORION_STUDIO_TTS_MODEL=unsupported-old-default\n"
                "ORION_TTS_THREADS=2\nUNRELATED=value\n", encoding="utf-8")
            gateway = OrionGateway(Mock(), home / "project")
            pcm = b"\x01\x00" * 480
            with patch.dict(os.environ, {"HOME": str(home)}, clear=True), \
                 patch("gateway.subprocess.run", return_value=Mock(stdout=pcm)) as run, \
                 patch.object(gateway, "upload_speech") as upload:
                gateway.speak("Studio pairing code.")
            args = run.call_args.args[0]
            self.assertEqual(args[0:3], [str(python), "-I", "-c"])
            self.assertIn("PiperAlbaSynthesizer", args[3])
            self.assertIn("sys.stdout.buffer.write(chunk.pcm)", args[3])
            self.assertEqual(args[4:], ["piper-alba-medium", "Studio pairing code.", str(release / "speech/.venv")])
            options = run.call_args.kwargs
            self.assertEqual((options["timeout"], options["check"]), (30, True))
            self.assertEqual(options["cwd"], release)
            self.assertNotIn("PYTHONPATH", options["env"])
            self.assertEqual(options["env"]["ORION_PIPER_MODEL_DIR"], str(home / "models/piper"))
            self.assertEqual(options["env"]["ORION_TTS_THREADS"], "2")
            self.assertNotIn("ORION_STUDIO_TTS_MODEL", options["env"])
            self.assertNotIn("UNRELATED", options["env"])
            wav_bytes, request_id = upload.call_args.args
            self.assertEqual(request_id, "pairing-code")
            with wave.open(io.BytesIO(wav_bytes), "rb") as wav:
                self.assertEqual((wav.getnchannels(), wav.getsampwidth(), wav.getframerate()), (1, 2, 24_000))
                self.assertEqual(wav.readframes(wav.getnframes()), pcm)

    def test_missing_release_does_not_use_the_asset_checkout_as_fallback(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            root = home / "project"
            (root / "speech/orion_speech_worker").mkdir(parents=True)
            gateway = OrionGateway(Mock(), root)
            program = home / "unpackaged/orion_studio/gateway.py"
            with patch.dict(os.environ, {"HOME": str(home)}, clear=True), \
                 patch("gateway.GATEWAY_PROGRAM", program), \
                 patch("gateway.subprocess.run") as run, \
                 patch.object(gateway, "upload_speech"):
                with self.assertRaisesRegex(RuntimeError, "missing release.json"):
                    gateway.speak("Pairing code.")
            run.assert_not_called()

    def test_piper_stderr_is_logged_beside_the_code_when_synthesis_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.installed_release(root)
            gateway = OrionGateway(Mock(), root)
            pairing = PairingCodes("a" * 32, gateway.speak)
            failure = subprocess.CalledProcessError(1, "piper", stderr=b"Piper Alba model is incomplete: /models/piper\n")
            with patch.dict(os.environ, {"HOME": str(root)}, clear=True), \
                 patch("gateway.secrets.randbelow", return_value=482190), \
                 patch("gateway.subprocess.run", side_effect=failure), \
                 patch("sys.stdout", new_callable=io.StringIO) as output, \
                 patch.object(gateway, "upload_speech") as upload:
                response = pairing.request_code()
            self.assertFalse(response["spoken"])
            self.assertIn("Studio pairing code: 482190; speech failed: Piper synthesis failed: Piper Alba model is incomplete: /models/piper", output.getvalue())
            self.assertEqual(pairing.exchange("482190")["token"], "a" * 32)
            upload.assert_not_called()

    def test_missing_env_uses_release_python_and_default_model_and_timeout_propagates(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _release, python = self.installed_release(root)
            gateway = OrionGateway(Mock(), root)
            with patch.dict(os.environ, {"HOME": str(root)}, clear=True), \
                 patch("gateway.subprocess.run", side_effect=subprocess.TimeoutExpired("piper", 30)) as run, \
                 patch.object(gateway, "upload_speech") as upload:
                with self.assertRaises(subprocess.TimeoutExpired):
                    gateway.speak("Pairing code.")
            args = run.call_args.args[0]
            self.assertEqual(args[0], str(python))
            self.assertEqual(args[4], "piper-alba-medium")
            upload.assert_not_called()


class InstalledPairingSpeechTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.home = self.base / "settings-home"
        self.home.mkdir()
        self.assets = self.base / "asset-checkout"
        poison = self.assets / "speech/orion_speech_worker"
        poison.mkdir(parents=True)
        (poison / "__init__.py").write_text("")
        (poison / "piper.py").write_text("raise RuntimeError('asset checkout must not supply speech')")
        self.enterContext(patch.dict(os.environ, {
            "HOME": str(self.home), "PYTHONPATH": str(self.assets / "speech"),
            "PYTHONHOME": str(self.base / "wrong-python-home"),
        }, clear=True))

    def install(self, relative: str, revision: str, sample: int) -> Path:
        root = self.base / relative
        program = root / "orion_studio/gateway.py"
        program.parent.mkdir(parents=True)
        shutil.copyfile(Path(__file__).resolve().parents[1] / "gateway.py", program)
        (root / "release.json").write_text(json.dumps({"revision": revision}))
        environment = root / "speech/.venv"
        venv.EnvBuilder(with_pip=False).create(environment)
        python = environment / "bin/python"
        packages = Path(subprocess.check_output(
            [str(python), "-I", "-c", "import sysconfig; print(sysconfig.get_path('purelib'))"],
            text=True).strip())
        package = packages / "orion_speech_worker"
        package.mkdir()
        (package / "__init__.py").write_text("")
        # Give each release distinguishable PCM without loading speech models.
        (package / "piper.py").write_text(
            "from types import SimpleNamespace\n"
            "class PiperAlbaSynthesizer:\n"
            "    def __init__(self, model):\n"
            "        assert model == 'piper-alba-medium'\n"
            "    def stream(self, text):\n"
            f"        yield SimpleNamespace(pcm={sample.to_bytes(2, 'little')!r} * 480)\n")
        return program

    def load_gateway(self, program: Path):
        spec = importlib.util.spec_from_file_location("installed_pairing_gateway", program)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module.OrionGateway(Mock(), self.assets)

    def assert_speech(self, gateway, sample: int) -> None:
        with chdir(self.assets), patch.object(gateway, "upload_speech") as upload:
            gateway.speak("Studio pairing code.")
        wav_bytes, request_id = upload.call_args.args
        self.assertEqual(request_id, "pairing-code")
        with wave.open(io.BytesIO(wav_bytes), "rb") as wav:
            self.assertEqual((wav.getnchannels(), wav.getsampwidth(), wav.getframerate()), (1, 2, 24_000))
            self.assertEqual(wav.readframes(wav.getnframes()), sample.to_bytes(2, "little") * 480)

    def test_pairing_speech_uses_installed_package_at_different_paths(self) -> None:
        for relative, sample in (("home/alice/.local/share/orion/voice-stack/releases/one", 1),
                                 ("opt/custom-orion/releases/two", 2),
                                 ("users/Bob Smith/custom install/three", 3)):
            with self.subTest(path=relative):
                self.assert_speech(self.load_gateway(self.install(relative, str(sample), sample)), sample)

    def test_update_keeps_running_gateway_with_its_release_and_new_gateway_uses_new_speech(self) -> None:
        first = self.install("releases/first", "first", 1)
        second = self.install("releases/second", "second", 2)
        launcher = self.base / "gateway.py"
        launcher.symlink_to(first)
        running = self.load_gateway(launcher)
        self.assert_speech(running, 1)
        launcher.unlink()
        launcher.symlink_to(second)
        self.assert_speech(running, 1)
        self.assert_speech(self.load_gateway(launcher), 2)

    def test_missing_package_and_environment_fail_without_asset_fallback(self) -> None:
        program = self.install("releases/incomplete", "incomplete", 1)
        gateway = self.load_gateway(program)
        environment = program.parent.parent / "speech/.venv"
        python = environment / "bin/python"
        packages = Path(subprocess.check_output(
            [str(python), "-I", "-c", "import sysconfig; print(sysconfig.get_path('purelib'))"],
            text=True).strip())
        shutil.rmtree(packages / "orion_speech_worker")
        with patch.object(gateway, "upload_speech") as upload:
            with self.assertRaisesRegex(RuntimeError, "Piper synthesis failed"):
                gateway.speak("Pairing code.")
            python.unlink()
            with self.assertRaisesRegex(RuntimeError, "Missing speech Python environment"):
                gateway.speak("Pairing code.")
        upload.assert_not_called()

    def test_invalid_release_metadata_is_reported_before_synthesis(self) -> None:
        program = self.install("releases/invalid", "invalid", 1)
        gateway = self.load_gateway(program)
        manifest = program.parent.parent / "release.json"
        for content, error in (("not json", "Invalid Orion release metadata"),
                               ('{"revision":""}', "Missing revision"),
                               ("[]", "Missing revision")):
            with self.subTest(content=content):
                manifest.write_text(content)
                with self.assertRaisesRegex(RuntimeError, error):
                    gateway.speak("Pairing code.")


if __name__ == "__main__":
    unittest.main()
