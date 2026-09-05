import { describe, expect, it, vi } from "vitest";

import {
  StudioVoicePipeline,
  piVoiceUrl,
  type VoiceWorkerConnection,
  type VoiceWorkerLauncher,
  type VoiceWorkerTransport,
} from "./studioVoicePipeline";
import type { VoiceWorkerEvent, VoiceWorkerListener, VoiceWorkerReadyEvent } from "./voiceWorkerProtocol";
import type { SpeechPlayer } from "./studioSpeaker";

class FakeLauncher implements VoiceWorkerLauncher {
  stopCount = 0;
  async start(): Promise<VoiceWorkerConnection> {
    return { url: "ws://127.0.0.1:8765", token: "secret", asrModel: "qwen" };
  }
  async stop() { this.stopCount += 1; }
}

const READY: VoiceWorkerReadyEvent = {
  type: "ready",
  protocol: 7,
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
  it("observes background playback without uploading or acknowledging it", async () => {
    const transport = new FakeTransport();
    const speaker = { play: vi.fn(), append: vi.fn(), finish: vi.fn(), stop: vi.fn() };
    const pipeline = new StudioVoicePipeline({ launcher: new FakeLauncher(), createTransport: () => transport, speaker });
    await pipeline.start();
    transport.emit({ type: "synthesis.started", requestId: 1 });
    expect(pipeline.current().phase).toBe("synthesizing");
    transport.emit({ type: "speech.started", requestId: 1 });
    expect(pipeline.current().phase).toBe("speaking");
    transport.emit({ type: "microphone.status", muted: true });
    expect(pipeline.current().muted).toBe(true);
    transport.emit({ type: "speech.completed", requestId: 1 });
    expect(speaker.append).not.toHaveBeenCalled();
    expect(speaker.finish).not.toHaveBeenCalled();
    expect(transport.playback).toEqual([]);
  });

  it("streams chunks before synthesis ends and acknowledges only terminal playback", async () => {
    const transport = new FakeTransport();
    let complete: () => void = () => undefined;
    const speaker = { play: vi.fn(), stop: vi.fn(), append: vi.fn().mockResolvedValue(undefined), finish: vi.fn(() => new Promise<void>(resolve => { complete = resolve; })) };
    const pipeline = new StudioVoicePipeline({ launcher: new FakeLauncher(), createTransport: () => transport, speaker });
    await pipeline.start();
    transport.emit({ type: "wake.candidate", name: "hey_orion", score: .8 });
    transport.emit({ type: "speech.chunk", requestId: 1, sequence: 0, sampleRate: 24000, samples: 2, durationMs: 1, synthesisMs: 900, pcm: Int16Array.of(1, 2) });
    await vi.waitFor(() => expect(speaker.append).toHaveBeenCalledOnce());
    expect(transport.playback).toEqual([]);
    transport.emit({ type: "speech.chunk", requestId: 1, sequence: 1, sampleRate: 24000, samples: 2, durationMs: 1, synthesisMs: 1200, pcm: Int16Array.of(3, 4) });
    transport.emit({ type: "speech.end", requestId: 1, sequence: 2, synthesisMs: 1400 });
    await vi.waitFor(() => expect(speaker.finish).toHaveBeenCalledWith(2));
    expect(speaker.append.mock.calls.map(call => call[2])).toEqual([0, 1]);
    expect(transport.playback).toEqual([]);
    expect(pipeline.current().latency).toMatchObject({ synthesisFirstChunkMs: 900, synthesisTotalMs: 1400 });
    complete();
    await vi.waitFor(() => expect(transport.playback).toEqual([{ requestId: 1, error: undefined }]));
  });

  it("does not return to speaking when an in-flight chunk resolves after stop", async () => {
    const transport = new FakeTransport();
    let accepted: () => void = () => undefined;
    const speaker = { play: vi.fn(), stop: vi.fn().mockResolvedValue(undefined), append: vi.fn(() => new Promise<void>(resolve => { accepted = resolve; })) };
    const pipeline = new StudioVoicePipeline({ launcher: new FakeLauncher(), createTransport: () => transport, speaker });
    await pipeline.start();
    transport.emit({ type: "speech.chunk", requestId: 1, sequence: 0, sampleRate: 24000, samples: 2, durationMs: 1, synthesisMs: 900, pcm: Int16Array.of(1, 2) });
    await vi.waitFor(() => expect(speaker.append).toHaveBeenCalledOnce());
    await pipeline.stop();
    accepted();
    await Promise.resolve();
    await Promise.resolve();
    expect(pipeline.current().phase).toBe("off");
    expect(transport.playback).toEqual([]);
  });

  it.each([
    ["http://orion.local:7447", "ws://orion.local:7448/"],
    ["https://192.168.1.50:7447", "ws://192.168.1.50:7448/"],
    ["http://[::1]:7447", "ws://[::1]:7448/"],
    ["http://user:password@orion.local:7447/api?token=secret#status", "ws://orion.local:7448/"],
  ])("derives the plain Pi voice endpoint from %s", (gateway, expected) => {
    expect(piVoiceUrl(gateway)).toBe(expected);
  });

  it("receives Pi events without microphone capture and publishes the confirmed command", async () => {
    const launcher = new FakeLauncher();
    const transport = new FakeTransport();
    const speaker = new FakeSpeaker();
    const pipeline = new StudioVoicePipeline({
      launcher, speaker, createTransport: () => transport,
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

    expect(pipeline.current().deviceLabel).toContain("Pi capture");
    expect(transport.audio).toHaveLength(0);

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
      launcher: new FakeLauncher(),
      createTransport: () => transport,
    });
    await pipeline.start();
    transport.emit({ type: "wake.candidate", name: "hey_orion", score: 0.45 });
    transport.emit({ type: "transcription.started", purpose: "wake_and_command" });
    transport.emit({ type: "wake.rejected", text: "we should turn left" });
    expect(pipeline.current().phase).toBe("ready");
  });

  it("stops playback and the persistent worker", async () => {
    const launcher = new FakeLauncher();
    const pipeline = new StudioVoicePipeline({
      launcher,
      createTransport: () => new FakeTransport(),
    });
    await pipeline.start();
    await pipeline.stop();
    expect(pipeline.current().phase).toBe("off");
    expect(launcher.stopCount).toBe(1);
  });

  it("closes playback and worker after a fatal worker error", async () => {
    const launcher = new FakeLauncher();
    const transport = new FakeTransport();
    const pipeline = new StudioVoicePipeline({
      launcher, createTransport: () => transport,
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
