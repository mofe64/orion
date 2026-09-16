"""CPU speech adapters; credentials and robot control remain outside this worker."""
from __future__ import annotations

import base64
import io
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
import wave
import uuid

import numpy as np

from .providers import Transcript
from .tts import SpeechAudio

VOICES = ("alba", "anna", "azelma", "cosette", "eve", "fantine", "jane", "vera")


class QwenGgufTranscriber:
    provider = "qwen3-asr"

    def __init__(self, model):
        root = Path(model)
        weights, projector = root / "model.gguf", root / "mmproj.gguf"
        binary = Path(os.environ["ORION_LLAMA_SERVER"]).resolve(strict=True)
        if not weights.is_file() or not projector.is_file():
            raise ValueError("Qwen model folder requires model.gguf and mmproj.gguf")
        self.model_name = str(root)
        self.key = secrets.token_hex(32)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        self.url = f"http://127.0.0.1:{port}"
        threads = int(os.environ.get("ORION_ASR_THREADS", "3"))
        if not 1 <= threads <= 4:
            raise ValueError("ASR threads must be between one and four")
        self.process = subprocess.Popen([
            sys.executable, "-m", "orion_speech_worker.native", str(binary),
            "-m", str(weights), "--mmproj", str(projector), "--host", "127.0.0.1",
            "--port", str(port), "--api-key", self.key, "-t", str(threads), "-tb", str(threads),
            "-ngl", "0", "-c", "4096", "-np", "1", "--no-warmup", "--log-disable",
        ], stdout=sys.stderr, stderr=sys.stderr)
        deadline = time.monotonic() + 120
        try:
            while time.monotonic() < deadline:
                if self.process.poll() is not None:
                    raise RuntimeError("Qwen server exited during loading")
                try:
                    self.request("/health", timeout=1)
                    break
                except (urllib.error.URLError, TimeoutError):
                    time.sleep(.1)
            else:
                raise TimeoutError("Qwen loading timed out")
        except BaseException:
            self.close()
            raise

    def request(self, path, payload=None, timeout=120):
        request = urllib.request.Request(self.url + path,
            data=None if payload is None else json.dumps(payload).encode(),
            headers={"Authorization": "Bearer " + self.key, "Content-Type": "application/json"})
        with urllib.request.urlopen(request, timeout=timeout) as response:
            data = response.read(1024 * 1024 + 1)
        if len(data) > 1024 * 1024:
            raise ValueError("Oversized Qwen response")
        return json.loads(data)

    def transcribe(self, pcm):
        audio = io.BytesIO()
        with wave.open(audio, "wb") as wav:
            wav.setparams((1, 2, 16000, 0, "NONE", ""))
            wav.writeframes(pcm)
        if directory := os.environ.get("ORION_ASR_CAPTURE_DIR"):
            # Explicit diagnostic sessions only; never enabled by the installer.
            root = Path(directory); root.mkdir(mode=0o700, parents=True, exist_ok=True)
            if len(list(root.glob("*.wav"))) < 16:
                fd = os.open(root / f"{time.time_ns()}-{uuid.uuid4().hex}.wav", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                with os.fdopen(fd, "wb") as capture: capture.write(audio.getvalue())
        result = self.request("/v1/chat/completions", {
            "messages": [{"role": "system", "content": os.environ.get("ORION_ASR_CONTEXT", "Orion")},
                         {"role": "user", "content": [{"type": "input_audio", "input_audio": {
                "data": base64.b64encode(audio.getvalue()).decode(), "format": "wav"}}]}],
            "temperature": 0, "max_tokens": 1024, "cache_prompt": False,
        })["choices"][0]
        if result.get("finish_reason") == "length":
            raise ValueError("Qwen reached its output limit; transcript was not accepted")
        raw = result["message"]["content"]
        prefix, marker, text = raw.partition("<asr_text>")
        language = prefix.removeprefix("language ").strip() if marker else None
        return Transcript((text if marker else raw).strip(), language)

    def close(self):
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()


class PocketSynthesizer:
    provider = "pocket-tts"

    def __init__(self, model):
        import torch
        from pocket_tts import TTSModel

        if model not in ("pocket-fp32", "pocket-int8"):
            raise ValueError("Pocket model must be pocket-fp32 or pocket-int8")
        threads = int(os.environ.get("ORION_TTS_THREADS", "3"))
        if not 1 <= threads <= 4:
            raise ValueError("TTS threads must be between one and four")
        torch.set_num_threads(threads)
        torch.set_num_interop_threads(1)
        self.model_name = model
        self.model = TTSModel.load_model(quantize=model == "pocket-int8")
        self.states = {}

    def stream(self, text, voice="alba"):
        if voice not in VOICES:
            raise ValueError("Unknown Orion voice preset")
        if voice not in self.states:
            self.states[voice] = self.model.get_state_for_audio_prompt(voice)
        if self.model.sample_rate != 24000:
            raise ValueError("Pocket model must produce 24 kHz audio")
        for chunk in self.model.generate_audio_stream(self.states[voice], text):
            audio = chunk.detach().cpu().numpy().astype(np.float32).reshape(-1)
            if not audio.size or not np.isfinite(audio).all():
                raise ValueError("Pocket produced invalid audio")
            # Fixed gain preserves dynamics and avoids pumping between chunks.
            pcm = (np.clip(audio, -1, 1) * 32767).astype("<i2").tobytes()
            for offset in range(0, len(pcm), 96000):
                yield SpeechAudio(pcm[offset:offset+96000], 24000)
