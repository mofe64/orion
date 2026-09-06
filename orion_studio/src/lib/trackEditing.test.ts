import { describe, expect, it } from "vitest";
import { projectCatalog } from "./catalog";
import { sequenceMedia, moveTrackEvent, validateMediaStarts } from "./trackEditing";
import { buildSceneDocument } from "./sceneDocument";

describe("single-owner media tracks", () => {
  it("queues competing items, preserves movement cues, includes delays and is stable on repeat", () => {
    const scene = { ...structuredClone(projectCatalog.scenes.acknowledge_left), lighting: [{ id: "a", at: 0, effect: "constant" as const, duration: 2 }, { id: "b", on_marker: "notice", effect: "pulse" as const, duration: 1, delay: .5 }], audio: [{ id: "c", at: 0, cue: "agree_soft", duration: 1 }, { id: "d", at: 0, cue: "agree_soft", duration: 2 }] };
    const trajectories = { [scene.motion[0].id]: { markers: [{ name: "notice", time_seconds: .3 }] } } as never;
    const result = sequenceMedia(scene,trajectories);
    expect(result.lighting[1].resolved_at).toBe(2.5);
    expect(result.lighting[1].on_marker).toBe("notice");
    expect(result.audio[1].resolved_at).toBe(1);
    expect(sequenceMedia(result,trajectories)).toBe(result);
    const stored = buildSceneDocument(result);
    expect(stored.scene.lighting[1].at).toBe(2.5);
    expect(stored.scene.audio[1]).not.toHaveProperty("duration");
    expect(stored.scene.lighting[1]).not.toHaveProperty("delay");
  });
  it("does not reuse old cue timing after its movement is removed", () => {
    const scene = { ...projectCatalog.scenes.return_home, motion: [], audio: [], lighting: [{ id: "a", effect: "constant" as const, on_marker: "removed", resolved_at: 2 }] };
    const result = sequenceMedia(scene,{});
    expect(result.lighting[0].resolved_at).toBeUndefined();
    expect(() => validateMediaStarts(result,{})).toThrow("no longer available");
  });
  it("keeps a light transition inside its reserved time", () => {
    const scene = { ...projectCatalog.scenes.return_home, audio: [], lighting: [{ id: "a", at: 0, effect: "settle_glow" as const, duration: 2, transition: 1 }, { id: "b", at: 0, effect: "off" as const, duration: 1 }] };
    expect(sequenceMedia(scene,{}).lighting[1].resolved_at).toBe(3);
  });
  it("reorders dragged sounds and prevents overlap", () => {
    const scene = { ...projectCatalog.scenes.return_home, audio: [{ id: "a", at: 0, cue: "a", duration: 2 }, { id: "b", at: 3, cue: "b", duration: 1 }] };
    const result = moveTrackEvent(scene,{ track: "audio", id: "b" }, .5, {});
    expect(result.audio[1].resolved_at).toBe(2);
    const first = moveTrackEvent(scene,{ track: "audio", id: "b" },0,{});
    expect(first.audio[0].id).toBe("b");
    expect(first.audio[1].resolved_at).toBe(1);
  });
});
