import { sceneCatalog, previewPoses, renameScene } from "./scenePoses";
import { sequenceMedia, withSoundDurations, validateMediaStarts } from "./trackEditing";
import type { JointPositions, ProjectCatalog, SceneDefinition } from "../types";
import { compileMotionPreview, type GatewayConnection } from "./gateway";
import { sequenceSceneMotions, validateSceneMotionSchedule, type SceneTrajectoryPreviews } from "./preview";

export const label = (name: string) => name.replaceAll("_", " ");
export function samePose(a: JointPositions, b: JointPositions): boolean {
  return Object.keys(b).every(key => Math.abs(a[key as keyof JointPositions] - b[key as keyof JointPositions]) < 0.015);
}
export function movementOnly(scene: SceneDefinition): SceneDefinition {
  return { ...scene, lighting: [], audio: [] };
}
export async function prepareAnimation(connection: GatewayConnection, scene: SceneDefinition, catalog: ProjectCatalog, start: string) {
  catalog = sceneCatalog(catalog, scene);
  const trajectories: SceneTrajectoryPreviews = {};
  let finalPose = start;
  for (const event of [...scene.motion].sort((a, b) => a.at - b.at)) {
    const motion = catalog.motions[event.play];
    if (!motion) throw new Error(`Movement ${label(event.play)} is unavailable.`);
    trajectories[event.id] = await compileMotionPreview(connection, motion.source === "draft" ? { format_version: 2, motion: { name: motion.name, description: motion.description, space: motion.space, style: motion.style, return_to_anchor: motion.return_to_anchor, keyframes: motion.keyframes } } : event.play, finalPose, motion.space === "anchor_relative" ? finalPose : undefined, Object.keys(scene.custom_poses ?? {}).length ? previewPoses(catalog) : undefined);
    if (motion.space === "absolute") finalPose = [...motion.keyframes].reverse().find(frame => frame.pose)?.pose ?? finalPose;
  }
  const resolved = sequenceMedia(sequenceSceneMotions(await withSoundDurations(scene, catalog), trajectories), trajectories);
  validateSceneMotionSchedule(resolved, trajectories);
  validateMediaStarts(resolved, trajectories);
  return { scene: resolved, trajectories, finalPose };
}
export function createSceneDraft(catalog: ProjectCatalog, kind: "scene" | "pose", name: string, id: string): SceneDefinition {
  const base: SceneDefinition = kind === "scene" ? catalog.scenes[name] : {
    format_version: 2, name: "", description: `Starting from ${label(name)}.`, source: "draft",
    motion: [], lighting: [], audio: [], finish: { anchor: "final_pose", lighting: "pose_default" },
  };
  let draftName = `my_${name}`;
  let suffix = 2;
  while (catalog.scenes[draftName]) draftName = `my_${name}_${suffix++}`;
  const owned = renameScene(structuredClone(base), draftName);
  return { ...owned, name: draftName, motion: owned.motion.map((event, index) => ({ ...event, id: `${id}-movement-${index}` })), lighting: base.lighting.map((event, index) => ({ ...event, id: `${id}-light-${index}` })), audio: base.audio.map((event, index) => ({ ...event, id: `${id}-sound-${index}` })), source: "draft", remote_revision: undefined, starting_pose: kind === "pose" ? name : "home" };
}
