import { VOICE_SAMPLE_RATE, WORKER_FRAME_SAMPLES } from "./voiceAudio";

export const VOICE_WORKER_PROTOCOL_VERSION = 5;

export interface VoiceWorkerReadyEvent {
  type: "ready";
  protocol: 5;
  asr: { provider: "qwen3-asr"; model: string };
  wake: { provider: "rustpotter"; model: string; threshold: number };
  agent: { provider: string; model: string };
  tts: { provider: "chatterbox-turbo"; model: string };
}

export interface WakeCandidateEvent {
  type: "wake.candidate";
  name: string;
  score: number;
}

export interface WakeConfirmedEvent {
  type: "wake.confirmed";
  text: string;
  hasCommand: boolean;
}

export interface WakeRejectedEvent {
  type: "wake.rejected";
  text: string;
}

export interface CommandStartedEvent {
  type: "command.started";
}

export interface TranscriptionStartedEvent {
  type: "transcription.started";
  purpose: "wake_and_command" | "command";
}

export interface TranscriptFinalEvent {
  type: "transcript.final";
  text: string;
  rawText: string;
  language: string | null;
  durationMs: number;
}

export interface AgentStartedEvent {
  type: "agent.started";
  requestId: number;
}

export interface AgentResponseEvent {
  type: "agent.response";
  requestId: number;
  text: string;
  durationMs: number;
}

export interface SynthesisStartedEvent {
  type: "synthesis.started";
  requestId: number;
}

interface SpeechAudioMetadataEvent {
  type: "speech.audio";
  requestId: number;
  sampleRate: number;
  samples: number;
  durationMs: number;
  synthesisMs: number;
}

export interface SpeechAudioEvent extends SpeechAudioMetadataEvent {
  pcm: Int16Array;
}

export interface SpeechCompletedEvent {
  type: "speech.completed";
  requestId: number;
}

export interface VoiceWorkerErrorEvent {
  type: "worker.error";
  code: string;
  message: string;
  recoverable: boolean;
}

export type VoiceWorkerEvent =
  | VoiceWorkerReadyEvent
  | WakeCandidateEvent
  | WakeConfirmedEvent
  | WakeRejectedEvent
  | CommandStartedEvent
  | TranscriptionStartedEvent
  | TranscriptFinalEvent
  | AgentStartedEvent
  | AgentResponseEvent
  | SynthesisStartedEvent
  | SpeechAudioEvent
  | SpeechCompletedEvent
  | VoiceWorkerErrorEvent;

type VoiceWorkerControlEvent = Exclude<VoiceWorkerEvent, SpeechAudioEvent> | SpeechAudioMetadataEvent;

export type VoiceWorkerListener = (event: VoiceWorkerEvent) => void;

export type VoiceWorkerConnectionPhase =
  | "disconnected"
  | "connecting"
  | "ready"
  | "error";

interface WorkerSocket {
  binaryType: BinaryType;
  readyState: number;
  onopen: ((event: Event) => void) | null;
  onmessage: ((event: MessageEvent) => void) | null;
  onerror: ((event: Event) => void) | null;
  onclose: ((event: CloseEvent) => void) | null;
  send(data: string | ArrayBuffer): void;
  close(code?: number, reason?: string): void;
}

