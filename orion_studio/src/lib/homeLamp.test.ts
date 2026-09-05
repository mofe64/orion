import { describe, expect, it } from "vitest";
import { hueName, lampChannels, lampPreview } from "./homeLamp";

describe("Home lamp channel isolation", () => {
  it("uses the physical white channel for warm white, independent of the custom hue", () => {
    expect(lampChannels({ mood: "Warm white", brightness: 40, hue: 280, enabled: true })).toEqual([0, 0, 0, 102]);
    expect(lampChannels({ mood: "Warm white", brightness: 100, hue: 0, enabled: true })).toEqual([0, 0, 0, 255]);
  });
  it("turns every channel off in either mood", () => {
    for (const mood of ["Warm white", "Custom color"] as const) {
      expect(lampChannels({ mood, brightness: 100, hue: 120, enabled: false })).toEqual([0, 0, 0, 0]);
    }
  });
  it("converts the spectrum's primary hues and scales their brightness without adding white", () => {
    const setting = { mood: "Custom color" as const, brightness: 100, enabled: true };
    expect(lampChannels({ ...setting, hue: 0 })).toEqual([233, 99, 99, 0]);
    expect(lampChannels({ ...setting, hue: 120 })).toEqual([99, 233, 99, 0]);
    expect(lampChannels({ ...setting, hue: 240 })).toEqual([99, 99, 233, 0]);
    expect(lampChannels({ ...setting, hue: 360 })).toEqual(lampChannels({ ...setting, hue: 0 }));
    expect(lampChannels({ ...setting, hue: 0, brightness: 40 })).toEqual([93, 40, 40, 0]);
  });
  it("keeps the renderer's preview channels identical to the command", () => {
    expect(lampPreview({ mood: "Warm white", brightness: 40, hue: 210, enabled: true })).toEqual({ red: 0, green: 0, blue: 0, white: 102 });
    expect(hueName(240)).toBe("Blue");
  });
});
