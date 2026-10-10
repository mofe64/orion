import { beforeEach, describe, expect, it, vi } from "vitest";
import { projectCatalog } from "./catalog";
import { attachScenePose } from "./scenePoses";
import { readDraft, saveDraft } from "./drafts";
import { publishDisabledReason, saveNamedScene, sceneName, sceneNameProblem, sceneSaveLabel } from "./sceneDraft";

const original = { ...projectCatalog.scenes.acknowledge_left, name: "my_scene", source: "draft" as const };
beforeEach(() => {
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value), removeItem: (key: string) => values.delete(key) });
});
describe("scene name commits", () => {
  it("keeps scene identity when the committed name is unchanged", () => {
    expect(saveNamedScene(original, original.name)).toBe(original);
  });
  it("normalizes names and keeps required, duplicate and built-in name checks", () => {
    expect(sceneName("  My new scene!  ")).toBe("my_new_scene");
    expect(sceneNameProblem(original, "  ", projectCatalog.scenes)).toBe("Give your scene a name first.");
    expect(sceneNameProblem(original, "Agreement", projectCatalog.scenes)).toContain("already in use");
    expect(sceneNameProblem(projectCatalog.scenes.agreement, "Agreement", projectCatalog.scenes)).toBe("Choose a name for your own scene.");
    expect(sceneNameProblem(original, "My scene", { ...projectCatalog.scenes, my_scene: original })).toBeNull();
  });
  it("saves the new name and its owned references before retiring the old draft", () => {
    const event = original.motion[0];
    const scene = attachScenePose(original, projectCatalog, event.id, 0, projectCatalog.poses.home);
    saveDraft("scene", scene);
    const saved = saveNamedScene(scene, "renamed_scene");
    expect(readDraft("scene", saved)).toEqual(saved);
    expect(localStorage.getItem("orion-studio:draft:v1:scene:my_scene")).toBeNull();
    expect(saved.motion[0].play).toMatch(/^renamed_scene_/);
    expect(Object.values(saved.custom_poses!)[0].owner_scene).toBe("renamed_scene");
  });
  it("preserves the previous draft if a rename cannot be stored", () => {
    saveDraft("scene", original);
    vi.spyOn(localStorage, "setItem").mockImplementation(() => { throw new Error("quota"); });
    expect(() => saveNamedScene(original, "new_name")).toThrow("quota");
    expect(readDraft("scene", original)).toEqual(original);
  });
});
describe("local save and publication feedback", () => {
  it("never claims a changed scene is saved using the previous write result", () => {
    expect(sceneSaveLabel(original, null)).toBe("Saving…");
    expect(sceneSaveLabel(original, { scene: original, failed: false })).toBe("Saved on this computer");
    expect(sceneSaveLabel({ ...original, description: "Changed" }, { scene: original, failed: false })).toBe("Saving…");
    expect(sceneSaveLabel(original, { scene: original, failed: true })).toBe("Couldn't save");
  });
  it("explains the existing disconnected and timing gates for publishing", () => {
    expect(publishDisabledReason(false, true)).toBe("Connect Orion to publish.");
    expect(publishDisabledReason(true, true)).toContain("calculating timing");
    expect(publishDisabledReason(true, false)).toBeNull();
  });
});
