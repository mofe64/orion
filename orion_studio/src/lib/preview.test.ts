import { describe, expect, it } from "vitest";

import { projectCatalog } from "./catalog";
import {
  expandSceneTimeline,
  quinticBlend,
  sampleSceneLight,
  sceneDuration,
  splitSceneEvent,
} from "./preview";

describe("Studio preview semantics", () => {
  it("uses a quintic blend with zero endpoints", () => {
    expect(quinticBlend(0)).toBe(0);
    expect(quinticBlend(1)).toBe(1);
    expect(quinticBlend(0.5)).toBeCloseTo(0.5);
  });

  it("derives duration from semantic scene events", () => {
    expect(sceneDuration(projectCatalog.scenes.acknowledge_left, projectCatalog)).toBeCloseTo(5.0);
  });

  it("previews RGBW transitions without changing the scene schema", () => {
    const scene = projectCatalog.scenes.lighting_acknowledge;
    expect(sampleSceneLight(scene, 0)).toEqual({ red: 0, green: 0, blue: 0, white: 0 });
    expect(sampleSceneLight(scene, 0.35)).toEqual({ red: 8, green: 3, blue: 0, white: 20 });
    expect(sampleSceneLight(scene, 2)).toEqual({ red: 0, green: 0, blue: 0, white: 28 });
  });

  it("splits motions into timed poses while retaining holds as gaps", () => {
    const parts = splitSceneEvent({
      id: "motion",
      at: 0,
      type: "play_motion",
      motion: "look_at_left_expressive",
    }, projectCatalog);

    expect(parts.map((part) => part.type)).toEqual(Array(8).fill("goto_pose"));
    expect(parts.map((part) => part.at)).toEqual([0, 1.2, 1.4, 2.2, 2.35, 3.35, 3.5, 4.2]);
    expect(parts.map((part) => part.type === "goto_pose" ? part.duration_seconds : 0))
      .toEqual([1.2, 0.2, 0.8, 0.15, 1, 0.15, 0.7, 0.8]);
    const last = parts.at(-1)!;
    expect(last.at + (last.type === "goto_pose" ? last.duration_seconds : 0)).toBeCloseTo(5);
  });

  it("flattens Studio scene clips before runtime submission", () => {
    const nested = projectCatalog.scenes.acknowledge_left;
    const parent = {
      format_version: 1 as const,
      name: "parent",
      description: "Composite scene",
      source: "draft" as const,
      timeline: [{ id: "nested", at: 2, type: "scene" as const, scene: nested.name }],
    };

    const expanded = expandSceneTimeline(parent, projectCatalog);
    expect(expanded).toHaveLength(nested.timeline.length);
    expect(expanded[0].at).toBe(nested.timeline[0].at + 2);
    expect(expanded.every((event) => event.type !== "scene")).toBe(true);
  });
});
