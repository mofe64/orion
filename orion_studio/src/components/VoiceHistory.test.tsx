import { renderToStaticMarkup } from "react-dom/server";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadHistory, loadBehaviours, ConversationTimeline, BehaviourTimeline } from "./VoiceHistory";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => vi.resetAllMocks());

describe("conversation history failure mapping", () => {
  it.each([
    [null, null, "Studio couldn't load Orion's conversation history. Please try again."],
    ["session", null, "Studio couldn't open this conversation. Please try again."],
    [null, "cursor", "Studio couldn't load older conversations. Please try again."],
  ])("maps failures for session %s and cursor %s", async (conversationId, before, message) => {
    vi.mocked(invoke).mockRejectedValue("history IPC stack trace");
    await expect(loadHistory(conversationId, before)).rejects.toMatchObject({ message, detail: "history IPC stack trace" });
    expect(invoke).toHaveBeenCalledWith("load_voice_history", { conversationId, before });
  });
});


it("renders multiple exchanges together with delivery status and collapsed processing", () => {
  const html = renderToStaticMarkup(<ConversationTimeline turns={[
    {sessionId:"first",startedAt:"2026-10-12T11:00:00Z",updatedAt:"2026-10-12T11:00:01Z",command:"Look at the book",reply:"A blue book",error:null,delivery:"interrupted",events:[{at:"2026-10-12T11:00:00Z",event:{type:"wake.rejected"}}]},
    {sessionId:"second",startedAt:"2026-10-12T12:00:00Z",updatedAt:"2026-10-12T12:00:01Z",command:"Continue",reply:null,error:"Agent unavailable",delivery:"failed",events:[]},
  ]}/>);
  expect(html).toContain("Look at the book"); expect(html).toContain("Continue");
  expect(html).toContain("Reply interrupted"); expect(html).toContain("Agent unavailable");
  expect(html).not.toContain("0 steps"); expect(html).not.toContain("wake.rejected");
  expect(html).not.toContain("<details open");
});
it("shows observed targets and failed actions with actual duration", () => {
  const html = renderToStaticMarkup(<BehaviourTimeline events={[{id:"event",at:"2026-10-12T11:00:00Z",category:"attention",behaviour:"object_peek",phase:"failed",target:"book-1",targetCategory:"book",reason:"Motion timed out",durationSeconds:3.5,actionId:"action-1"}]}/>);
  expect(html).toContain("Object peek"); expect(html).toContain("failed · book · 3.5 s");
  expect(html).toContain("Motion timed out"); expect(html).not.toContain("completed");
});
it("forwards behaviour pagination and filters through the native bridge", async () => {
  vi.mocked(invoke).mockResolvedValue({items:[],nextCursor:null});
  await loadBehaviours("cursor","rest");
  expect(invoke).toHaveBeenCalledWith("load_behaviour_history",{before:"cursor",category:"rest"});
});
