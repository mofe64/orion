import { renderToStaticMarkup } from "react-dom/server";
import type { AgentProfile } from "../lib/agentProfile";
import type { Failure } from "../lib/feedback";
const state = vi.hoisted(() => ({ index: 0, profile: null as AgentProfile | null, error: null as Failure | null }));
vi.mock("react", async importOriginal => {
  const react = await importOriginal<typeof import("react")>();
  return { ...react, useState: (initial: unknown) => {
    const index = state.index++;
    if (index === 0) return react.useState(state.profile);
    if (index === 2) return react.useState(!state.error && !state.profile);
    if (index === 4) return react.useState(state.error);
    return react.useState(initial);
  } };
});
import { describe, expect, it, vi } from "vitest";
import { AgentProfileSettings, PROFILE_SAVING_NOTICE } from "./AgentProfileSettings";

describe("profile save feedback", () => {
  it("refers to Orion as it while explaining when the save applies", () => {
    expect(PROFILE_SAVING_NOTICE).toBe("Saving… If Orion is replying, this will apply when it finishes.");
  });
});

function markup(profile: AgentProfile | null, error: Failure | null = null) {
  state.index = 0; state.profile = profile; state.error = error;
  return renderToStaticMarkup(<AgentProfileSettings onboard desktop />);
}
describe("profile readiness", () => {
  it("shows the desktop prerequisite without an impossible browser retry", () => {
    state.index = 0; state.profile = null; state.error = { message: "Failed load", detail: "missing native bridge" };
    const html = renderToStaticMarkup(<AgentProfileSettings onboard desktop={false} />);
    expect(html).toContain("Open Orion Studio’s desktop app and connect Orion");
    expect(html).not.toContain("Try again");
    expect(html).not.toContain("Loading personality");
    expect(html).not.toContain("Failed load");
    expect(html).not.toContain('class="settings-card');
  });
  it("hides empty cards during loading", () => {
    const html = markup(null);
    expect(html).toContain("Loading personality and memories…");
    expect(html).not.toContain('class="settings-card');
  });
  it("keeps cards hidden after a load failure and offers retry with collapsed detail", () => {
    const html = markup(null, { message: "Studio couldn't load Orion's personality and memories. Please try again.", detail: "native profile error" });
    expect(html).toContain('role="alert"');
    expect(html).toContain("Try again");
    expect(html).toContain("native profile error");
    expect(html).not.toContain('class="settings-card');
    expect(html).not.toContain("<details open");
  });
  it("shows loaded cards and preserves memory privacy copy after a save failure", () => {
    const profile: AgentProfile = { soul: { revision: "test", personality: { traits: [], behaviors: [] } }, traits: [], behaviors: [], preview: "", memories: [], personalityEnabled: true, memoryEnabled: true };
    const html = markup(profile, { message: "Studio couldn't save these changes.", detail: "save error" });
    expect(html).toContain("<h3>Personality</h3>");
    expect(html).toContain("<h3>Memories</h3>");
    expect(html).toContain("Stored on Orion. Relevant memories are sent to Codex when Orion uses them. Editing or deleting starts a fresh conversation; it does not erase information already sent to Codex.");
    expect(html).not.toContain("Stored on this computer.");
    expect(html).toContain("Choices save automatically.");
    expect(html).not.toContain("Save personality");
    expect(html).not.toContain("Save memory");
  });
});
