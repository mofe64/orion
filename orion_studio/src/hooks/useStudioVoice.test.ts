import { invoke, isTauri } from "@tauri-apps/api/core";
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadVoiceSettings, saveVoiceSettings, useStudioVoice, VOICE_FAILURE_MESSAGES } from "./useStudioVoice";
import { DEFAULT_VOICE_SETTINGS } from "../lib/studioVoicePipeline";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn(() => true) }));
afterEach(() => vi.resetAllMocks());

describe("voice settings failure mapping", () => {
  it("keeps browser voice settings unknown and directs listening to the desktop app", () => {
    vi.mocked(isTauri).mockReturnValue(false);
    function BrowserVoice() {
      const voice = useStudioVoice({ url: "http://test", token: "test" });
      expect(voice.loaded).toBe(false);
      expect(voice.loadError).toBeNull();
      return createElement("p", null, voice.label);
    }
    expect(renderToStaticMarkup(createElement(BrowserVoice))).toContain("Open Orion Studio’s desktop app to enable listening");
  });
  it("maps load failures without exposing native text in the message", async () => {
    vi.mocked(invoke).mockRejectedValue("native service stack trace");
    await expect(loadVoiceSettings()).rejects.toMatchObject({ message: VOICE_FAILURE_MESSAGES.load, detail: "native service stack trace" });
    expect(VOICE_FAILURE_MESSAGES.load).toContain("Please try again.");
    expect(VOICE_FAILURE_MESSAGES.load).not.toContain("Reconnect");
  });
  it("maps save failures and preserves the complete settings payload", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("native save failed"));
    await expect(saveVoiceSettings(DEFAULT_VOICE_SETTINGS)).rejects.toMatchObject({ message: VOICE_FAILURE_MESSAGES.save, detail: "native save failed" });
    expect(invoke).toHaveBeenCalledWith("save_voice_settings", { settings: DEFAULT_VOICE_SETTINGS });
  });
});
