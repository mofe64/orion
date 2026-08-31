import { describe, expect, it } from "vitest";

import { projectCatalog } from "./catalog";
import { quinticBlend, sampleSceneLight, sceneDuration } from "./preview";

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
});
