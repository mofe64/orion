import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { CameraCard, drawCameraDetections } from "./CameraCard";

describe("camera controls and overlay", () => {
  it("explains privacy and disables unavailable snapshots", () => {
    const html = renderToStaticMarkup(<CameraCard connection={null} />);
    expect(html).toContain("Connect Orion to view its camera");
    expect(html).toContain("Turning the camera off stops capture for everyone.");
    expect(html).toMatch(/disabled=""[^>]*>Save snapshot/);
    expect(html).not.toContain("<img");
  });
  it("offers camera-off before a live frame arrives", () => {
    const html = renderToStaticMarkup(<CameraCard connection={{ url: "http://test", token: "token" }} />);
    expect(html).toContain("Turn camera off");
    expect(html).toContain('width="640" height="360"');
  });
  it("scales normalised boxes and ignores invalid coordinates", () => {
    const context = { clearRect: vi.fn(), strokeRect: vi.fn(), fillText: vi.fn() } as unknown as CanvasRenderingContext2D;
    drawCameraDetections(context, [{ track_id: 4, score: .95, chosen_target: true, capture_boottime_ns: "1", box: [.25, .5, .1, .2] },
      { track_id: 5, score: .95, chosen_target: false, capture_boottime_ns: "1", box: [NaN, 0, 1, 1] }], 640, 360);
    expect(context.clearRect).toHaveBeenCalledWith(0, 0, 640, 360);
    expect(context.strokeRect).toHaveBeenCalledOnce();
    expect(context.strokeRect).toHaveBeenCalledWith(160, 180, 64, 72);
    expect(context.fillText).toHaveBeenCalledWith("Face 4 · target", 164, 175);
  });
  it("highlights only the chosen target and labels every track", () => {
    const strokes: [string | CanvasGradient | CanvasPattern, number][] = [];
    const context = { clearRect: vi.fn(), fillText: vi.fn(), strokeRect: () => strokes.push([context.strokeStyle, context.lineWidth]) } as unknown as CanvasRenderingContext2D;
    drawCameraDetections(context, [true, false].map((chosen_target, index) => ({ track_id: index + 1,
      score: .95, chosen_target, capture_boottime_ns: "1", box: [.1, .2, .3, .4] as [number, number, number, number] })), 640, 360);
    expect(strokes).toEqual([["#ffe299", 3], ["#78e6cf", 2]]);
    expect(context.fillText).toHaveBeenCalledWith("Face 2", 68, 67);
  });
});
