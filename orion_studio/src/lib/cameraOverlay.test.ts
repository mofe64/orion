import { afterEach, describe, expect, it, vi } from "vitest";
import { CameraOverlay } from "./cameraOverlay";
import type { CameraDetectionBatch } from "./gateway";

const stamp = "12345678901234567";
const batch: CameraDetectionBatch = { capture_boottime_ns: stamp, age_ms: 50,
  detections: [{ box: [.1, .2, .3, .4], score: .95, track_id: 4, chosen_target: true, capture_boottime_ns: stamp }] };
afterEach(() => vi.useRealTimers());

describe("frame-matched face boxes", () => {
  it("clears the canvas on its expiry timer even when the stream and polling stop", () => {
    vi.useFakeTimers(); vi.setSystemTime(0);
    const overlay = new CameraOverlay(), draw = vi.fn();
    overlay.frame(stamp, 0); overlay.update({ ...batch, age_ms: 150 }, 0);
    overlay.render(true, draw, Date.now);
    expect(draw).toHaveBeenLastCalledWith(batch.detections);
    vi.advanceTimersByTime(150);
    expect(draw).toHaveBeenLastCalledWith([]);
    expect(vi.getTimerCount()).toBe(0);
  });
  it("compares exact timestamps beyond JavaScript number precision", () => {
    const overlay = new CameraOverlay();
    overlay.frame(stamp, 100); overlay.update(batch, 100);
    expect(overlay.view(110).detections).toEqual(batch.detections);
    overlay.frame("12345678901234568", 110);
    expect(overlay.view(110).detections).toEqual([]);
  });
  it("counts capture age and network delay and expires without another poll", () => {
    const overlay = new CameraOverlay();
    overlay.frame(stamp, 100); overlay.update({ ...batch, age_ms: 150 }, 100);
    expect(overlay.view(249).detections).toEqual(batch.detections);
    expect(overlay.view(249).expiresIn).toBe(1);
    expect(overlay.view(300).detections).toEqual([]);
    overlay.update({ ...batch, age_ms: 50 }, 0);
    expect(overlay.view(300).detections).toEqual([]);
  });
  it("does not refresh old boxes when a timestamp repeats and clears on disconnect", () => {
    const overlay = new CameraOverlay();
    overlay.frame(stamp, 0); overlay.update({ ...batch, age_ms: 0 }, 0);
    overlay.frame(stamp, 299); overlay.update({ ...batch, age_ms: 0 }, 299);
    expect(overlay.view(301).detections).toEqual([]);
    overlay.clear(); expect(overlay.view(100).detections).toEqual([]);
  });
  it("hides missing, invalid, and individually mismatched timestamp evidence", () => {
    const overlay = new CameraOverlay();
    overlay.frame(null, 0); overlay.update(batch, 0);
    expect(overlay.view(0).detections).toEqual([]);
    overlay.frame(stamp, 0);
    overlay.update({ ...batch, age_ms: -1 }, 0);
    expect(overlay.view(0).detections).toEqual([]);
    overlay.update({ ...batch, detections: [{ ...batch.detections[0], capture_boottime_ns: "1" }] }, 0);
    expect(overlay.view(0).detections).toEqual([]);
  });
});
