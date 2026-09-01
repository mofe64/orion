import { describe, expect, it } from "vitest";

import {
  Pcm16FrameBatcher,
  StreamingLinearResampler,
  VoiceAudioConditioner,
  WORKER_FRAME_SAMPLES,
  float32ToPcm16,
} from "./voiceAudio";

describe("StreamingLinearResampler", () => {
  it("converts native 48 kHz chunks to continuous 16 kHz audio", () => {
    const resampler = new StreamingLinearResampler(48_000);
    const first = resampler.push(Float32Array.from({ length: 960 }, (_, index) => index));
    const second = resampler.push(Float32Array.from({ length: 960 }, (_, index) => 960 + index));

    expect(first).toHaveLength(320);
    expect(second).toHaveLength(320);
    expect([...first.slice(0, 4)]).toEqual([0, 3, 6, 9]);
    expect(second[0]).toBe(960);
  });

  it("maintains timing across awkward 44.1 kHz chunk boundaries", () => {
    const resampler = new StreamingLinearResampler(44_100);
    const chunks = Array.from({ length: 50 }, () => new Float32Array(882));
    const outputCount = chunks.reduce((count, chunk) => count + resampler.push(chunk).length, 0);

    expect(outputCount).toBe(16_000);
  });
});

describe("PCM conversion and buffering", () => {
  it("clips floats to signed little-endian-compatible PCM16 values", () => {
    expect([...float32ToPcm16(Float32Array.of(-2, -1, -0.5, 0, 0.5, 1, 2))]).toEqual([
      -32_768, -32_768, -16_384, 0, 16_384, 32_767, 32_767,
    ]);
  });

  it("batches 20 ms input into exact 80 ms worker frames", () => {
    const batcher = new Pcm16FrameBatcher();
    expect(batcher.push(new Int16Array(320))).toHaveLength(0);
    expect(batcher.push(new Int16Array(640))).toHaveLength(0);
    const frames = batcher.push(new Int16Array(640));
    expect(frames).toHaveLength(1);
    expect(frames[0]).toHaveLength(WORKER_FRAME_SAMPLES);
  });

  it("returns fixed worker frames after native-rate conversion", () => {
    const conditioner = new VoiceAudioConditioner();
    const result = conditioner.accept(new Float32Array(3_840).fill(0.25), 48_000);

    expect(result.pcm).toHaveLength(1_280);
    expect(result.workerFrames).toHaveLength(1);
  });
});
