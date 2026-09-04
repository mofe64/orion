import { invoke } from "@tauri-apps/api/core";

import { OrionSpeechPlayer, type SpeechPlayer } from "./studioSpeaker";
import {
  VoiceWorkerClient,
  type VoiceWorkerEvent,
  type VoiceWorkerListener,
  type VoiceWorkerReadyEvent,
} from "./voiceWorkerProtocol";
import type { GatewayConnection } from "./gateway";

export type StudioVoicePhase =
  | "off"
  | "starting"
  | "ready"
  | "wake_candidate"
  | "confirming_wake"
  | "command_listening"
  | "transcribing"
  | "thinking"
  | "synthesizing"
  | "speaking"
  | "stopping"
  | "error";

export interface VoiceWorkerConnection {
  url: string;
  token: string;
  asrModel: string;
}

export interface VoiceWorkerLauncher {
  start(): Promise<VoiceWorkerConnection>;
  stop(): Promise<void>;
}

export interface VoiceWorkerTransport {
  connect(): Promise<VoiceWorkerReadyEvent>;
  subscribe(listener: VoiceWorkerListener): () => void;
  finishPlayback(requestId: number, error?: string): void;
  close(): void;
}

export interface StudioVoiceSnapshot {
  deviceLabel: string | null;
  sampleRate: number | null;
  error: string | null;
  phase: StudioVoicePhase;
  asrProvider: string | null;
  asrModel: string | null;
  wakeProvider: string | null;
  wakeModel: string | null;
  wakeThreshold: number | null;
  agentProvider: string | null;
  agentModel: string | null;
  ttsProvider: string | null;
  ttsModel: string | null;
  transcript: string | null;
  response: string | null;
}

export interface StudioVoicePipelineOptions {
  connection?: GatewayConnection;
  launcher?: VoiceWorkerLauncher;
  createTransport?: (connection: VoiceWorkerConnection) => VoiceWorkerTransport;
  speaker?: SpeechPlayer;
  retryDelayMs?: number;
}

export function piVoiceUrl(gatewayUrl: string): string {
  const url = new URL(gatewayUrl);
  url.protocol = "ws:";
  url.port = "7448";
  url.pathname = "/";
  url.search = "";
  url.hash = "";
  url.username = "";
  url.password = "";
  return url.toString();
}

class TauriVoiceWorkerLauncher implements VoiceWorkerLauncher {
  constructor(private readonly connection?: GatewayConnection) {}
  start(): Promise<VoiceWorkerConnection> {
    if (!this.connection) throw new Error("Connect Orion before starting Voice.");
    return invoke<VoiceWorkerConnection>("start_voice_worker", {
      piUrl: piVoiceUrl(this.connection.url), piToken: this.connection.token,
    });
  }

  stop(): Promise<void> {
    return invoke("stop_voice_worker");
  }
}

const INITIAL_SNAPSHOT: StudioVoiceSnapshot = {
  phase: "off",
  deviceLabel: null,
  sampleRate: null,
  error: null,
  asrProvider: null,
  asrModel: null,
  wakeProvider: null,
  wakeModel: null,
  wakeThreshold: null,
  agentProvider: null,
  agentModel: null,
  ttsProvider: null,
  ttsModel: null,
  transcript: null,
  response: null,
};

/** Studio processes Pi-triggered utterances; it never opens a microphone. */
export class StudioVoicePipeline {
  private snapshot: StudioVoiceSnapshot = { ...INITIAL_SNAPSHOT };
  private listeners = new Set<(snapshot: StudioVoiceSnapshot) => void>();
  private readonly launcher: VoiceWorkerLauncher;
  private readonly createTransport: (connection: VoiceWorkerConnection) => VoiceWorkerTransport;
  private readonly retryDelayMs: number;
  private readonly speaker: SpeechPlayer;
  private transport: VoiceWorkerTransport | null = null;
  private unsubscribeTransport: (() => void) | null = null;
  private operation = 0;

  constructor(options: StudioVoicePipelineOptions = {}) {
    this.launcher = options.launcher ?? new TauriVoiceWorkerLauncher(options.connection);
    this.createTransport = options.createTransport ?? ((connection) => new VoiceWorkerClient({
      url: connection.url,
      token: connection.token,
      connectTimeoutMs: 180_000,
    }));
    this.retryDelayMs = options.retryDelayMs ?? 100;
    this.speaker = options.speaker ?? new OrionSpeechPlayer(() => options.connection ?? null);
  }

  current(): StudioVoiceSnapshot {
    return { ...this.snapshot };
  }

  subscribe(listener: (snapshot: StudioVoiceSnapshot) => void): () => void {
    this.listeners.add(listener);
    listener(this.current());
    return () => this.listeners.delete(listener);
  }

