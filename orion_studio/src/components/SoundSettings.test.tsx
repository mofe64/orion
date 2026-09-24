import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { SoundSettings } from "./SoundSettings";
import type { GatewayStatus } from "../types";

const routines = {
  sounds: { alarm: "club_alarm", timer: "funny_alarm" },
  available_sounds: [{ id: "two_tone", name: "Two-tone (default)" }, { id: "club_alarm", name: "Club alarm" }, { id: "funny_alarm", name: "Funny alarm" }],
} as GatewayStatus["routines"];

describe("Orion sound settings", () => {
  it("shows both saved alert sounds", () => {
    const html = renderToStaticMarkup(<SoundSettings connected routines={routines} onSound={vi.fn()} />);
    expect(html).toContain('value="club_alarm" selected=""');
    expect(html).toContain('value="funny_alarm" selected=""');
    expect(html).not.toContain('disabled=""');
    expect(html).toContain("Alert sounds");
  });
  it("does not offer unconfirmed alarm choices on an older runtime", () => {
    const html = renderToStaticMarkup(<SoundSettings connected routines={{ mode: "idle" } as GatewayStatus["routines"]} onSound={vi.fn()} />);
    expect(html.match(/disabled=""/g)).toHaveLength(2);
    expect(html).toContain("Update Orion");
    expect(html).not.toContain('value="club_alarm"');
  });
  it("blocks writes while disconnected", () => {
    const html = renderToStaticMarkup(<SoundSettings connected={false} routines={routines} onSound={vi.fn()} />);
    expect(html.match(/disabled=""/g)).toHaveLength(2);
  });
});
