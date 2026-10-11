import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { TrackingControl } from "./TrackingControl";
import type { TrackingMode } from "../lib/gateway";

const props = { connected: true, supported: true, mode: "faces" as TrackingMode, cameraEnabled: true,
  pending: false, notice: "", error: null, onChange: vi.fn() };
describe("Home tracking mode", () => {
  it.each(["off", "faces"] as const)("selects only the saved %s mode", mode => {
    const html = renderToStaticMarkup(<TrackingControl {...props} mode={mode} />);
    expect(html.match(/checked=""/g)).toHaveLength(1);
    expect(html.match(/<input[^>]*checked=""[^>]*>/g)?.[0]).toContain(`value="${mode}"`);
    expect(html).toContain(">Off</span>"); expect(html).toContain(">Faces</span>");
    expect(html).not.toContain('role="switch"');
    expect(html).not.toContain('disabled=""');
  });
  it.each([
    [{ cameraEnabled: false }, "Camera is off · mode saved"],
    [{ connected: false }, "Connect Orion to change tracking"],
    [{ supported: false }, "Camera unavailable on V1"],
    [{ mode: null }, "Checking tracking…"],
    [{ pending: true }, "Saving…"],
  ])("disables the control with an explanation: %s", (state, message) => {
    const html = renderToStaticMarkup(<TrackingControl {...props} {...state} />);
    expect(html).toContain(message); expect(html).toMatch(/fieldset[^>]*disabled=""/);
  });
  it("keeps notices and saving errors beside the control", () => {
    const html = renderToStaticMarkup(<TrackingControl {...props} mode="off" notice="Tracking is off."
      error={{ message: "Could not save", detail: "disk unavailable" }} />);
    expect(html).toContain('role="status"'); expect(html).toContain("Tracking is off.");
    expect(html).toContain("Could not save");
  });
});
