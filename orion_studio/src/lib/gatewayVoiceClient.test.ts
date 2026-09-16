import { describe, expect, it, vi } from "vitest";
import { GatewayVoiceClient } from "./gatewayVoiceClient";
const ready = { eventId: 1, type: "ready", protocol: 7, asr: { provider: "qwen3-asr", model: "qwen" }, wake: { provider: "rustpotter", model: "hey_orion", threshold: .6 }, agent: { provider: "codex", model: "test" }, tts: { provider: "pocket-tts", model: "pocket-fp32" } };
describe("onboard observer", () => {
  it("authenticates, deduplicates snapshots, and accepts new coordinator generations", async () => {
    let call = 0;
    const fetcher = vi.fn(async () => new Response(JSON.stringify({ generation: ++call < 3 ? "first" : "second", events: [ready] })));
    const client = new GatewayVoiceClient("http://orion.local/api/v2/voice/events", "secret", fetcher, 1);
    const listener = vi.fn(); client.subscribe(listener);
    expect((await client.connect()).tts.provider).toBe("pocket-tts");
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
