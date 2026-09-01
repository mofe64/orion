from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
import re
from typing import Protocol

from . import SAMPLE_RATE
from .endpoint import EnergyEndpointDetector
from .providers import Transcript
from .wake import WakeDetection


class Transcriber(Protocol):
    provider: str
    model_name: str

    def transcribe(self, pcm: bytes) -> Transcript: ...


class WakeDetector(Protocol):
    provider: str
    model_name: str
    threshold: float

    def process(self, pcm: bytes) -> WakeDetection | None: ...
    def reset(self) -> None: ...


class SessionPhase(str, Enum):
    LISTENING = "listening"
    CAPTURING_WAKE = "capturing_wake"
    TRANSCRIBING_WAKE = "transcribing_wake"
    CAPTURING_COMMAND = "capturing_command"
    TRANSCRIBING_COMMAND = "transcribing_command"
    PROCESSING_RESPONSE = "processing_response"
    PLAYING_RESPONSE = "playing_response"


class TranscriptionPurpose(str, Enum):
    WAKE = "wake_and_command"
    COMMAND = "command"


@dataclass(frozen=True)
class SessionEvent:
    type: str
    fields: dict[str, object]


@dataclass(frozen=True)
class PendingTranscription:
    pcm: bytes
    purpose: TranscriptionPurpose


WAKE_PHRASE = re.compile(r"^\s*hey[\s,.:;!?-]+orion\b[\s,.:;!?-]*(.*)$", re.IGNORECASE)


def command_after_wake_phrase(text: str) -> str | None:
    match = WAKE_PHRASE.match(text)
    return match.group(1).strip() if match else None


class VoiceSession:
    """Owns continuous wake detection, endpointing, and ASR confirmation state."""

    def __init__(self, asr: Transcriber, wake: WakeDetector, pre_roll_seconds: int = 3) -> None:
        self.asr = asr
        self.wake = wake
        self.phase = SessionPhase.LISTENING
        self._pre_roll_limit = pre_roll_seconds * SAMPLE_RATE * 2
        self._pre_roll = bytearray()
        self._utterance = bytearray()
        self._endpoint = EnergyEndpointDetector()
        self._playback_request_id: int | None = None

    def accept_audio(self, pcm: bytes) -> tuple[list[SessionEvent], PendingTranscription | None]:
        if len(pcm) % 2:
            return [SessionEvent("worker.error", {
                "code": "invalid_pcm",
                "message": "PCM16 audio must contain complete two-byte samples.",
                "recoverable": True,
            })], None

        if self.phase is SessionPhase.LISTENING:
            self._append_pre_roll(pcm)
            detection = self.wake.process(pcm)
            if detection is None:
                return [], None
            self._begin_wake_capture()
            return [SessionEvent("wake.candidate", {
                "name": detection.name,
                "score": detection.score,
            })], None

        if self.phase in {SessionPhase.CAPTURING_WAKE, SessionPhase.CAPTURING_COMMAND}:
            self._utterance.extend(pcm)
            if self._endpoint.accept(pcm):
                purpose = (
                    TranscriptionPurpose.WAKE
                    if self.phase is SessionPhase.CAPTURING_WAKE
                    else TranscriptionPurpose.COMMAND
                )
                self.phase = (
                    SessionPhase.TRANSCRIBING_WAKE
                    if purpose is TranscriptionPurpose.WAKE
                    else SessionPhase.TRANSCRIBING_COMMAND
                )
                return [], PendingTranscription(bytes(self._utterance), purpose)
        return [], None

    def complete_transcription(
        self,
        purpose: TranscriptionPurpose,
        transcript: Transcript,
    ) -> list[SessionEvent]:
        self._require_transcription(purpose)
        text = transcript.text.strip()

        if purpose is TranscriptionPurpose.WAKE:
            command = command_after_wake_phrase(text)
            if command is None:
                self._return_to_listening()
                return [SessionEvent("wake.rejected", {"text": text})]

            confirmed = SessionEvent("wake.confirmed", {
                "text": text,
                "hasCommand": bool(command),
            })
            if command:
                self.phase = SessionPhase.PROCESSING_RESPONSE
                return [confirmed, SessionEvent("transcript.final", {
                    "text": command,
                    "rawText": text,
                })]

            self._begin_followup_capture()
            return [confirmed, SessionEvent("command.started", {})]

        self.phase = SessionPhase.PROCESSING_RESPONSE
        return [SessionEvent("transcript.final", {"text": text, "rawText": text})]

    def fail_transcription(self, purpose: TranscriptionPurpose) -> None:
        self._require_transcription(purpose)
        self._return_to_listening()

    def begin_playback(self, request_id: int) -> None:
        if self.phase is not SessionPhase.PROCESSING_RESPONSE:
            raise ValueError("No agent response is ready for playback.")
        if request_id <= 0:
            raise ValueError("Playback request ID must be positive.")
        self._playback_request_id = request_id
        self.phase = SessionPhase.PLAYING_RESPONSE

    def finish_playback(self, request_id: int) -> None:
        if self.phase is not SessionPhase.PLAYING_RESPONSE:
            raise ValueError("No agent response is playing.")
        if request_id != self._playback_request_id:
            raise ValueError("Playback acknowledgement does not match the active response.")
        self._return_to_listening()

    def fail_response(self) -> None:
        if self.phase not in {SessionPhase.PROCESSING_RESPONSE, SessionPhase.PLAYING_RESPONSE}:
            raise ValueError("No agent response is active.")
        self._return_to_listening()

    def _begin_wake_capture(self) -> None:
        self.phase = SessionPhase.CAPTURING_WAKE
        self._utterance = bytearray(self._pre_roll)
        self._endpoint = EnergyEndpointDetector()
        self._endpoint.prime_detected_speech()
        self._pre_roll.clear()

    def _begin_followup_capture(self) -> None:
        self.phase = SessionPhase.CAPTURING_COMMAND
        self._utterance.clear()
        self._endpoint = EnergyEndpointDetector()

    def _append_pre_roll(self, pcm: bytes) -> None:
        self._pre_roll.extend(pcm)
        if len(self._pre_roll) > self._pre_roll_limit:
            del self._pre_roll[:-self._pre_roll_limit]

    def _require_transcription(self, purpose: TranscriptionPurpose) -> None:
        expected = (
            SessionPhase.TRANSCRIBING_WAKE
            if purpose is TranscriptionPurpose.WAKE
            else SessionPhase.TRANSCRIBING_COMMAND
        )
        if self.phase is not expected:
            raise ValueError(f"No {purpose.value} transcription is active.")

    def _return_to_listening(self) -> None:
        self.phase = SessionPhase.LISTENING
        self._pre_roll.clear()
        self._utterance.clear()
        self._endpoint = EnergyEndpointDetector()
        self._playback_request_id = None
        self.wake.reset()
