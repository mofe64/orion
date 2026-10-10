import { invoke } from "@tauri-apps/api/core";
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadHistory } from "./VoiceHistory";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => vi.resetAllMocks());

describe("conversation history failure mapping", () => {
  it.each([
    [null, null, "Studio couldn't load Orion's conversation history. Please try again."],
    ["session", null, "Studio couldn't open this conversation. Please try again."],
    [null, "cursor", "Studio couldn't load older conversations. Please try again."],
  ])("maps failures for session %s and cursor %s", async (sessionId, before, message) => {
    vi.mocked(invoke).mockRejectedValue("history IPC stack trace");
    await expect(loadHistory(sessionId, before)).rejects.toMatchObject({ message, detail: "history IPC stack trace" });
    expect(invoke).toHaveBeenCalledWith("load_voice_history", { sessionId, before });
  });
});
