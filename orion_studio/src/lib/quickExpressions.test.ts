import { describe,expect,it } from "vitest";
import { quickExpressions } from "./quickExpressions";
import type { ProjectCatalog,SceneDefinition } from "../types";
const scene = (name: string,source: SceneDefinition["source"]) => ({name,source}) as SceneDefinition;
const catalog = (scenes: SceneDefinition[]) => ({scenes:Object.fromEntries(scenes.map(scene => [scene.name,scene]))}) as ProjectCatalog;
describe("Home expressions", () => {
  it("includes published owner scenes with built-ins, while excluding drafts and administration", () => {
    const picks = quickExpressions(catalog([scene("greeting","built_in"),scene("owner_wave","user"),scene("unfinished","draft"),scene("return_home","built_in"),scene("deployment_safe","built_in")]));
    expect(picks.map(scene => scene.name)).toEqual(["greeting","owner_wave"]);
  });
  it("reserves room for owner scenes and caps a large catalog at six", () => {
    const picks = quickExpressions(catalog([...Array.from({length:9},(_,i) => scene(`builtin_${i}`,"built_in")),...Array.from({length:9},(_,i) => scene(`owner_${i}`,"user"))]));
    expect(picks).toHaveLength(6);
    expect(picks.filter(scene => scene.source === "built_in")).toHaveLength(3);
    expect(picks.filter(scene => scene.source === "user")).toHaveLength(3);
  });
  it("uses spare space for built-ins and preserves raw scene IDs", () => {
    const scenes = Array.from({length:8},(_,i) => scene(`raw_scene_${i}`,"built_in"));
    expect(quickExpressions(catalog(scenes)).map(scene => scene.name)).toEqual(scenes.slice(0,6).map(scene => scene.name));
    expect(quickExpressions(catalog(scenes),0)).toEqual([]);
  });
});
