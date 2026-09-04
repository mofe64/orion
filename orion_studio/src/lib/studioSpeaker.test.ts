import { afterEach, describe, expect, it, vi } from "vitest";

const gateway = vi.hoisted(() => ({
  uploadSpeech: vi.fn(),
  getSpeechRun: vi.fn(),
  cancelRun: vi.fn(),
}));

vi.mock("./gateway", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./gateway")>()),
  ...gateway,
}));

import { OrionSpeechPlayer, type OrionPlaybackSnapshot } from "./studioSpeaker";

describe("OrionSpeechPlayer", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.clearAllMocks();
  });

  it("resolves only after the Pi reports speech completion", async () => {
    gateway.uploadSpeech.mockResolvedValue({
      api_version: 2,
      accepted: true,
      studio_voice_request_id: "request-1",
      run_id: 42,
      state: "queued",
    });
    gateway.getSpeechRun
      .mockResolvedValueOnce({ api_version: 2, run_id: 42, state: "playing" })
      .mockResolvedValueOnce({ api_version: 2, run_id: 42, state: "completed" });
    vi.spyOn(globalThis, "setTimeout").mockImplementation(((callback: () => void) => {
      callback();
      return 1;
    }) as typeof globalThis.setTimeout);
    const snapshots: OrionPlaybackSnapshot[] = [];
    const connection = { url: "http://orion.local:8090", token: "secret" };
    const player = new OrionSpeechPlayer(() => connection, (status) => snapshots.push(status));

    await player.play(Int16Array.of(100, -100), 24_000);

    expect(gateway.uploadSpeech).toHaveBeenCalledOnce();
    expect(gateway.getSpeechRun).toHaveBeenCalledTimes(2);
    expect(snapshots.map((snapshot) => snapshot.state)).toEqual([
      "uploading",
      "queued",
      "playing",
      "completed",
    ]);
  });

  it("cancels a run accepted after stop was pressed during upload", async () => {
    let accept: (value: unknown) => void = () => undefined;
    gateway.uploadSpeech.mockReturnValue(new Promise((resolve) => { accept = resolve; }));
    gateway.cancelRun.mockResolvedValue({ ok: true });
    const connection = { url: "http://orion.local:7447", token: "secret" };
    const player = new OrionSpeechPlayer(() => connection);
    const playback = player.play(Int16Array.of(100, -100), 24_000);
    await player.stop();
    accept({ run_id: 12, state: "queued" });
    await expect(playback).rejects.toThrow("cancelled during upload");
    expect(gateway.cancelRun).toHaveBeenCalledWith(connection, "speech", 12);
    expect(gateway.getSpeechRun).not.toHaveBeenCalled();
  });

  it("cancels the matching Pi run when Studio playback is stopped", async () => {
    gateway.cancelRun.mockResolvedValue({ ok: true });
    gateway.uploadSpeech.mockResolvedValue({
      api_version: 2,
      accepted: true,
      studio_voice_request_id: "request-2",
      run_id: 7,
      state: "playing",
    });
    let pollScheduled = false;
    let releasePoll: () => void = () => undefined;
    vi.spyOn(globalThis, "setTimeout").mockImplementation(((callback: () => void) => {
      pollScheduled = true;
      releasePoll = callback;
      return 1;
    }) as typeof globalThis.setTimeout);
    const connection = { url: "http://orion.local:8090", token: "secret" };
    const player = new OrionSpeechPlayer(() => connection);
    const playback = player.play(Int16Array.of(100, -100), 24_000);
    await vi.waitFor(() => expect(gateway.uploadSpeech).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(pollScheduled).toBe(true));

    await player.stop();
    releasePoll();

    await expect(playback).rejects.toThrow("cancelled");
    expect(gateway.cancelRun).toHaveBeenCalledWith(connection, "speech", 7);
  });
});
