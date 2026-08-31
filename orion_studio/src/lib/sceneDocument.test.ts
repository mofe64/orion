import { describe, expect, it } from "vitest";

import { projectCatalog } from "./catalog";
import { buildSceneDocument } from "./sceneDocument";

describe("runtime scene documents", () => {
  it("compiles a final return-to-rest scene clip for Orion hardware", () => {
    const scene = {
      format_version: 1 as const,
      name: "custom_scene",
      description: "Custom scene ending at rest",
      source: "draft" as const,
      timeline: [
        { id: "opening", at: 0, type: "goto_pose" as const, pose: "home", duration_seconds: 2 },
        { id: "return", at: 2, type: "scene" as const, scene: "return_to_rest" },
      ],
    };

    const document = buildSceneDocument(scene, scene.name, projectCatalog);
    expect(document.scene.timeline).toHaveLength(3);
    expect(document.scene.timeline[1]).toEqual({
      at: 2,
      type: "goto_pose",
      pose: "rest",
      duration_seconds: 3,
    });
    expect(document.scene.timeline[2]).toEqual({
      at: 2,
      type: "light",
      red: 0,
      green: 0,
      blue: 0,
      white: 0,
      transition_seconds: 0.5,
    });
  });
});
