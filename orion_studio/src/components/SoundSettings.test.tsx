import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { SoundSettings } from "./SoundSettings";
import type { StudioVoice } from "../hooks/useStudioVoice";
import type { GatewayStatus } from "../types";

const voice = { settings: { ttsModel: "pocket-fp32", ttsVoice: "anna" }, loaded: true, saving: false } as StudioVoice;
const routines = {
  sounds: { alarm: "club_alarm", timer: "funny_alarm" },
  available_sounds: [{ id: "two_tone", name: "Two-tone (default)" }, { id: "club_alarm", name: "Club alarm" }, { id: "funny_alarm", name: "Funny alarm" }],
} as GatewayStatus["routines"];

describe("Orion sound settings", () => {
  it("shows saved Pi choices independently and leaves the Pocket voice available while listening", () => {
    const html = renderToStaticMarkup(<SoundSettings voice={voice} connected routines={routines} onSound={vi.fn()} />);
    expect(html).toContain('value="anna" selected=""');
    expect(html).toContain('value="club_alarm" selected=""');
    expect(html).toContain('value="funny_alarm" selected=""');
    expect(html).not.toContain('disabled=""');
    for (const preset of ["alba", "anna", "azelma", "cosette", "eve", "fantine", "jane", "vera"]) {
      expect(html).toContain(`value="${preset}"`);
    }
  });
  it("does not offer unconfirmed alarm choices on an older runtime", () => {
    const html = renderToStaticMarkup(<SoundSettings voice={voice} connected routines={{ mode: "idle" } as GatewayStatus["routines"]} onSound={vi.fn()} />);
    expect(html.match(/disabled=""/g)).toHaveLength(2);
    expect(html).toContain("Update Orion");
    expect(html).not.toContain('value="club_alarm"');
  });
  it("blocks writes while disconnected or before voice settings load", () => {
    const html = renderToStaticMarkup(<SoundSettings voice={voice} connected={false} routines={routines} onSound={vi.fn()} />);
    expect(html.match(/disabled=""/g)).toHaveLength(3);
    const loading = renderToStaticMarkup(<SoundSettings voice={{ ...voice, loaded: false }} connected routines={routines} onSound={vi.fn()} />);
    expect(loading).toMatch(/Default Pocket voice<select disabled=""/);
  });
  it("shows Piper Alba as a fixed voice without Pocket presets", () => {
    const piper = { ...voice, settings: { ttsModel: "piper-alba-medium", ttsVoice: "jane" } } as StudioVoice;
    const html = renderToStaticMarkup(<SoundSettings voice={piper} connected routines={routines} onSound={vi.fn()} />);
    expect(html).toContain("Piper Alba Medium");
    expect(html).not.toContain("Default Pocket voice");
    expect(html).not.toContain('value="jane"');
  });
});
