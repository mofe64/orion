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
  muted?: boolean;
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
  agentEffort?: string;
  runtime?: string;
  models?: { model: string; name: string; efforts: string[] }[];
  latency?: Record<string, number>;
}

export interface VoiceSettings { model: string; effort: string; }
export const DEFAULT_VOICE_SETTINGS: VoiceSettings = { model: "gpt-5.6-sol", effort: "medium" };

export interface StudioVoicePipelineOptions {
  settings?: VoiceSettings;
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
  constructor(private readonly connection?: GatewayConnection, private readonly settings = DEFAULT_VOICE_SETTINGS) {}
  start(): Promise<VoiceWorkerConnection> {
    if (!this.connection) throw new Error("Connect Orion before starting Voice.");
    return invoke<VoiceWorkerConnection>("start_voice_worker", {
      gatewayUrl: this.connection.url, piUrl: piVoiceUrl(this.connection.url), piToken: this.connection.token, agentModel: this.settings.model, agentEffort: this.settings.effort,
    });
  }

  stop(): Promise<void> {
    // The native application owns the child; detaching a panel does not stop it.
    return Promise.resolve();
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
  private streamQueue = Promise.resolve();
  private streamFailed = false;
  private speechEpoch = 0;
  private streamBytes = 0;

  constructor(options: StudioVoicePipelineOptions = {}) {
    this.launcher = options.launcher ?? new TauriVoiceWorkerLauncher(options.connection, options.settings);
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
        muted: ready.muted ?? false,
        asrProvider: ready.asr.provider,
        asrModel: ready.asr.model,
        wakeProvider: ready.wake.provider,
        wakeModel: ready.wake.model,
        wakeThreshold: ready.wake.threshold,
        agentProvider: ready.agent.provider,
        agentModel: ready.agent.model,
        agentEffort: ready.agent.effort,
        runtime: ready.agent.runtime,
        models: ready.agent.models,
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

  async setMuted(muted: boolean): Promise<void> {
    const status = await invoke<{ muted: boolean }>("set_voice_microphone", { muted });
    this.publish({ ...this.snapshot, muted: status.muted });
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
      case "ready":
        this.publish({ ...this.snapshot, phase: "ready", error: null, muted: event.muted ?? false,
          asrProvider: event.asr.provider, asrModel: event.asr.model,
          wakeProvider: event.wake.provider, wakeModel: event.wake.model, wakeThreshold: event.wake.threshold,
          agentProvider: event.agent.provider, agentModel: event.agent.model, agentEffort: event.agent.effort,
          runtime: event.agent.runtime, models: event.agent.models,
          ttsProvider: event.tts.provider, ttsModel: event.tts.model });
        break;
      case "microphone.status":
        this.publish({ ...this.snapshot, muted: event.muted });
        break;
      case "speech.started":
        this.publish({ ...this.snapshot, phase: "speaking" });
        break;
      case "wake.candidate":
        ++this.speechEpoch;
        this.snapshot.latency = {};
        this.streamFailed = false;
        this.streamBytes = 0;
        this.streamQueue = Promise.resolve();
        this.publish({ ...this.snapshot, phase: "wake_candidate", error: null });
        break;
      case "stage.timing":
        this.publish({ ...this.snapshot, latency: { ...this.snapshot.latency, [event.stage]: event.durationMs } });
        break;
      case "transcription.started":
        if (typeof event.captureMs === "number") this.snapshot.latency = { ...this.snapshot.latency, ["capture_" + event.purpose]: event.captureMs };
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
        this.snapshot.latency = { ...this.snapshot.latency, transcriptionMs: event.durationMs };
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
        this.snapshot.latency = { ...this.snapshot.latency, agentMs: event.durationMs };
        this.publish({ ...this.snapshot, phase: "thinking", response: event.text, error: null });
        break;
      case "synthesis.started":
        this.publish({ ...this.snapshot, phase: "synthesizing", error: null });
        break;
      case "speech.chunk": {
        if (event.sequence === 0) this.snapshot.latency = { ...this.snapshot.latency, synthesisFirstChunkMs: event.synthesisMs };
        this.streamBytes += event.pcm.byteLength;
        const operation = this.operation;
        const epoch = this.speechEpoch;
        this.streamQueue = this.streamQueue.then(async () => {
          if (operation !== this.operation || epoch !== this.speechEpoch || this.streamFailed) return;
          if (!this.speaker.append || this.streamBytes > 120 * 48_000) throw new Error("Streaming playback is unavailable or oversized.");
          await this.speaker.append(event.pcm, event.sampleRate, event.sequence!);
          if (operation !== this.operation || epoch !== this.speechEpoch || this.streamFailed) return;
          this.publish({ ...this.snapshot, phase: "speaking" });
        }).catch(error => { if (epoch === this.speechEpoch) return this.failStream(event.requestId, operation, error); });
        break;
      }
      case "speech.end": {
        this.publish({ ...this.snapshot, latency: { ...this.snapshot.latency, synthesisTotalMs: event.synthesisMs } });
        const operation = this.operation;
        const epoch = this.speechEpoch;
        this.streamQueue = this.streamQueue.then(async () => {
          if (operation !== this.operation || epoch !== this.speechEpoch || this.streamFailed) return;
          if (!this.speaker.finish) throw new Error("Streaming playback is unavailable.");
          await this.speaker.finish(event.sequence);
          if (operation === this.operation && epoch === this.speechEpoch && !this.streamFailed) this.transport?.finishPlayback(event.requestId);
        }).catch(error => { if (epoch === this.speechEpoch) return this.failStream(event.requestId, operation, error); });
        break;
      }
      case "speech.audio":
        void this.playSpeech(event.requestId, event.pcm, event.sampleRate);
        break;
      case "speech.completed":
        this.publish({ ...this.snapshot, phase: "ready", error: null });
        break;
      case "worker.error":
        this.streamFailed = true;
        void this.speaker.stop();
        if (event.recoverable) {
          this.publish({ ...this.snapshot, phase: "ready", error: event.message });
        } else {
          void this.failPipeline(event.message);
        }
        break;
    }
  }

  private async failStream(requestId: number, operation: number, error: unknown): Promise<void> {
    if (operation !== this.operation || this.streamFailed) return;
    this.streamFailed = true;
    const epoch = this.speechEpoch;
    await this.speaker.stop().catch(() => undefined);
    if (operation !== this.operation || epoch !== this.speechEpoch) return;
    const message = error instanceof Error ? error.message : String(error);
    try { this.transport?.finishPlayback(requestId, message); }
    catch { await this.failPipeline(message); }
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
