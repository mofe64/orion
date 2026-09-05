import { cancelRun, getSpeechRun, uploadSpeech, uploadSpeechChunk, endSpeechStream, type GatewayConnection } from "./gateway";
import { pcm16MonoWav } from "./voiceAudio";

export interface SpeechPlayer {
  play(pcm: Int16Array, sampleRate: number): Promise<void>;
  append?(pcm: Int16Array, sampleRate: number, sequence: number): Promise<void>;
  finish?(sequence: number): Promise<void>;
  stop(): Promise<void>;
}

export interface OrionPlaybackSnapshot {
  firstPlaybackMs?: number | null;
  uploadMs?: number;
  elapsedMs?: number;
  runId: number | null;
  state: "idle" | "uploading" | "queued" | "playing" | "completed" | "cancelled" | "failed";
}

/** Routes synthesized speech to Orion and resolves only after Pi playback ends. */
export class OrionSpeechPlayer implements SpeechPlayer {
  private operation = 0;
  private uploading = false;
  private active: { connection: GatewayConnection; runId: number; cancelled: boolean } | null = null;

  private nextChunk = 0;
  private streamRequest = "";
  private streamCompletion: Promise<void> | null = null;
  private uploadMs = 0;

  async append(pcm: Int16Array, sampleRate: number, sequence: number): Promise<void> {
    if (sampleRate !== 24_000 || pcm.length === 0 || pcm.length > 48_000) throw new Error("Invalid streaming audio chunk.");
    const connection = this.connection();
    if (!connection) throw new Error("Connect Orion before streaming speech.");
    if (sequence === 0) {
      if (this.active || this.uploading) throw new Error("Speech is already active.");
      ++this.operation;
      this.nextChunk = 0;
      this.uploadMs = 0;
      this.streamRequest = crypto.randomUUID();
    }
    if (sequence !== this.nextChunk || (sequence > 0 && !this.active)) throw new Error("Out-of-order speech chunk.");
    const operation = this.operation;
    const started = performance.now();
    this.uploading = true;
    let accepted;
    try { accepted = await uploadSpeechChunk(connection, pcm16MonoWav(pcm, sampleRate), this.streamRequest, this.active?.runId, sequence); }
    finally { this.uploading = false; }
    if (operation !== this.operation) {
      if (sequence === 0) await cancelRun(connection, "speech", accepted.run_id).catch(() => undefined);
      throw new Error("Speech stream was cancelled during upload.");
    }
    this.uploadMs += performance.now() - started;
    this.nextChunk++;
    if (sequence === 0) {
      const run = { connection, runId: accepted.run_id, cancelled: false };
      this.active = run;
      this.streamCompletion = this.observeStream(run);
      void this.streamCompletion.catch(() => undefined);
    }
  }

  async finish(sequence: number): Promise<void> {
    const run = this.active;
    if (!run || sequence !== this.nextChunk) throw new Error("Invalid speech stream end.");
    const completion = this.streamCompletion;
    await endSpeechStream(run.connection, run.runId, sequence);
    try { await completion; }
    finally { if (this.active === run) { this.active = null; this.streamCompletion = null; } }
  }

  private async observeStream(run: { connection: GatewayConnection; runId: number; cancelled: boolean }): Promise<void> {
    const deadline = performance.now() + 150_000;
    while (!run.cancelled && performance.now() < deadline) {
      const status = await getSpeechRun(run.connection, run.runId);
      if (run.cancelled) throw new Error("Speech stream cancelled.");
      this.onStatus({ runId: run.runId, state: status.state, firstPlaybackMs: status.first_playback_ms, elapsedMs: status.elapsed_ms, uploadMs: Math.round(this.uploadMs) });
      if (status.state === "completed") return;
      if (status.state === "failed" || status.state === "cancelled") throw new Error(status.error ?? "Speech stream stopped.");
      await new Promise(resolve => globalThis.setTimeout(resolve, 100));
    }
    throw new Error("Speech stream cancelled or timed out.");
  }

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
