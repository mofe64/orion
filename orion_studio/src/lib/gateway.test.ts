import { afterEach, describe, expect, it, vi } from "vitest";

import { compileMotionPreview, uploadSpeech, setAlertSound } from "./gateway";

afterEach(() => vi.unstubAllGlobals());

describe("gateway v2 client", () => {
  it("saves one alert sound without sending stale mode, alerts or another sound preference", async () => {
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ ok: true }) });
    vi.stubGlobal("fetch", fetch);
    await setAlertSound({ url: "http://orion.local:7447", token: "secret" }, "timer", "funny_alarm");
    const [url, init] = fetch.mock.calls[0];
    expect(url).toBe("http://orion.local:7447/api/v2/operations");
    expect(init.headers.Authorization).toBe("Bearer secret");
    expect(JSON.parse(init.body)).toEqual({ operation: "routines", request: { action: "set_sound", kind: "timer", sound: "funny_alarm" } });
  });
  it("sends an unsaved v2 motion document for Rust compilation", async () => {
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ format_version: 2 }) });
    vi.stubGlobal("fetch", fetch);
    const document = { format_version: 2, motion: { name: "draft" } };
    await compileMotionPreview({ url: "http://orion.local:7447/", token: "secret" }, document, "home", "home");
    const [url, init] = fetch.mock.calls[0];
    expect(url).toBe("http://orion.local:7447/api/v2/trajectory");
    expect(JSON.parse(init.body)).toEqual({ document, start_pose: "home", anchor_pose: "home" });
    expect(init.headers.Authorization).toBe("Bearer secret");
  });

  it("uploads WAV bytes with the Studio request identity", async () => {
    const fetch = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ run_id: 4 }) });
    vi.stubGlobal("fetch", fetch);
    await uploadSpeech({ url: "http://orion", token: "secret" }, new Uint8Array([1, 2, 3]), "voice-1");
    const [, init] = fetch.mock.calls[0];
    expect(init.headers["X-Orion-Voice-Request-ID"]).toBe("voice-1");
    expect(new Uint8Array(init.body)).toEqual(new Uint8Array([1, 2, 3]));
  });
});
