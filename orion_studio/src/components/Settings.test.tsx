import { describe, expect, it } from "vitest";
import { canSaveVoiceSettings } from "./Settings";
import type { StudioVoice } from "../hooks/useStudioVoice";
import { DEFAULT_VOICE_SETTINGS } from "../lib/studioVoicePipeline";

const voice = { settings: DEFAULT_VOICE_SETTINGS, models: [], loaded: true, saving: false,
  snapshot: { muted: false } } as unknown as StudioVoice;

describe("voice settings save availability", () => {
  it("allows a speech change while listening even before the model list loads", () => {
    const draft = { ...DEFAULT_VOICE_SETTINGS, ttsModel: "pocket-int8" };
    expect(canSaveVoiceSettings(voice, draft, "codex", true)).toBe(true);
  });
  it("requires a change and a connected Pi", () => {
    expect(canSaveVoiceSettings(voice, DEFAULT_VOICE_SETTINGS, "codex", true)).toBe(false);
    expect(canSaveVoiceSettings(voice, { ...DEFAULT_VOICE_SETTINGS, ttsModel: "pocket-int8" }, "codex", false)).toBe(false);
  });
});
