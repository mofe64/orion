import { cancelRun, getSpeechRun, uploadSpeech, type GatewayConnection } from "./gateway";
import { pcm16MonoWav } from "./voiceAudio";

export interface SpeechPlayer {
  play(pcm: Int16Array, sampleRate: number): Promise<void>;
  stop(): Promise<void>;
}

export interface OrionPlaybackSnapshot {
  runId: number | null;
  state: "idle" | "uploading" | "queued" | "playing" | "completed" | "cancelled" | "failed";
}

/** Routes synthesized speech to Orion and resolves only after Pi playback ends. */
export class OrionSpeechPlayer implements SpeechPlayer {
  private active: { connection: GatewayConnection; runId: number; cancelled: boolean } | null = null;

  constructor(
    private readonly connection: () => GatewayConnection | null,
    private readonly onStatus: (status: OrionPlaybackSnapshot) => void = () => undefined,
  ) {}

  async play(pcm: Int16Array, sampleRate: number): Promise<void> {
    if (this.active) throw new Error("Orion speech playback is already active.");
    const connection = this.connection();
    if (!connection) throw new Error("Connect Studio to Orion before enabling Voice playback.");
    const wav = pcm16MonoWav(pcm, sampleRate);
    const durationMs = Math.ceil((wav.byteLength - 44) / 2 / 24_000 * 1_000);
    this.onStatus({ runId: null, state: "uploading" });
    const accepted = await uploadSpeech(connection, wav, crypto.randomUUID());
    const run = { connection, runId: accepted.run_id, cancelled: false };
    this.active = run;
    this.onStatus({ runId: accepted.run_id, state: accepted.state });
    const deadline = Date.now() + durationMs + 30_000;
    try {
      while (!run.cancelled) {
        if (Date.now() > deadline) throw new Error("Orion did not report speech completion before the playback deadline.");
        await new Promise((resolve) => globalThis.setTimeout(resolve, 250));
        if (run.cancelled) break;
        const status = await getSpeechRun(connection, accepted.run_id);
        this.onStatus({ runId: accepted.run_id, state: status.state });
        if (status.state === "completed") return;
        if (status.state === "cancelled") throw new Error("Orion speech playback was cancelled.");
        if (status.state === "failed") throw new Error(status.error ?? "Orion could not play the speech audio.");
      }
      throw new Error("Orion speech playback was cancelled.");
    } finally {
      if (this.active === run) this.active = null;
    }
  }

  async stop(): Promise<void> {
    const active = this.active;
    if (active) active.cancelled = true;
    if (active) await cancelRun(active.connection, "speech", active.runId).catch(() => undefined);
    if (this.active === active) this.active = null;
    this.onStatus({ runId: active?.runId ?? null, state: active ? "cancelled" : "idle" });
  }
}

export class StudioSpeaker implements SpeechPlayer {
  private context: AudioContext | null = null;
  private source: AudioBufferSourceNode | null = null;

  async play(pcm: Int16Array, sampleRate: number): Promise<void> {
    if (pcm.length === 0 || sampleRate <= 0) throw new Error("Speech audio is empty or invalid.");
    if (this.source) throw new Error("Speech playback is already active.");

    const context = this.context ?? new AudioContext();
    this.context = context;
    if (context.state === "suspended") await context.resume();

    const buffer = context.createBuffer(1, pcm.length, sampleRate);
    const channel = buffer.getChannelData(0);
    for (let index = 0; index < pcm.length; index += 1) channel[index] = pcm[index] / 32_768;

    const source = context.createBufferSource();
    source.buffer = buffer;
    source.connect(context.destination);
    this.source = source;
    await new Promise<void>((resolve) => {
      source.onended = () => {
        if (this.source === source) this.source = null;
        resolve();
      };
      source.start();
    });
  }

  async stop(): Promise<void> {
    const source = this.source;
    this.source = null;
    if (source) {
      source.onended = null;
      source.stop();
      source.disconnect();
    }
    const context = this.context;
    this.context = null;
    if (context && context.state !== "closed") await context.close();
  }
}
