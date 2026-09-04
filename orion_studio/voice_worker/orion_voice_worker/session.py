"""Processing state for Pi-triggered utterances; no microphone or wake detector."""
from dataclasses import dataclass
from enum import Enum
import re


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
    fields: dict


@dataclass(frozen=True)
class PendingTranscription:
    pcm: bytes
    purpose: TranscriptionPurpose


WAKE_PHRASE = re.compile(r"^\s*hey[\s,.:;!?-]+orion\b[\s,.:;!?-]*(.*)$", re.IGNORECASE)


def command_after_wake_phrase(text):
    match = WAKE_PHRASE.match(text)
    return match.group(1).strip() if match else None


class VoiceSession:
    def __init__(self, asr):
        self.asr = asr
        self.reset()

    def reset(self):
        self.phase = SessionPhase.LISTENING
        self.session_id = None
        self._playback_request_id = None

    def begin(self, session_id):
        if self.session_id is not None or not isinstance(session_id, str) or not re.fullmatch(r"[a-f0-9]{32}", session_id):
            raise ValueError("Invalid or overlapping Pi voice session")
        self.session_id = session_id
        self.phase = SessionPhase.CAPTURING_WAKE

    def accept_utterance(self, session_id, purpose, pcm):
        purpose = TranscriptionPurpose(purpose)
        expected = SessionPhase.CAPTURING_WAKE if purpose is TranscriptionPurpose.WAKE else SessionPhase.CAPTURING_COMMAND
        if session_id != self.session_id or self.phase is not expected:
            raise ValueError("Stale or unexpected Pi utterance")
        if not pcm or len(pcm) % 2 or len(pcm) > 18 * 32000:
            raise ValueError("Invalid or oversized PCM16 utterance")
        self.phase = SessionPhase.TRANSCRIBING_WAKE if purpose is TranscriptionPurpose.WAKE else SessionPhase.TRANSCRIBING_COMMAND
        return PendingTranscription(pcm, purpose)

    def complete_transcription(self, purpose, transcript):
        self._require_transcription(purpose)
        text = transcript.text.strip()
        if purpose is TranscriptionPurpose.WAKE:
            command = command_after_wake_phrase(text)
            if command is None:
                self.reset()
                return [SessionEvent("wake.rejected", {"text": text})]
            confirmed = SessionEvent("wake.confirmed", {"text": text, "hasCommand": bool(command)})
            if not command:
                self.phase = SessionPhase.CAPTURING_COMMAND
                return [confirmed, SessionEvent("command.started", {})]
            self.phase = SessionPhase.PROCESSING_RESPONSE
            return [confirmed, SessionEvent("transcript.final", {"text": command, "rawText": text})]
        if not text:
            raise ValueError("No command was heard")
        self.phase = SessionPhase.PROCESSING_RESPONSE
        return [SessionEvent("transcript.final", {"text": text, "rawText": text})]

    def _require_transcription(self, purpose):
        expected = SessionPhase.TRANSCRIBING_WAKE if purpose is TranscriptionPurpose.WAKE else SessionPhase.TRANSCRIBING_COMMAND
        if self.phase is not expected:
            raise ValueError("No matching transcription is active")

    def fail_transcription(self, purpose):
        self._require_transcription(purpose)
        self.reset()

    def begin_playback(self, request_id):
        if self.phase is not SessionPhase.PROCESSING_RESPONSE or request_id <= 0:
            raise ValueError("No response is ready for playback")
        self._playback_request_id = request_id
        self.phase = SessionPhase.PLAYING_RESPONSE

    def finish_playback(self, request_id):
        if self.phase is not SessionPhase.PLAYING_RESPONSE or request_id != self._playback_request_id:
            raise ValueError("Playback acknowledgement does not match the active response")
        self.reset()

    def fail_response(self):
        self.reset()
