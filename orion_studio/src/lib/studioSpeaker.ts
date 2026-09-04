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
  private operation = 0;
  private uploading = false;
  private active: { connection: GatewayConnection; runId: number; cancelled: boolean } | null = null;

  constructor(
    private readonly connection: () => GatewayConnection | null,
    private readonly onStatus: (status: OrionPlaybackSnapshot) => void = () => undefined,
  ) {}

  async play(pcm: Int16Array, sampleRate: number): Promise<void> {
    if (this.active || this.uploading) throw new Error("Orion speech playback is already active.");
    const operation = ++this.operation;
    const connection = this.connection();
    if (!connection) throw new Error("Connect Studio to Orion before enabling Voice playback.");
    const wav = pcm16MonoWav(pcm, sampleRate);
    const durationMs = Math.ceil((wav.byteLength - 44) / 2 / 24_000 * 1_000);
    this.onStatus({ runId: null, state: "uploading" });
    this.uploading = true;
    let accepted;
    try {
      accepted = await uploadSpeech(connection, wav, crypto.randomUUID());
    } finally {
      this.uploading = false;
    }
    if (operation !== this.operation) {
      await cancelRun(connection, "speech", accepted.run_id).catch(() => undefined);
      throw new Error("Orion speech playback was cancelled during upload.");
    }
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
    ++this.operation;
    const active = this.active;
    if (active) active.cancelled = true;
    if (active) await cancelRun(active.connection, "speech", active.runId).catch(() => undefined);
    if (this.active === active) this.active = null;
    this.onStatus({ runId: active?.runId ?? null, state: active ? "cancelled" : "idle" });
  }
}
