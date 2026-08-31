"""Single-owner wake, command-capture, and transcription loop."""

from __future__ import annotations

import time
from typing import Callable

from .speech import MoonshineTranscriber, SherpaSpeechSegmenter
from .wake import AlsaPcmCapture, SherpaWakeDetector, WakeEventPublisher


DEFAULT_COMMAND_TIMEOUT_SECONDS = 8.0


class VoiceLoopController:
    """Advance Orion's voice state machine one microphone chunk at a time."""

    def __init__(
        self,
        detector: SherpaWakeDetector,
        segmenter: SherpaSpeechSegmenter,
        transcriber: MoonshineTranscriber,
        publisher: WakeEventPublisher,
        *,
        command_timeout_seconds: float = DEFAULT_COMMAND_TIMEOUT_SECONDS,
        monotonic: Callable[[], float] = time.monotonic,
    ) -> None:
        if command_timeout_seconds <= 0:
            raise ValueError("command timeout must be positive")
        self._detector = detector
        self._segmenter = segmenter
        self._transcriber = transcriber
        self._publisher = publisher
        self._command_timeout_seconds = command_timeout_seconds
        self._monotonic = monotonic
        self._state = "wake_listening"
        self._command_started_at: float | None = None

    @property
    def state(self) -> str:
        return self._state

    def process_pcm(self, pcm: bytes) -> None:
        if self._state == "wake_listening":
            phrases = self._detector.accept_pcm16(pcm)
            if not phrases:
                return
            self._publisher.publish(phrases[0])
            self._segmenter.reset()
            self._state = "command_listening"
            self._command_started_at = self._monotonic()
            return

        try:
            segments = self._segmenter.accept_pcm16(pcm)
        except Exception as error:
            self._publisher.publish_command("failed", error=str(error))
            self._arm_wake_detection()
            return
        if segments:
            self._finish_command(segments[0])
            return

        assert self._command_started_at is not None
        if self._monotonic() - self._command_started_at >= self._command_timeout_seconds:
            self._publisher.publish_command("timed_out")
            self._arm_wake_detection()

    def _finish_command(self, samples) -> None:
        try:
            result = self._transcriber.transcribe(samples)
            if not result.text:
                self._publisher.publish_command(
                    "failed", error="speech recognizer returned an empty transcript"
                )
            else:
                self._publisher.publish_command(
                    "transcribed",
                    text=result.text,
                    audio_seconds=result.audio_seconds,
                    inference_seconds=result.inference_seconds,
                )
        except Exception as error:
            self._publisher.publish_command("failed", error=str(error))
        finally:
            self._arm_wake_detection()

    def _arm_wake_detection(self) -> None:
        self._segmenter.reset()
        self._state = "wake_listening"
        self._command_started_at = None


class VoiceLoopWorker:
    def __init__(
        self,
        controller: VoiceLoopController,
        capture: AlsaPcmCapture,
        publisher: WakeEventPublisher,
        wake_phrase: str,
    ) -> None:
        self._controller = controller
        self._capture = capture
        self._publisher = publisher
        self._wake_phrase = wake_phrase

    def serve_forever(self) -> None:
        self._publisher.open()
        try:
            self._capture.open()
            print(
                f"orion-listener: waiting for {self._wake_phrase}",
                flush=True,
            )
            while True:
                self._publisher.accept_pending()
                self._controller.process_pcm(self._capture.read())
        finally:
            self.close()

    def close(self) -> None:
        self._capture.close()
        self._publisher.close()
