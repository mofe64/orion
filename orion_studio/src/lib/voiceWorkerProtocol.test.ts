import { describe, expect, it } from "vitest";

import {
  VoiceWorkerClient,
  parseVoiceWorkerEvent,
} from "./voiceWorkerProtocol";

class FakeSocket {
  binaryType: BinaryType = "blob";
  readyState: number = WebSocket.CONNECTING;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  sent: (string | ArrayBuffer)[] = [];

  send(data: string | ArrayBuffer) { this.sent.push(data); }
  close() { this.readyState = WebSocket.CLOSED; }
  open() { this.readyState = WebSocket.OPEN; this.onopen?.(new Event("open")); }
  message(data: string | ArrayBuffer) { this.onmessage?.(new MessageEvent("message", { data })); }
}

const ready = JSON.stringify({
  type: "ready",
  protocol: 5,
  asr: { provider: "qwen3-asr", model: "Qwen/Qwen3-ASR-0.6B" },
  wake: { provider: "rustpotter", model: "hey_orion_reference.rpw", threshold: 0.4 },
  agent: { provider: "codex", model: "configured-default" },
  tts: { provider: "chatterbox-turbo", model: "mlx-community/chatterbox-turbo-8bit" },
});

describe("VoiceWorkerClient", () => {
  it("authenticates once and keeps binary audio on the same socket", async () => {
    const socket = new FakeSocket();
    const client = new VoiceWorkerClient({
      url: "ws://127.0.0.1:8765",
      token: "secret",
      createSocket: () => socket,
    });
    const connected = client.connect();
    socket.open();
    expect(JSON.parse(socket.sent[0] as string)).toMatchObject({
      type: "hello", protocol: 5, token: "secret", sampleRate: 16_000, frameSamples: 1_280,
    });
    socket.message(ready);
    await expect(connected).resolves.toMatchObject({ type: "ready" });

    expect(socket.sent).toHaveLength(1); // Studio sends no microphone frames.
  });

  it("pairs speech metadata with its binary PCM and acknowledges playback", async () => {
    const socket = new FakeSocket();
    const client = new VoiceWorkerClient({
      url: "ws://127.0.0.1:8765",
      token: "secret",
      createSocket: () => socket,
    });
    const events: unknown[] = [];
    client.subscribe((event) => events.push(event));
    const connected = client.connect();
    socket.open();
    socket.message(ready);
    await connected;

    socket.message(JSON.stringify({
      type: "speech.audio",
      requestId: 7,
      sampleRate: 24_000,
      samples: 2,
      durationMs: 0,
      synthesisMs: 250,
    }));
    socket.message(Int16Array.of(100, -100).buffer);
    expect(events.at(-1)).toMatchObject({
      type: "speech.audio",
      requestId: 7,
      pcm: Int16Array.of(100, -100),
    });

    client.finishPlayback(7);
    expect(JSON.parse(socket.sent.at(-1) as string)).toEqual({
      type: "playback.finished",
      requestId: 7,
    });
  });

  it("rejects binary speech that does not match its metadata", async () => {
    const socket = new FakeSocket();
    const client = new VoiceWorkerClient({
      url: "ws://127.0.0.1:8765",
      token: "secret",
      createSocket: () => socket,
    });
    const connected = client.connect();
    socket.open();
    socket.message(ready);
    await connected;
    socket.message(JSON.stringify({
      type: "speech.audio", requestId: 1, sampleRate: 24_000, samples: 2,
      durationMs: 1, synthesisMs: 1,
    }));
    socket.message(Int16Array.of(1).buffer);
    expect(client.currentPhase()).toBe("error");
  });

  it("rejects non-local worker URLs so microphone audio cannot be redirected", () => {
    expect(() => new VoiceWorkerClient({ url: "ws://example.com", token: "secret" })).toThrow("localhost");
  });
});

describe("parseVoiceWorkerEvent", () => {
  it("rejects unknown messages", () => {
    expect(() => parseVoiceWorkerEvent('{"type":"surprise"}')).toThrow("unsupported");
  });
});
