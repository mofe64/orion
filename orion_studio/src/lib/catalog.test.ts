import { describe, expect, it } from "vitest";

import { JOINT_NAMES, LIGHTING_EFFECTS, MOTION_STYLES } from "../types";
import { catalogs, isOwnerPose, ownerAssetDescription, projectCatalog } from "./catalog";

describe("v2 project catalog", () => {
  it("keeps acknowledgement scenes and motion timing identical apart from direction", () => {
    const mirror = (value: unknown) => JSON.parse(JSON.stringify(value).replaceAll("left", "right"));
    expect(mirror(projectCatalog.scenes.acknowledge_left)).toEqual(projectCatalog.scenes.acknowledge_right);
    expect(mirror(projectCatalog.motions.look_at_left_expressive)).toEqual(projectCatalog.motions.look_at_right_expressive);
    expect(projectCatalog.scenes.agreement.motion).toEqual([
      { id: "agreement-motion-0", at: 0, play: "acknowledge_nod" },
    ]);
    const markers = projectCatalog.motions.acknowledge_nod.keyframes.map(frame => frame.marker);
    for (const event of [...projectCatalog.scenes.agreement.lighting, ...projectCatalog.scenes.agreement.audio]) {
      expect(markers).toContain(event.on_marker);
    }
  });
  it("loads the complete commissioned character vocabulary", () => {
    const names = Object.keys(projectCatalog.motions);
    expect(names.filter((name) => name.startsWith("idle_"))).toHaveLength(8);
    expect(names.filter((name) => name.startsWith("speak_"))).toHaveLength(4);
    expect(projectCatalog.scenes.acknowledge_left.format_version).toBe(2);
    expect(projectCatalog.cues).toContain("acknowledge_warm");
  });

  it("keeps every pose inside the tracked calibration copy", () => {
    const limits = Object.fromEntries(projectCatalog.jointLimits.map((limit) => [limit.name, limit]));
    for (const pose of Object.values(projectCatalog.poses)) {
      expect(Object.keys(pose.positions).sort()).toEqual([...JOINT_NAMES].sort());
      for (const joint of JOINT_NAMES) {
        expect(pose.positions[joint]).toBeGreaterThanOrEqual(limits[joint].lower_rad);
        expect(pose.positions[joint]).toBeLessThanOrEqual(limits[joint].upper_rad);
      }
    }
    expect(projectCatalog.poses.rest.tags).toContain("shutdown_only");
    expect(projectCatalog.poses.rest.idle_profile).toBeUndefined();
  });

  it("contains only v2 styles, effects, and valid semantic references", () => {
    for (const motion of Object.values(projectCatalog.motions)) {
      expect(MOTION_STYLES).toContain(motion.style);
      expect(motion.keyframes.at(-1)?.arrival).toBe("settle");
      if (motion.space === "anchor_relative") {
        expect(motion.return_to_anchor).toBe(true);
        expect(Object.values(motion.keyframes.at(-1)?.offsets ?? {}).every((value) => value === 0)).toBe(true);
      }
    }
    for (const scene of Object.values(projectCatalog.scenes)) {
      for (const clip of scene.motion) expect(projectCatalog.motions[clip.play]).toBeDefined();
      for (const event of scene.lighting) expect(LIGHTING_EFFECTS).toContain(event.effect);
      for (const event of scene.audio) expect(projectCatalog.cues).toContain(event.cue);
      expect(scene.finish).toEqual({ anchor: "final_pose", lighting: "pose_default" });
    }
  });
});

describe("owner library presentation", () => {
  it.each(["v1", "v2"] as const)("hides internal poses without removing %s movement dependencies", hardware => {
    const catalog = catalogs[hardware];
    const names = Object.values(catalog.poses).filter(isOwnerPose).map(pose => pose.name);
    expect(names).toContain("home");
    expect(names).toContain("look_left");
    for (const name of ["zero_reference", "rest", "look_left_anticipation", "look_left_lean", "look_left_overshoot"]) {
      expect(names).not.toContain(name);
      expect(catalog.poses[name]).toBeDefined();
    }
    expect(isOwnerPose({ ...catalog.poses.home, name: "my_lean", source: "user", tags: [] })).toBe(true);
    expect(isOwnerPose({ ...catalog.poses.home, owner_scene: "my_scene" })).toBe(false);
  });
  it("keeps engineering records intact and only supplies approved built-in copy", () => {
    const home = catalogs.v2.poses.home;
    expect(home.description).toContain("calibration zero");
    expect(ownerAssetDescription("pose", home)).toBe("A compact, forward-facing pose.");
    expect(ownerAssetDescription("pose", { ...home, name: "unknown_reference" })).toBe("");
    expect(ownerAssetDescription("pose", { ...home, source: "user", description: "My reading pose." })).toBe("My reading pose.");
    expect(ownerAssetDescription("scene", catalogs.v2.scenes.acknowledge_left)).toBe("Turn left with a warm light and a short sound.");
  });
});
