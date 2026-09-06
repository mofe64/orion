import { describe, expect, it } from "vitest";
import { projectCatalog } from "./catalog";
import { attachScenePose, renameScene, sceneCatalog } from "./scenePoses";
import { buildSceneDocument } from "./sceneDocument";

describe("scene-owned poses", () => {
  it("copies only the selected pose, embeds ownership and renames every reference", () => {
    const scene = { ...structuredClone(projectCatalog.scenes.acknowledge_left), name: "my_scene", source: "draft" as const };
    const event = scene.motion[0];
    const original = projectCatalog.motions[event.play];
    const pose = projectCatalog.poses[original.keyframes[0].pose!];
    const edited = attachScenePose(scene,projectCatalog,event.id,0,{ ...pose, positions: { ...pose.positions, head_pitch_joint: .1 } });
    expect(projectCatalog.poses[pose.name].positions.head_pitch_joint).toBe(pose.positions.head_pitch_joint);
    expect(edited.custom_poses?.my_scene_custom_pose_1.owner_scene).toBe("my_scene");
    const renamed = renameScene({ ...edited, source: "user", remote_revision: "old-revision" },"new_scene");
    expect(renamed.source).toBe("draft");
    expect(renamed.remote_revision).toBeUndefined();
    expect(renamed.custom_poses?.new_scene_custom_pose_1).toBeDefined();
    const catalog = sceneCatalog(projectCatalog,renamed);
    expect(catalog.motions[renamed.motion[0].play].keyframes[0].pose).toBe("new_scene_custom_pose_1");
    expect(buildSceneDocument(renamed).studio?.poses.new_scene_custom_pose_1).toBeDefined();
    expect(JSON.stringify(renamed)).not.toContain("my_scene_custom_");
  });
});