export interface VoiceWorkerClientOptions {
  url: string;
  token: string;
  connectTimeoutMs?: number;
  createSocket?: (url: string) => WorkerSocket;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function parseVoiceWorkerEvent(data: unknown): VoiceWorkerControlEvent {
  if (typeof data !== "string") throw new Error("Voice worker sent a non-text control message.");
  const message: unknown = JSON.parse(data);
  if (!isRecord(message) || typeof message.type !== "string") {
    throw new Error("Voice worker sent an invalid control message.");
  }

  switch (message.type) {
    case "ready":
      if (
        message.protocol !== VOICE_WORKER_PROTOCOL_VERSION
        || !isRecord(message.asr)
        || !isRecord(message.wake)
        || !isRecord(message.agent)
        || !isRecord(message.tts)
        || message.wake.provider !== "rustpotter"
        || typeof message.wake.model !== "string"
        || typeof message.wake.threshold !== "number"
        || typeof message.agent.provider !== "string"
        || typeof message.agent.model !== "string"
        || message.tts.provider !== "chatterbox-turbo"
        || typeof message.tts.model !== "string"
      ) break;
      return message as unknown as VoiceWorkerReadyEvent;
    case "wake.candidate":
      if (typeof message.name !== "string" || typeof message.score !== "number") break;
      return message as unknown as WakeCandidateEvent;
    case "wake.confirmed":
      if (typeof message.text !== "string" || typeof message.hasCommand !== "boolean") break;
      return message as unknown as WakeConfirmedEvent;
    case "wake.rejected":
      if (typeof message.text !== "string") break;
      return message as unknown as WakeRejectedEvent;
    case "command.started":
      return { type: "command.started" };
    case "transcription.started":
      if (message.purpose !== "wake_and_command" && message.purpose !== "command") break;
      return message as unknown as TranscriptionStartedEvent;
    case "transcript.final":
      if (
        typeof message.text !== "string"
        || typeof message.rawText !== "string"
        || typeof message.durationMs !== "number"
      ) break;
      return message as unknown as TranscriptFinalEvent;
    case "agent.started":
      if (!isRequestId(message.requestId)) break;
      return message as unknown as AgentStartedEvent;
    case "agent.response":
      if (
        !isRequestId(message.requestId)
        || typeof message.text !== "string"
        || typeof message.durationMs !== "number"
      ) break;
      return message as unknown as AgentResponseEvent;
    case "synthesis.started":
      if (!isRequestId(message.requestId)) break;
      return message as unknown as SynthesisStartedEvent;
    case "speech.audio":
      if (
        !isRequestId(message.requestId)
        || !isPositiveInteger(message.sampleRate)
        || !isPositiveInteger(message.samples)
        || typeof message.durationMs !== "number"
        || typeof message.synthesisMs !== "number"
      ) break;
      return message as unknown as SpeechAudioMetadataEvent;
    case "speech.completed":
      if (!isRequestId(message.requestId)) break;
      return message as unknown as SpeechCompletedEvent;
    case "worker.error":
      if (typeof message.code !== "string" || typeof message.message !== "string" || typeof message.recoverable !== "boolean") break;
      return message as unknown as VoiceWorkerErrorEvent;
  }
  throw new Error(`Voice worker sent an unsupported ${String(message.type)} message.`);
}

function isPositiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value > 0;
}

function isRequestId(value: unknown): value is number {
  return isPositiveInteger(value);
}

function assertLocalWorkerUrl(url: string): void {
  const parsed = new URL(url);
  if (parsed.protocol !== "ws:" || !["127.0.0.1", "localhost", "[::1]"].includes(parsed.hostname)) {
    throw new Error("The voice worker must use an unencrypted localhost WebSocket.");
  }
}

/** One long-lived socket: JSON controls the session and binary frames carry PCM16. */
export class VoiceWorkerClient {
  private socket: WorkerSocket | null = null;
  private listeners = new Set<VoiceWorkerListener>();
  private phase: VoiceWorkerConnectionPhase = "disconnected";
  private pendingSpeech: SpeechAudioMetadataEvent | null = null;

  constructor(private readonly options: VoiceWorkerClientOptions) {
    assertLocalWorkerUrl(options.url);
  }

  currentPhase(): VoiceWorkerConnectionPhase {
    return this.phase;
  }

