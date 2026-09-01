import { describe, expect, it, vi } from "vitest";

import {
  StudioVoicePipeline,
  type VoiceWorkerConnection,
  type VoiceWorkerLauncher,
  type VoiceWorkerTransport,
} from "./studioVoicePipeline";
import type { VoiceWorkerEvent, VoiceWorkerListener, VoiceWorkerReadyEvent } from "./voiceWorkerProtocol";
import type { VoiceAudioFrame, VoiceCaptureSource } from "./voiceRuntime";
import type { SpeechPlayer } from "./studioSpeaker";

class FakeSource implements VoiceCaptureSource {
  onFrame: ((frame: VoiceAudioFrame) => void) | null = null;

  async start(onFrame: (frame: VoiceAudioFrame) => void) {
    this.onFrame = onFrame;
    return { deviceLabel: "Mac microphone", sampleRate: 48_000, channels: 1 as const };
  }

  async stop() {}

  emit(samples = new Float32Array(960).fill(0.2)) {
    this.onFrame?.({ samples, sampleRate: 48_000, channels: 1, sequence: 0, capturedAtMs: 0 });
  }
}

class FakeLauncher implements VoiceWorkerLauncher {
  stopCount = 0;
  async start(): Promise<VoiceWorkerConnection> {
    return { url: "ws://127.0.0.1:8765", token: "secret", asrModel: "qwen" };
  }
  async stop() { this.stopCount += 1; }
}

const READY: VoiceWorkerReadyEvent = {
  type: "ready",
  protocol: 4,
  asr: { provider: "qwen3-asr", model: "Qwen/Qwen3-ASR-0.6B" },
  wake: { provider: "rustpotter", model: "hey_orion_reference.rpw", threshold: 0.4 },
  agent: { provider: "codex", model: "configured-default" },
  tts: { provider: "chatterbox-turbo", model: "mlx-community/chatterbox-turbo-8bit" },
};

class FakeTransport implements VoiceWorkerTransport {
  listeners = new Set<VoiceWorkerListener>();
  audio: Int16Array[] = [];
  playback: { requestId: number; error?: string }[] = [];
  async connect() { return READY; }
  subscribe(listener: VoiceWorkerListener) { this.listeners.add(listener); return () => this.listeners.delete(listener); }
  sendAudio(pcm: Int16Array) { this.audio.push(pcm); }
  finishPlayback(requestId: number, error?: string) { this.playback.push({ requestId, error }); }
  close() {}
  emit(event: VoiceWorkerEvent) { for (const listener of this.listeners) listener(event); }
}

class FakeSpeaker implements SpeechPlayer {
  played: { pcm: Int16Array; sampleRate: number }[] = [];
  stopCount = 0;
  async play(pcm: Int16Array, sampleRate: number) { this.played.push({ pcm, sampleRate }); }
  async stop() { this.stopCount += 1; }
}

describe("StudioVoicePipeline", () => {
  it("streams continuously and publishes the confirmed command", async () => {
    const source = new FakeSource();
    const launcher = new FakeLauncher();
    const transport = new FakeTransport();
    const speaker = new FakeSpeaker();
    const pipeline = new StudioVoicePipeline({
      source, launcher, speaker, createTransport: () => transport,
    });

    await pipeline.start();
    expect(pipeline.current()).toMatchObject({
      phase: "ready",
      asrProvider: "qwen3-asr",
      wakeProvider: "rustpotter",
      wakeThreshold: 0.4,
      agentProvider: "codex",
      ttsProvider: "chatterbox-turbo",
    });

    for (let index = 0; index < 4; index += 1) source.emit();
    expect(transport.audio).toHaveLength(1);
    expect(transport.audio[0]).toHaveLength(1_280);

    transport.emit({ type: "wake.candidate", name: "hey_orion", score: 0.62 });
    expect(pipeline.current().phase).toBe("wake_candidate");
    transport.emit({ type: "transcription.started", purpose: "wake_and_command" });
    expect(pipeline.current().phase).toBe("confirming_wake");
    transport.emit({ type: "wake.confirmed", text: "Hey Orion, turn left", hasCommand: true });
    transport.emit({ type: "transcript.final", text: "turn left", rawText: "Hey Orion, turn left", language: "English", durationMs: 180 });
    expect(pipeline.current()).toMatchObject({ phase: "thinking", transcript: "turn left" });
    transport.emit({ type: "agent.started", requestId: 1 });
    transport.emit({ type: "agent.response", requestId: 1, text: "Turning left.", durationMs: 400 });
    transport.emit({ type: "synthesis.started", requestId: 1 });
    expect(pipeline.current()).toMatchObject({ phase: "synthesizing", response: "Turning left." });
    transport.emit({
      type: "speech.audio",
      requestId: 1,
      sampleRate: 24_000,
      samples: 2,
      durationMs: 1,
      synthesisMs: 300,
      pcm: Int16Array.of(100, -100),
    });
    expect(pipeline.current().phase).toBe("speaking");
    await vi.waitFor(() => expect(transport.playback).toEqual([{ requestId: 1, error: undefined }]));
    expect(speaker.played[0]).toMatchObject({ sampleRate: 24_000 });
    transport.emit({ type: "speech.completed", requestId: 1 });
    expect(pipeline.current().phase).toBe("ready");
  });

  it("returns to listening after ASR rejects a wake candidate", async () => {
    const transport = new FakeTransport();
    const pipeline = new StudioVoicePipeline({
      source: new FakeSource(),
      launcher: new FakeLauncher(),
      createTransport: () => transport,
    });
    await pipeline.start();
    transport.emit({ type: "wake.candidate", name: "hey_orion", score: 0.45 });
    transport.emit({ type: "transcription.started", purpose: "wake_and_command" });
    transport.emit({ type: "wake.rejected", text: "we should turn left" });
    expect(pipeline.current().phase).toBe("ready");
  });

  it("stops both microphone and persistent worker", async () => {
    const launcher = new FakeLauncher();
    const pipeline = new StudioVoicePipeline({
      source: new FakeSource(),
      launcher,
      createTransport: () => new FakeTransport(),
    });
    await pipeline.start();
    await pipeline.stop();
    expect(pipeline.current().phase).toBe("off");
    expect(launcher.stopCount).toBe(1);
  });

  it("closes capture and worker after a fatal worker error", async () => {
    const launcher = new FakeLauncher();
    const transport = new FakeTransport();
    const pipeline = new StudioVoicePipeline({
      source: new FakeSource(), launcher, createTransport: () => transport,
    });
    await pipeline.start();
    transport.emit({
      type: "worker.error",
      code: "model_failed",
      message: "ASR model failed",
      recoverable: false,
    });
    expect(pipeline.current()).toMatchObject({ phase: "error", error: "ASR model failed" });
    await vi.waitFor(() => expect(launcher.stopCount).toBe(1));
  });
});
