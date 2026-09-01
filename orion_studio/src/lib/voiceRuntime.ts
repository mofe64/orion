export type VoiceCapturePhase = "off" | "starting" | "listening" | "stopping" | "error";

export interface VoiceAudioFrame {
  samples: Float32Array;
  sampleRate: number;
  channels: 1;
  sequence: number;
  capturedAtMs: number;
}

export interface VoiceCaptureInfo {
  deviceLabel: string;
  sampleRate: number;
  channels: 1;
}

export interface VoiceCaptureSource {
  start(
    onFrame: (frame: VoiceAudioFrame) => void,
    onError: (error: unknown) => void,
  ): Promise<VoiceCaptureInfo>;
  stop(): Promise<void>;
}

export interface VoiceCaptureSnapshot {
  phase: VoiceCapturePhase;
  deviceLabel: string | null;
  sampleRate: number | null;
  levelDbfs: number | null;
  frameCount: number;
  error: string | null;
}

export type VoiceCaptureListener = (snapshot: VoiceCaptureSnapshot) => void;

const OFF_SNAPSHOT: VoiceCaptureSnapshot = {
  phase: "off",
  deviceLabel: null,
  sampleRate: null,
  levelDbfs: null,
  frameCount: 0,
  error: null,
};

export function audioLevelDbfs(samples: Float32Array): number {
  if (samples.length === 0) return -120;
  let sumOfSquares = 0;
  for (const sample of samples) sumOfSquares += sample * sample;
  const rms = Math.sqrt(sumOfSquares / samples.length);
  if (rms <= Number.EPSILON) return -120;
  return Math.max(-120, Math.min(0, 20 * Math.log10(rms)));
}

export function describeVoiceCaptureError(error: unknown): string {
  const name = error instanceof Error ? error.name : "";
  if (name === "NotAllowedError" || name === "SecurityError") {
    return "Microphone access was denied. Allow Orion Studio in the operating system's microphone settings, then try again.";
  }
  if (name === "NotFoundError" || name === "DevicesNotFoundError") {
    return "No microphone is available to Orion Studio.";
  }
  if (name === "NotReadableError" || name === "TrackStartError") {
    return "The selected microphone is busy or could not be opened.";
  }
  if (error instanceof Error && error.message.trim()) return error.message;
  return "Orion Studio could not start microphone capture.";
}

/**
 * Owns microphone lifecycle without knowing whether capture comes from a
 * browser WebView, a native adapter, or a test double. Future activation/STT model
 * adapters subscribe downstream from this boundary.
 */
export class VoiceCaptureRuntime {
  private snapshot: VoiceCaptureSnapshot = { ...OFF_SNAPSHOT };
  private listeners = new Set<VoiceCaptureListener>();
  private operation = 0;

  constructor(
    private readonly source: VoiceCaptureSource,
    private readonly frameSink?: (frame: VoiceAudioFrame) => void,
  ) {}

  current(): VoiceCaptureSnapshot {
    return { ...this.snapshot };
  }

  subscribe(listener: VoiceCaptureListener): () => void {
    this.listeners.add(listener);
    listener(this.current());
    return () => this.listeners.delete(listener);
  }

  async start(): Promise<void> {
    if (this.snapshot.phase === "starting" || this.snapshot.phase === "listening") return;
    const operation = ++this.operation;
    this.publish({
      phase: "starting",
      deviceLabel: null,
      sampleRate: null,
      levelDbfs: null,
      frameCount: 0,
      error: null,
    });

    try {
      const info = await this.source.start(
        (frame) => this.acceptFrame(operation, frame),
        (error) => this.fail(operation, error),
      );
      if (operation !== this.operation) return;
      this.publish({
        phase: "listening",
        deviceLabel: info.deviceLabel,
        sampleRate: info.sampleRate,
        levelDbfs: -120,
        frameCount: 0,
        error: null,
      });
    } catch (error) {
      if (operation !== this.operation) return;
      await this.source.stop().catch(() => undefined);
      this.publish({
        phase: "error",
        deviceLabel: null,
        sampleRate: null,
        levelDbfs: null,
        frameCount: 0,
        error: describeVoiceCaptureError(error),
      });
    }
  }

  async stop(): Promise<void> {
    if (this.snapshot.phase === "off") return;
    const operation = ++this.operation;
    this.publish({ ...this.snapshot, phase: "stopping", levelDbfs: null, error: null });
    try {
      await this.source.stop();
    } finally {
      if (operation === this.operation) this.publish({ ...OFF_SNAPSHOT });
    }
  }

  private acceptFrame(operation: number, frame: VoiceAudioFrame): void {
    if (operation !== this.operation || this.snapshot.phase !== "listening") return;
    try {
      this.frameSink?.(frame);
    } catch (error) {
      this.fail(operation, error);
      return;
    }
    this.publish({
      ...this.snapshot,
      sampleRate: frame.sampleRate,
      levelDbfs: audioLevelDbfs(frame.samples),
      frameCount: this.snapshot.frameCount + 1,
    });
  }

  private fail(operation: number, error: unknown): void {
    if (operation !== this.operation || this.snapshot.phase === "off") return;
    ++this.operation;
    void this.source.stop().catch(() => undefined);
    this.publish({
      ...this.snapshot,
      phase: "error",
      levelDbfs: null,
      error: describeVoiceCaptureError(error),
    });
  }

  private publish(snapshot: VoiceCaptureSnapshot): void {
    this.snapshot = snapshot;
    for (const listener of this.listeners) listener(this.current());
  }
}