  subscribe(listener: VoiceWorkerListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  connect(): Promise<VoiceWorkerReadyEvent> {
    if (this.socket) throw new Error("Voice worker connection is already active.");
    this.phase = "connecting";
    const socket = this.options.createSocket?.(this.options.url)
      ?? new WebSocket(this.options.url) as unknown as WorkerSocket;
    socket.binaryType = "arraybuffer";
    this.socket = socket;

    return new Promise((resolve, reject) => {
      const timeout = globalThis.setTimeout(() => {
        this.phase = "error";
        socket.close(4000, "Worker handshake timed out");
        reject(new Error("Voice worker did not become ready in time."));
      }, this.options.connectTimeoutMs ?? 30_000);

      const fail = (error: Error) => {
        globalThis.clearTimeout(timeout);
        const wasConnecting = this.phase === "connecting";
        this.phase = "error";
        this.pendingSpeech = null;
        if (wasConnecting) reject(error);
        else this.emit({
          type: "worker.error",
          code: "protocol_error",
          message: error.message,
          recoverable: false,
        });
        if (socket.readyState < WebSocket.CLOSING) socket.close(4002, "Protocol error");
      };

      socket.onopen = () => {
        socket.send(JSON.stringify({
          type: "hello",
          protocol: VOICE_WORKER_PROTOCOL_VERSION,
          token: this.options.token,
          sampleRate: VOICE_SAMPLE_RATE,
          channels: 1,
          encoding: "pcm_s16le",
          frameSamples: WORKER_FRAME_SAMPLES,
        }));
      };
      socket.onmessage = (event) => {
        try {
          if (event.data instanceof ArrayBuffer) {
            this.acceptSpeechAudio(event.data);
            return;
          }
          if (this.pendingSpeech) {
            throw new Error("Voice worker did not send binary audio after speech metadata.");
          }
          const message = parseVoiceWorkerEvent(event.data);
          if (message.type === "speech.audio") {
            if (this.pendingSpeech) throw new Error("Voice worker sent overlapping speech audio.");
            this.pendingSpeech = message;
            return;
          }
          if (message.type === "ready") {
            globalThis.clearTimeout(timeout);
            this.phase = "ready";
            resolve(message);
          }
          if (message.type === "worker.error" && this.phase === "connecting" && !message.recoverable) {
            fail(new Error(message.message));
            return;
          }
          this.emit(message);
        } catch (error) {
          fail(error instanceof Error ? error : new Error(String(error)));
        }
      };
      socket.onerror = () => fail(new Error("Could not connect to the local voice worker."));
      socket.onclose = () => {
        globalThis.clearTimeout(timeout);
        this.socket = null;
        if (this.phase === "connecting") fail(new Error("Voice worker closed during startup."));
        else if (this.phase !== "error" && this.phase !== "disconnected") {
          this.phase = "disconnected";
          this.emit({ type: "worker.error", code: "worker_closed", message: "Voice connection closed. Reconnect Orion to continue.", recoverable: false });
        }
      };
    });
  }

  finishPlayback(requestId: number, error?: string): void {
    const socket = this.requireReadySocket();
    socket.send(JSON.stringify(error
      ? { type: "playback.failed", requestId, message: error }
      : { type: "playback.finished", requestId }));
  }

  close(): void {
    const socket = this.socket;
    this.socket = null;
    this.phase = "disconnected";
    this.pendingSpeech = null;
    if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: "stop" }));
    socket?.close(1000, "Studio voice stopped");
  }

  private requireReadySocket(): WorkerSocket {
    if (this.phase !== "ready" || !this.socket || this.socket.readyState !== WebSocket.OPEN) {
      throw new Error("Voice worker is not ready for audio.");
    }
    return this.socket;
  }

  private emit(event: VoiceWorkerEvent): void {
    for (const listener of this.listeners) listener(event);
  }

  private acceptSpeechAudio(buffer: ArrayBuffer): void {
    const metadata = this.pendingSpeech;
    this.pendingSpeech = null;
    if (!metadata) throw new Error("Voice worker sent unexpected binary audio.");
    if (buffer.byteLength !== metadata.samples * 2) {
      throw new Error("Voice worker speech audio does not match its metadata.");
    }
    const copy = buffer.slice(0);
    this.emit({ ...metadata, pcm: new Int16Array(copy) });
  }
}
