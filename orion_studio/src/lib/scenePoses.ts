import type { MotionDefinition, PoseDefinition, ProjectCatalog, SceneDefinition } from "../types";
export function sceneCatalog(catalog: ProjectCatalog, scene: SceneDefinition): ProjectCatalog {
  return { ...catalog, poses: { ...catalog.poses, ...scene.custom_poses }, motions: { ...catalog.motions, ...scene.custom_motions } };
}
export function attachScenePose(scene: SceneDefinition, catalog: ProjectCatalog, eventId: string, index: number, pose: PoseDefinition): SceneDefinition {
  const event = scene.motion.find(item => item.id === eventId);
  if (!event) return scene;
  const original = scene.custom_motions?.[event.play] ?? catalog.motions[event.play];
  if (!original?.keyframes[index]) return scene;
  let n = 1;
  while (scene.custom_poses?.[`${scene.name}_custom_pose_${n}`]) n++;
  const name = `${scene.name}_custom_pose_${n}`;
  const value = { ...pose, name, source: "draft" as const, owner_scene: scene.name, remote_revision: undefined };
  const motionName = original.owner_scene === scene.name ? original.name : `${scene.name}_custom_movement_${Object.keys(scene.custom_motions ?? {}).length+1}`;
  const motion: MotionDefinition = { ...original, name: motionName, source: "draft", owner_scene: scene.name, keyframes: original.keyframes.map((frame,i) => i === index ? { ...frame, pose: name, offsets: undefined } : frame) };
  return { ...scene, custom_poses: { ...scene.custom_poses, [name]: value }, custom_motions: { ...scene.custom_motions, [motionName]: motion }, motion: scene.motion.map(item => item.id === eventId ? { ...item, play: motionName, show_parts: true } : item) };
}
export function renameScene(scene: SceneDefinition, name: string): SceneDefinition {
  const names = new Map(Object.keys(scene.custom_poses ?? {}).map(old => [old, name + old.slice(scene.name.length)]));
  const movements = new Map(Object.keys(scene.custom_motions ?? {}).map(old => [old, name + old.slice(scene.name.length)]));
  return { ...scene, name, ...(name !== scene.name ? { source: "draft" as const, remote_revision: undefined } : {}),
    custom_poses: Object.fromEntries(Object.entries(scene.custom_poses ?? {}).map(([old,pose]) => { const next = names.get(old)!; return [next,{ ...pose, name: next, owner_scene: name }]; })),
    custom_motions: Object.fromEntries(Object.entries(scene.custom_motions ?? {}).map(([old,motion]) => { const next = movements.get(old)!; return [next,{ ...motion, name: next, owner_scene: name, keyframes: motion.keyframes.map(frame => ({ ...frame, pose: frame.pose ? names.get(frame.pose) ?? frame.pose : undefined })) }]; })),
    motion: scene.motion.map(event => ({ ...event, play: movements.get(event.play) ?? event.play })),
    starting_pose: scene.starting_pose ? names.get(scene.starting_pose) ?? scene.starting_pose : undefined,
  };
}

export function previewPoses(catalog: ProjectCatalog) {
  return { format_version: 2, units: "radians", poses: Object.fromEntries(Object.entries(catalog.poses).map(([name,pose]) => [name,{ description: pose.description, tags: pose.tags, idle_profile: pose.idle_profile, default_lighting: pose.default_lighting, positions: pose.positions }])) };
}

/** Freeze a relative movement's compiled poses for scene-local joint editing. */
export function absoluteSceneMovement(scene: SceneDefinition, catalog: ProjectCatalog, eventId: string, trajectory: import("../types").CompiledTrajectoryPreview): SceneDefinition {
  const event = scene.motion.find(item => item.id === eventId)!;
  const original = catalog.motions[event.play];
  const poses = { ...scene.custom_poses };
  let n = 1;
  const keyframes = original.keyframes.map((frame,index) => {
    while (poses[`${scene.name}_custom_pose_${n}`]) n++;
    const name = `${scene.name}_custom_pose_${n++}`;
    const sample = trajectory.samples.find(sample => sample.keyframe_index === index+1) ?? trajectory.samples.at(-1)!;
    poses[name] = { name, description: "Scene-specific pose.", tags: ["powered"], source: "draft", owner_scene: scene.name, positions: Object.fromEntries(trajectory.joint_names.map((joint,i) => [joint,sample.positions[i]])) as PoseDefinition["positions"] };
    return { ...frame, offsets: undefined, pose: name };
  });
  const name = `${scene.name}_custom_movement_${Object.keys(scene.custom_motions ?? {}).length+1}`;
  const motion: MotionDefinition = { ...original, name, source: "draft", owner_scene: scene.name, space: "absolute", return_to_anchor: false, keyframes };
  return { ...scene, custom_poses: poses, custom_motions: { ...scene.custom_motions, [name]: motion }, motion: scene.motion.map(item => item.id === eventId ? { ...item, play: name, show_parts: true } : item) };
}
