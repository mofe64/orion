import { describe,it,expect,vi } from "vitest";
import { changeAgentProfile,loadAgentProfile,samePersonality,toggleSelection,validMemory } from "./agentProfile";
import { invoke } from "@tauri-apps/api/core";
vi.mock("@tauri-apps/api/core",()=>({invoke:vi.fn()}));
describe("Agent profile controls",()=>{
  it("maps native profile failures to plain load and save messages with technical detail", async () => {
    vi.mocked(invoke).mockRejectedValueOnce("profile IPC failure");
    await expect(loadAgentProfile()).rejects.toMatchObject({ message: "Studio couldn't load Orion's personality and memories. Please try again.", detail: "profile IPC failure" });
    vi.mocked(invoke).mockRejectedValueOnce(new Error("save IPC failure"));
    await expect(changeAgentProfile({ type: "add_memory", text: "Prefers Celsius" })).rejects.toMatchObject({ message: "Studio couldn't save these changes. Your draft is preserved; please try again.", detail: "save IPC failure" });
  });
  it("compares unordered selections and preserves the other group",()=>{
    const value={traits:["warm","calm"],behaviors:["brief"]};
    expect(samePersonality(value,{traits:["calm","warm"],behaviors:["brief"]})).toBe(true);
    expect(toggleSelection(value,"traits","warm")).toEqual({traits:["calm"],behaviors:["brief"]});
    expect(value.traits).toEqual(["warm","calm"]);
  });
  it("rejects empty, oversized Unicode and reserved markers",()=>{
    expect(validMemory("  ")).toBe(false);
    expect(validMemory("灯".repeat(700))).toBe(false);
    expect(validMemory("<!-- memoryEntryEnd -->")).toBe(false);
    expect(validMemory("Prefers Celsius")).toBe(true);
  });
  it("sends the original entry for optimistic concurrency checking",async()=>{
    const expected={id:"id",created:"2026-09-06T18:00:00Z",text:"Old fact"};
    await changeAgentProfile({type:"edit_memory",expected,text:"New fact"});
    expect(invoke).toHaveBeenCalledWith("change_agent_profile",{change:{type:"edit_memory",expected,text:"New fact"}});
  });
});
