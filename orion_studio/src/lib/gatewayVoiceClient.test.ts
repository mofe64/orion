import { describe, expect, it, vi } from "vitest";
import { GatewayVoiceClient } from "./gatewayVoiceClient";
const ready = { eventId: 1, type: "ready", protocol: 7, asr: { provider: "qwen3-asr", model: "qwen" }, wake: { provider: "rustpotter", model: "hey_orion", threshold: .6 }, agent: { provider: "codex", model: "test" }, tts: { provider: "piper-tts", model: "piper-alba-medium" } };
describe("onboard observer", () => {
  it("preserves the browser receiver when using native fetch", async () => {
    const fetcher = vi.fn(function (this: unknown) {
      if (this !== globalThis) throw new TypeError("Can only call Window.fetch on instances of Window");
      return Promise.resolve(new Response(JSON.stringify({ generation: "first", events: [{ ...ready, muted: false }] })));
    });
    vi.stubGlobal("fetch", fetcher);
    const client = new GatewayVoiceClient("http://orion.local/api/v2/voice/events", "secret");
    try {
      expect((await client.connect()).muted).toBe(false);
      expect(fetcher).toHaveBeenCalledOnce();
    } finally {
      client.close();
      vi.unstubAllGlobals();
    }
  });
  it("authenticates, deduplicates snapshots, and accepts new coordinator generations", async () => {
    let call = 0;
    const fetcher = vi.fn(async () => new Response(JSON.stringify({ generation: ++call < 3 ? "first" : "second", events: [ready] })));
    const client = new GatewayVoiceClient("http://orion.local/api/v2/voice/events", "secret", fetcher, 1);
    const listener = vi.fn(); client.subscribe(listener);
    expect((await client.connect()).tts.provider).toBe("piper-tts");
    await vi.waitFor(() => expect(call).toBeGreaterThanOrEqual(3));
    client.close();
    expect(listener.mock.calls.filter(([event]) => event.type === "ready")).toHaveLength(2);
    expect(fetcher.mock.calls[0]).toEqual(["http://orion.local/api/v2/voice/events", expect.objectContaining({ headers: { Authorization: "Bearer secret" }, redirect: "error" })]);
  });
  it("closing a loading observer settles its pending connection", async () => {
    const client = new GatewayVoiceClient("http://orion.local/events", "secret", async () => new Response(JSON.stringify({ generation: null, events: [] })), 1);
    const loading = client.connect(); client.close();
    await expect(loading).rejects.toThrow("closed");
  });
  it("surfaces gateway failures without starting local inference", async () => {
    const client = new GatewayVoiceClient("http://orion.local/events", "secret", async () => new Response("unavailable", { status: 503 }));
    await expect(client.connect()).rejects.toThrow("503"); client.close();
  });
});
