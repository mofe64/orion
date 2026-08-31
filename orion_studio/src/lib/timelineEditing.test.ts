import { describe, expect, it } from "vitest";

import { projectCatalog } from "./catalog";
import { duplicateTimelineEvents } from "./timelineEditing";

describe("timeline duplication", () => {
  it("appends a selected group after its end while retaining relative timing", () => {
    const scene = {
      format_version: 1 as const,
      name: "group",
      description: "Group duplication",
      source: "draft" as const,
      timeline: [
        { id: "first", at: 2, type: "goto_pose" as const, pose: "home", duration_seconds: 1 },
        { id: "second", at: 4, type: "goto_pose" as const, pose: "rest", duration_seconds: 2 },
      ],
    };
    let sequence = 0;

    const result = duplicateTimelineEvents(
      scene,
      ["first", "second"],
      projectCatalog,
      () => `id-${++sequence}`,
    );

    expect(result.scene.timeline.slice(2).map((event) => event.at)).toEqual([6, 8]);
    expect(result.scene.timeline.slice(2).map((event) => event.type === "goto_pose" ? event.pose : ""))
      .toEqual(["home", "rest"]);
    expect(result.startsAt).toBe(6);
  });

  it("copies an edited pose independently while retaining its joint values", () => {
    const edited = {
      ...structuredClone(projectCatalog.poses.zero_reference),
      name: "studio_draft_original",
      source: "draft" as const,
      draftLabel: "zero_reference",
      positions: {
        ...projectCatalog.poses.zero_reference.positions,
        base_yaw_joint: 0.25,
      },
    };
    const scene = {
      format_version: 1 as const,
      name: "edited",
      description: "Edited pose duplication",
      source: "draft" as const,
      draftPoses: { [edited.name]: edited },
      timeline: [{
        id: "edited-pose",
        at: 0,
        type: "goto_pose" as const,
        pose: edited.name,
        duration_seconds: 1,
      }],
    };
    let sequence = 0;

    const result = duplicateTimelineEvents(
      scene,
      ["edited-pose"],
      projectCatalog,
      () => `id-${++sequence}`,
    );
    const copiedEvent = result.scene.timeline[1];
    expect(copiedEvent.type).toBe("goto_pose");
    if (copiedEvent.type !== "goto_pose") throw new Error("Expected a copied pose event.");
    expect(copiedEvent.pose).not.toBe(edited.name);
    expect(result.scene.draftPoses?.[copiedEvent.pose].positions.base_yaw_joint).toBe(0.25);
    result.scene.draftPoses![copiedEvent.pose].positions.base_yaw_joint = 0.5;
    expect(result.scene.draftPoses?.[edited.name].positions.base_yaw_joint).toBe(0.25);
  });

  it("refuses to duplicate a selection spanning different tracks", () => {
    const scene = {
      format_version: 1 as const,
      name: "mixed",
      description: "Mixed selection",
      source: "draft" as const,
      timeline: [
        { id: "pose", at: 0, type: "goto_pose" as const, pose: "home", duration_seconds: 1 },
        { id: "light", at: 0, type: "light" as const, red: 0, green: 0, blue: 0, white: 1, transition_seconds: 1 },
      ],
    };

    expect(() => duplicateTimelineEvents(scene, ["pose", "light"], projectCatalog))
      .toThrow("one track");
  });
});
