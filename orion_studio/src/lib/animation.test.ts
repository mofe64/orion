import { readFile } from "node:fs/promises";
import { afterEach, describe, expect, it, vi } from "vitest";
import { projectCatalog } from "./catalog";
import { createSceneDraft, movementOnly, prepareAnimation, samePose } from "./animation";
import { compileMotionPreview } from "./gateway";
vi.mock("./gateway", () => ({ compileMotionPreview: vi.fn() }));
describe("animation library", () => {
  afterEach(() => vi.unstubAllGlobals());
  it("copies a system scene without changing its original or exposing draft metadata to runtime", () => {
    const original = structuredClone(projectCatalog.scenes.acknowledge_left);
    const copy = createSceneDraft(projectCatalog, "scene", original.name, "123");
    copy.motion[0].at = 9;
    expect(projectCatalog.scenes.acknowledge_left).toEqual(original);
    expect(copy.source).toBe("draft");
    expect(copy.name).not.toBe(original.name);
  });
  it("seeds a new scene with the chosen pose", () => {
    const copy = createSceneDraft(projectCatalog, "pose", "look_left", "123");
    expect(copy.starting_pose).toBe("look_left");
    expect(copy.source).toBe("draft");
  });
  it("movement-only playback strips light and sound without mutating the scene", () => {
    const original = projectCatalog.scenes.return_home;
    const filtered = movementOnly(original);
    expect(filtered.motion).toEqual(original.motion);
    expect(filtered.lighting).toEqual([]);
    expect(filtered.audio).toEqual([]);
    expect(original.audio.length).toBeGreaterThan(0);
  });
  it("prepares the actual return-home scene from the selected held pose", async () => {
    vi.stubGlobal("fetch", async (url: string) => new Response(await readFile(url.replace("/@fs", ""))));
    vi.mocked(compileMotionPreview).mockResolvedValue({ duration_seconds: 1.7 } as never);
    const result = await prepareAnimation({} as never, projectCatalog.scenes.return_home, projectCatalog, "look_left");
    expect(compileMotionPreview).toHaveBeenCalledWith({}, "return_home", "look_left", undefined, undefined);
    expect(result.finalPose).toBe("home");
    expect(result.scene.audio[0].cue).toBe(projectCatalog.scenes.return_home.audio[0].cue);
    expect(result.scene.audio[0].duration).toBeGreaterThan(0);
  });
  it("does not confuse a directional pose with home", () => {
    expect(samePose(projectCatalog.poses.home.positions, projectCatalog.poses.home.positions)).toBe(true);
    expect(samePose(projectCatalog.poses.look_left.positions, projectCatalog.poses.home.positions)).toBe(false);
  });
});
