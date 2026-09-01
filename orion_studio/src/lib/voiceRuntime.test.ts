import { describe, expect, it } from "vitest";

import {
  VoiceCaptureRuntime,
  audioLevelDbfs,
  describeVoiceCaptureError,
  type VoiceAudioFrame,
  type VoiceCaptureSource,
} from "./voiceRuntime";

class FakeCapture implements VoiceCaptureSource {
  onFrame: ((frame: VoiceAudioFrame) => void) | null = null;
  onError: ((error: unknown) => void) | null = null;
  startError: Error | null = null;
  stopCount = 0;

  async start(onFrame: (frame: VoiceAudioFrame) => void, onError: (error: unknown) => void) {
    this.onFrame = onFrame;
    this.onError = onError;
    if (this.startError) throw this.startError;
    return { deviceLabel: "Test microphone", sampleRate: 48_000, channels: 1 as const };
  }

  async stop() {
    this.stopCount += 1;
  }
}

function frame(samples: number[]): VoiceAudioFrame {
  return {
    samples: Float32Array.from(samples),
    sampleRate: 48_000,
    channels: 1,
    sequence: 0,
    capturedAtMs: 0,
  };
}

describe("VoiceCaptureRuntime", () => {
  it("starts, reports microphone frames, and stops without retaining audio", async () => {
    const capture = new FakeCapture();
    const runtime = new VoiceCaptureRuntime(capture);
    const phases: string[] = [];
    runtime.subscribe((snapshot) => phases.push(snapshot.phase));

    await runtime.start();
    capture.onFrame?.(frame([0.5, -0.5]));

    expect(runtime.current()).toMatchObject({
      phase: "listening",
      deviceLabel: "Test microphone",
      sampleRate: 48_000,
      frameCount: 1,
    });
    expect(runtime.current().levelDbfs).toBeCloseTo(-6.02, 1);

    await runtime.stop();
    expect(runtime.current()).toEqual({
      phase: "off",
      deviceLabel: null,
      sampleRate: null,
      levelDbfs: null,
      frameCount: 0,
      error: null,
    });
    expect(capture.stopCount).toBe(1);
    expect(phases).toEqual(["off", "starting", "listening", "listening", "stopping", "off"]);
  });

  it("moves to an actionable error state when permission is denied", async () => {
    const capture = new FakeCapture();
    capture.startError = Object.assign(new Error("denied"), { name: "NotAllowedError" });
    const runtime = new VoiceCaptureRuntime(capture);

    await runtime.start();

    expect(runtime.current().phase).toBe("error");
    expect(runtime.current().error).toContain("Microphone access was denied");
    expect(capture.stopCount).toBe(1);
  });

  it("ignores frames from a stopped capture generation", async () => {
    const capture = new FakeCapture();
    const runtime = new VoiceCaptureRuntime(capture);
    await runtime.start();
    const staleFrame = capture.onFrame;

    await runtime.stop();
    staleFrame?.(frame([1, 1]));

    expect(runtime.current().phase).toBe("off");
    expect(runtime.current().frameCount).toBe(0);
  });
});

describe("voice audio helpers", () => {
  it("uses a bounded decibel floor for silence", () => {
    expect(audioLevelDbfs(new Float32Array(16))).toBe(-120);
    expect(audioLevelDbfs(Float32Array.of(1, -1))).toBe(0);
  });

  it("preserves useful unknown errors", () => {
    expect(describeVoiceCaptureError(new Error("AudioWorklet failed"))).toBe("AudioWorklet failed");
  });
});
