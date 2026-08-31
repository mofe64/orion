import { describe, expect, it } from "vitest";

import { projectCatalog } from "./catalog";

describe("Orion Studio asset catalog", () => {
  it("bundles every STL mesh referenced by the Orion URDF", () => {
    const referencedMeshes = [...projectCatalog.urdf.matchAll(/filename=["']([^"']+\.stl)["']/g)]
      .map((match) => match[1].split("/").at(-1));

    expect(referencedMeshes.length).toBeGreaterThan(0);
    for (const mesh of referencedMeshes) {
      expect(mesh, `missing bundled URL for ${mesh}`).toBeDefined();
      expect(projectCatalog.meshUrls[mesh!], `missing bundled URL for ${mesh}`).toBeTruthy();
    }
  });

  it("resolves every motion and scene reference through named project assets", () => {
    for (const motion of Object.values(projectCatalog.motions)) {
      for (const keyframe of motion.keyframes) {
        expect(projectCatalog.poses[keyframe.pose], `${motion.name} -> ${keyframe.pose}`).toBeDefined();
      }
    }

    for (const scene of Object.values(projectCatalog.scenes)) {
      for (const event of scene.timeline) {
        if (event.type === "play_motion") {
          expect(projectCatalog.motions[event.motion], `${scene.name} -> ${event.motion}`).toBeDefined();
        } else if (event.type === "goto_pose") {
          expect(projectCatalog.poses[event.pose], `${scene.name} -> ${event.pose}`).toBeDefined();
        } else if (event.type === "audio") {
          expect(projectCatalog.cueUrls[event.cue], `${scene.name} -> ${event.cue}`).toBeTruthy();
        }
      }
    }
  });

  it("maps logical pose zero into the calibrated physical model reference", () => {
    expect(projectCatalog.urdfJointOffsets.base_yaw_joint).toBeCloseTo(-0.315320384242287);
    expect(projectCatalog.urdfJointOffsets.shoulder_pitch_joint).toBeCloseTo(-0.096485421696463);
    expect(projectCatalog.urdfJointOffsets.elbow_pitch_joint).toBeCloseTo(-0.331339850183298);
    expect(projectCatalog.urdfJointOffsets.head_roll_joint).toBeCloseTo(-0.712126211333258);
    expect(projectCatalog.urdfJointOffsets.head_pitch_joint).toBeCloseTo(-0.053689327575997);
  });
});