  async start(): Promise<void> {
    if (this.snapshot.phase !== "off" && this.snapshot.phase !== "error") return;
    const operation = ++this.operation;
    this.publish({ ...INITIAL_SNAPSHOT, phase: "starting" });

    try {
      const connection = await this.launcher.start();
      if (operation !== this.operation) return;
      const { transport, ready, unsubscribe } = await this.connectWhenAvailable(connection, operation);
      if (operation !== this.operation) return;
      this.transport = transport;
      this.unsubscribeTransport = unsubscribe;
      this.publish({
        ...this.snapshot,
        asrProvider: ready.asr.provider,
        asrModel: ready.asr.model,
        wakeProvider: ready.wake.provider,
        wakeModel: ready.wake.model,
        wakeThreshold: ready.wake.threshold,
        agentProvider: ready.agent.provider,
        agentModel: ready.agent.model,
        ttsProvider: ready.tts.provider,
        ttsModel: ready.tts.model,
      });
      this.publish({ ...this.snapshot, phase: "ready", error: null,
        deviceLabel: "Orion ReSpeaker · Pi capture", sampleRate: 16_000 });
    } catch (error) {
      if (operation !== this.operation) return;
      await this.releaseResources();
      this.publish({
        ...this.snapshot,
        phase: "error",
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  async stop(): Promise<void> {
    if (this.snapshot.phase === "off") return;
    ++this.operation;
    this.publish({ ...this.snapshot, phase: "stopping" });
    await this.releaseResources();
    this.publish({ ...INITIAL_SNAPSHOT });
  }

  private async connectWhenAvailable(
    connection: VoiceWorkerConnection,
    operation: number,
  ): Promise<{
    transport: VoiceWorkerTransport;
    ready: VoiceWorkerReadyEvent;
    unsubscribe: () => void;
  }> {
    const deadline = Date.now() + 5_000;
    while (true) {
      const transport = this.createTransport(connection);
      const unsubscribe = transport.subscribe((event) => this.acceptWorkerEvent(event));
      this.transport = transport;
      this.unsubscribeTransport = unsubscribe;
      try {
        const ready = await transport.connect();
        return { transport, ready, unsubscribe };
      } catch (error) {
        unsubscribe();
        transport.close();
        if (this.transport === transport) this.transport = null;
        if (this.unsubscribeTransport === unsubscribe) this.unsubscribeTransport = null;
        if (operation !== this.operation) throw new Error("Voice startup was cancelled.");
        const message = error instanceof Error ? error.message : String(error);
        const workerNotBound = message.includes("Could not connect") || message.includes("closed during startup");
        if (!workerNotBound || Date.now() >= deadline) throw error;
        await new Promise((resolve) => globalThis.setTimeout(resolve, this.retryDelayMs));
      }
    }
  }

  private acceptWorkerEvent(event: VoiceWorkerEvent): void {
    switch (event.type) {
      case "wake.candidate":
        this.publish({ ...this.snapshot, phase: "wake_candidate", error: null });
        break;
      case "transcription.started":
        this.publish({
          ...this.snapshot,
          phase: event.purpose === "wake_and_command" ? "confirming_wake" : "transcribing",
        });
        break;
      case "wake.confirmed":
        if (!event.hasCommand) {
          this.publish({ ...this.snapshot, phase: "command_listening", error: null });
        }
        break;
      case "wake.rejected":
        this.publish({ ...this.snapshot, phase: "ready", error: null });
        break;
      case "command.started":
        this.publish({ ...this.snapshot, phase: "command_listening", error: null });
        break;
      case "transcript.final":
        this.publish({
          ...this.snapshot,
          phase: "thinking",
          transcript: event.text,
          error: null,
        });
        break;
      case "agent.started":
        this.publish({ ...this.snapshot, phase: "thinking", response: null, error: null });
        break;
      case "agent.response":
        this.publish({ ...this.snapshot, phase: "thinking", response: event.text, error: null });
        break;
      case "synthesis.started":
        this.publish({ ...this.snapshot, phase: "synthesizing", error: null });
        break;
      case "speech.audio":
        void this.playSpeech(event.requestId, event.pcm, event.sampleRate);
        break;
      case "speech.completed":
        this.publish({ ...this.snapshot, phase: "ready", error: null });
        break;
      case "worker.error":
        if (event.recoverable) {
          this.publish({ ...this.snapshot, phase: "ready", error: event.message });
        } else {
          void this.failPipeline(event.message);
        }
        break;
    }
  }

  private async playSpeech(requestId: number, pcm: Int16Array, sampleRate: number): Promise<void> {
    const operation = this.operation;
    this.publish({ ...this.snapshot, phase: "speaking", error: null });
    try {
      await this.speaker.play(pcm, sampleRate);
      if (operation === this.operation) this.transport?.finishPlayback(requestId);
    } catch (error) {
      if (operation !== this.operation) return;
      const message = error instanceof Error ? error.message : String(error);
      try {
        this.transport?.finishPlayback(requestId, message);
      } catch {
        await this.failPipeline(message);
      }
    }
  }

  private async failPipeline(message: string): Promise<void> {
    if (this.snapshot.phase === "off" || this.snapshot.phase === "stopping") return;
    ++this.operation;
    this.publish({ ...this.snapshot, phase: "error", error: message });
    await this.releaseResources();
  }

  private async releaseResources(): Promise<void> {
    await this.speaker.stop().catch(() => undefined);
    this.unsubscribeTransport?.();
    this.unsubscribeTransport = null;
    this.transport?.close();
    this.transport = null;
    await this.launcher.stop().catch(() => undefined);
  }

  private publish(snapshot: StudioVoiceSnapshot): void {
    this.snapshot = snapshot;
    for (const listener of this.listeners) listener(this.current());
  }
}
