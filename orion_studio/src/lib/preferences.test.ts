import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_PREFERENCES, loadPreferences, savePreferences } from "./preferences";
describe("persistent Studio preferences", () => {
  let value: string | null;
  beforeEach(() => { value = null; vi.stubGlobal("localStorage", { getItem: () => value, setItem: (_: string,next: string) => { value = next; } }); });
  it("survives a settings reload", () => { const preferences = { debugMode:true,reduceMotion:true,previewAudio:false }; savePreferences(preferences); expect(loadPreferences()).toEqual(preferences); });
  it("falls back safely for corrupt or invalid stored values", () => { value = "broken"; expect(loadPreferences()).toEqual(DEFAULT_PREFERENCES); value = '{"debugMode":"true","previewAudio":false}'; expect(loadPreferences()).toEqual({ ...DEFAULT_PREFERENCES,previewAudio:false }); });
});
