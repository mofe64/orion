import { afterEach, describe, expect, it, vi } from "vitest";
import { TrackingPreference } from "./trackingPreference";
import type { TrackingSetting } from "./gateway";

const connection = { url: "http://orion", token: "secret" };
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };
afterEach(() => vi.useRealTimers());
describe("saved tracking preference", () => {
  it("loads and saves Idle with its conversation behaviour explained", async () => {
    vi.useFakeTimers();
    const save = vi.fn().mockResolvedValue({ tracking_mode: "idle", camera_enabled: true });
    const model = new TrackingPreference(connection, {
      read: vi.fn().mockResolvedValue({ tracking_mode: "faces", camera_enabled: true }), save,
    });
    model.start(); await flush(); await model.choose("idle");
    expect(save.mock.calls[0][1]).toBe("idle");
    expect(model.snapshot()).toMatchObject({ mode: "idle", pending: false,
      notice: "Autonomous behaviours and conversation gaze are on.", error: null });
    model.stop();
    // A saved Idle response is also a valid read, not a corrupt preference.
    const restored = new TrackingPreference(connection, { read: save, save });
    restored.start(); await flush();
    expect(restored.snapshot().mode).toBe("idle");
    restored.stop();
  });
  it("loads the saved mode, optimistically changes one mode, and reverts on failure", async () => {
    vi.useFakeTimers();
    let reject!: (error: Error) => void;
    const save = vi.fn(() => new Promise<TrackingSetting>((_, failed) => { reject = failed; }));
    const model = new TrackingPreference(connection, { read: vi.fn().mockResolvedValue({ tracking_mode: "faces", camera_enabled: true }), save });
    model.start(); await flush();
    expect(model.snapshot().mode).toBe("faces");
    const saving = model.choose("off");
    expect(model.snapshot()).toMatchObject({ mode: "off", pending: true });
    await model.choose("faces"); expect(save).toHaveBeenCalledTimes(1);
    reject(new Error("disk unavailable")); await saving;
    expect(model.snapshot()).toMatchObject({ mode: "faces", pending: false, error: { detail: "disk unavailable" } });
    expect(model.snapshot().notice).toBe("");
    await vi.advanceTimersByTimeAsync(2000);
    expect(model.snapshot().error?.detail).toBe("disk unavailable"); model.stop();
  });
  it("ignores a read begun before saving and only announces after acknowledgement", async () => {
    vi.useFakeTimers();
    let resolveRead!: (value: TrackingSetting) => void;
    let resolveSave!: (value: TrackingSetting) => void;
    const read = vi.fn().mockResolvedValueOnce({ tracking_mode: "faces", camera_enabled: true })
      .mockImplementation(() => new Promise<TrackingSetting>(resolve => { resolveRead = resolve; }));
    const save = vi.fn(() => new Promise<TrackingSetting>(resolve => { resolveSave = resolve; }));
    const model = new TrackingPreference(connection, { read, save });
    model.start(); await flush(); await vi.advanceTimersByTimeAsync(2000);
    const saving = model.choose("off");
    expect(model.snapshot().notice).toBe("");
    resolveSave({ tracking_mode: "off", camera_enabled: true }); await saving;
    resolveRead({ tracking_mode: "faces", camera_enabled: true }); await flush();
    expect(model.snapshot()).toMatchObject({ mode: "off", pending: false, notice: "Tracking is off." }); model.stop();
  });
  it("camera-off prevents saves, and a disconnected request cannot change state", async () => {
    vi.useFakeTimers();
    const save = vi.fn().mockResolvedValue({ tracking_mode: "off", camera_enabled: true });
    let resolve!: (value: TrackingSetting) => void;
    const read = vi.fn().mockResolvedValueOnce({ tracking_mode: "faces", camera_enabled: false })
      .mockImplementation(() => new Promise<TrackingSetting>(done => { resolve = done; }));
    const model = new TrackingPreference(connection, { read, save });
    model.start(); await flush(); await model.choose("off"); expect(save).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2000); model.stop();
    resolve({ tracking_mode: "off", camera_enabled: true }); await flush();
    expect(model.snapshot()).toMatchObject({ mode: "faces", cameraEnabled: false });
  });
  it("a tracking acknowledgement cannot overwrite a later camera privacy change", async () => {
    vi.useFakeTimers();
    let resolve!: (value: TrackingSetting) => void;
    const model = new TrackingPreference(connection, {
      read: vi.fn().mockResolvedValue({ tracking_mode: "faces", camera_enabled: true }),
      save: vi.fn(() => new Promise<TrackingSetting>(done => { resolve = done; })),
    });
    model.start(); await flush();
    const saving = model.choose("off"); model.cameraChanged(false);
    resolve({ tracking_mode: "off", camera_enabled: true }); await saving;
    expect(model.snapshot()).toMatchObject({ mode: "off", cameraEnabled: false, pending: false }); model.stop();
  });
});
