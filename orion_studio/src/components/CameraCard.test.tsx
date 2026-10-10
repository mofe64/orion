import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { CameraCard, drawCameraObservations } from "./CameraCard";

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
    drawCameraObservations(context, [{ id: 4, class: "face", box: [.25, .5, .1, .2] },
      { id: 5, class: "invalid", box: [NaN, 0, 1, 1] }], 640, 360);
    expect(context.clearRect).toHaveBeenCalledWith(0, 0, 640, 360);
    expect(context.strokeRect).toHaveBeenCalledOnce();
    expect(context.strokeRect).toHaveBeenCalledWith(160, 180, 64, 72);
    expect(context.fillText).toHaveBeenCalledWith("face · 4", 164, 175);
  });
});
