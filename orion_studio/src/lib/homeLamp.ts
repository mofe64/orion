import type { LightPreview } from "../types";

export type LampMood = "Warm white" | "Custom color";
export interface LampSetting { mood: LampMood; brightness: number; hue: number }
export interface ManualLampSetting extends LampSetting { enabled: boolean }

/** One hue control, with fixed saturation/lightness; never exposes RGB to owners. */
export function lampChannels({ mood, brightness, hue, enabled }: ManualLampSetting): [number, number, number, number] {
  if (!enabled) return [0, 0, 0, 0];
  const amount = Math.max(1, Math.min(100, brightness)) / 100;
  if (mood === "Warm white") return [0, 0, 0, Math.round(255 * amount)];
  const turn = ((hue % 360) + 360) % 360 / 30;
  const channel = (offset: number) => {
    const k = (offset + turn) % 12;
    const value = .65 - .75 * .35 * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    return Math.round(255 * value * amount);
  };
  return [channel(0), channel(8), channel(4), 0];
}

export function lampPreview(setting: ManualLampSetting): LightPreview {
  const [red, green, blue, white] = lampChannels(setting);
  return { red, green, blue, white };
}

export function hueName(hue: number): string {
  return ["Red", "Amber", "Green", "Aqua", "Blue", "Violet", "Red"][Math.round(hue / 60)] ?? "Blue";
}
