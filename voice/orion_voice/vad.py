"""Stateful 16 kHz Silero inference, independent of capture and wake detection."""
from __future__ import annotations

from dataclasses import replace
import numpy as np

from .endpoint import EndpointConfig, EnergyEndpointDetector


class SileroModel:
    """Share immutable ONNX weights; each audio stream owns recurrent state."""

    def __init__(self, path, input_gain=2.0):
        import onnxruntime as ort

        options = ort.SessionOptions()
        options.intra_op_num_threads = 1
        options.inter_op_num_threads = 1
        self.session = ort.InferenceSession(str(path), sess_options=options,
                                           providers=["CPUExecutionProvider"])
        self.input_gain = input_gain

    def endpoint(self, config=EndpointConfig()):
        return SileroEndpoint(replace(config, trailing_silence_ms=1200), self.session, self.input_gain)


class SileroActivity:
    def __init__(self, session, input_gain=1.0):
        self.session = session
        if not np.isfinite(input_gain) or not 1 <= input_gain <= 8:
            raise ValueError("VAD gain must be between one and eight")
        self.input_gain = input_gain
        self.pending = bytearray()
        self.state = np.zeros((2, 1, 128), dtype=np.float32)
        self.context = np.zeros((1, 64), dtype=np.float32)
        self.speaking = False
        self.probability = 0.0

    def accept(self, pcm):
        self.pending.extend(pcm)
        # Capture supplies 320 samples; Silero requires 512. Never pad each
        # capture frame: that inserts artificial silence into the VAD history.
        while len(self.pending) >= 1024:
            block = bytes(self.pending[:1024])
            del self.pending[:1024]
            audio = np.frombuffer(block, dtype="<i2").astype(np.float32)[None, :] / 32768.0
            # ReSpeaker's quiet commands need 6 dB headroom for reliable VAD.
            # This branch does not change Rustpotter, the stored PCM, or ASR.
            audio = np.clip(audio * self.input_gain, -1.0, 1.0)
            window = np.concatenate((self.context, audio), axis=1)
            output, self.state = self.session.run(None, {
                "input": window, "state": self.state, "sr": np.array(16000, dtype=np.int64),
            })
            self.context = audio[:, -64:].copy()
            self.probability = float(output.reshape(-1)[0])
            if not np.isfinite(self.probability):
                raise ValueError("Silero returned an invalid speech probability")
            # Hysteresis prevents uncertain frames repeatedly toggling state.
            self.speaking = self.probability >= (0.35 if self.speaking else 0.5)
        return self.speaking


class SileroEndpoint(EnergyEndpointDetector):
    def __init__(self, config, session, input_gain=1.0):
        super().__init__(config)
        self.activity = SileroActivity(session, input_gain)

    def is_speech(self, pcm):
        return self.activity.accept(pcm)
