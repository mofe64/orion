import { beforeEach, describe, expect, it, vi } from "vitest";
import { projectCatalog } from "./catalog";
import { readDraft, saveDraft, discardDraft } from "./drafts";

beforeEach(() => {
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value), removeItem: (key: string) => values.delete(key) });
});
describe("workspace drafts", () => {
  it("preserves independent edits for all asset types across selection and reload", () => {
    const scene = Object.values(projectCatalog.scenes)[0];
    const motion = Object.values(projectCatalog.motions)[0];
    const pose = Object.values(projectCatalog.poses)[0];
    saveDraft("scene", { ...scene, description: "Scene edit" });
    saveDraft("motion", { ...motion, description: "Motion edit" });
    saveDraft("pose", { ...pose, description: "Pose edit" });
    expect(readDraft("scene", scene).description).toBe("Scene edit");
    expect(readDraft("motion", motion).description).toBe("Motion edit");
    expect(readDraft("pose", pose).description).toBe("Pose edit");
    discardDraft("scene", scene.name);
    expect(readDraft("scene", scene)).toEqual(scene);
    expect(readDraft("pose", pose).description).toBe("Pose edit");
  });
  it("falls back to the catalog for corrupted storage", () => {
    const scene = Object.values(projectCatalog.scenes)[0];
    localStorage.setItem(`orion-studio:draft:v1:scene:${scene.name}`, "{");
    expect(readDraft("scene", scene)).toEqual(scene);
  });
  it("reports storage write failure to the caller instead of claiming a saved draft", () => {
    vi.stubGlobal("localStorage", { setItem: () => { throw new Error("quota"); } });
    expect(() => saveDraft("pose", Object.values(projectCatalog.poses)[0])).toThrow("quota");
  });
  it("rejects malformed nested drafts before the editor renders them", () => {
    const scene = Object.values(projectCatalog.scenes)[0];
    const motion = Object.values(projectCatalog.motions)[0];
    const pose = Object.values(projectCatalog.poses)[0];
    for (const [kind, original, patch] of [["scene", scene, { motion: [null] }], ["motion", motion, { keyframes: [null] }], ["pose", pose, { positions: {} }]] as const) {
      localStorage.setItem(`orion-studio:draft:v1:${kind}:${original.name}`, JSON.stringify({ ...original, ...patch }));
      expect(readDraft(kind, original)).toEqual(original);
    }
  });
});
