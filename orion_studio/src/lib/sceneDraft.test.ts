import { describe, expect, it } from "vitest";

import { projectCatalog } from "./catalog";
import { materializeSceneDraft, usedDraftPoseNames } from "./sceneDraft";

describe("scene-local pose drafts", () => {
  it("names used temporary poses from the final scene name in timeline order", () => {
    const first = { ...structuredClone(projectCatalog.poses.zero_reference), name: "draft-a", source: "draft" as const, draftLabel: "zero reference" };
    const second = { ...structuredClone(projectCatalog.poses.rest), name: "draft-b", source: "draft" as const };
    const scene = {
      format_version: 1 as const,
      name: "working_scene",
      description: "Draft",
      source: "draft" as const,
      draftPoses: { "draft-a": first, "draft-b": second, unused: first },
      timeline: [
        { id: "one", at: 0, type: "goto_pose" as const, pose: "draft-b", duration_seconds: 1 },
        { id: "two", at: 1, type: "goto_pose" as const, pose: "draft-a", duration_seconds: 1 },
        { id: "three", at: 2, type: "goto_pose" as const, pose: "draft-b", duration_seconds: 1 },
      ],
    };

    expect(usedDraftPoseNames(scene)).toEqual(["draft-b", "draft-a"]);
    const materialized = materializeSceneDraft(scene, "desk_greeting", projectCatalog);
    expect(materialized.poses.map((pose) => pose.name)).toEqual([
      "desk_greeting_01", "desk_greeting_02",
    ]);
    expect(materialized.poses.every((pose) => pose.draftLabel === undefined)).toBe(true);
    expect(materialized.scene.timeline.map((event) => event.type === "goto_pose" ? event.pose : ""))
      .toEqual(["desk_greeting_01", "desk_greeting_02", "desk_greeting_01"]);
    expect(materialized.scene.draftPoses).toBeUndefined();
  });

  it("skips names that already exist in the pose library", () => {
    const draft = { ...structuredClone(projectCatalog.poses.home), name: "draft", source: "draft" as const };
    const scene = {
      format_version: 1 as const,
      name: "working",
      description: "Draft",
      source: "draft" as const,
      draftPoses: { draft },
      timeline: [{ id: "one", at: 0, type: "goto_pose" as const, pose: "draft", duration_seconds: 1 }],
    };
    const catalog = {
      ...projectCatalog,
      poses: {
        ...projectCatalog.poses,
        "desk_greeting_01": { ...draft, name: "desk_greeting_01", source: "user" as const },
      },
    };

    expect(materializeSceneDraft(scene, "desk_greeting", catalog).poses[0].name)
      .toBe("desk_greeting_02");
  });
});
