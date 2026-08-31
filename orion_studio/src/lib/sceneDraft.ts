import type { PoseDefinition, ProjectCatalog, SceneDefinition } from "../types";

export interface MaterializedSceneDraft {
  scene: SceneDefinition;
  poses: PoseDefinition[];
}

export function usedDraftPoseNames(scene: SceneDefinition): string[] {
  const draftPoses = scene.draftPoses ?? {};
  const names: string[] = [];
  const seen = new Set<string>();
  for (const event of scene.timeline) {
    if (event.type !== "goto_pose" || !draftPoses[event.pose] || seen.has(event.pose)) continue;
    seen.add(event.pose);
    names.push(event.pose);
  }
  return names;
}

export function materializeSceneDraft(
  scene: SceneDefinition,
  targetSceneName: string,
  catalog: ProjectCatalog,
): MaterializedSceneDraft {
  const draftPoses = scene.draftPoses ?? {};
  const reserved = new Set(Object.keys(catalog.poses));
  const assignments: Record<string, string> = {};
  const poses: PoseDefinition[] = [];
  let sequence = 1;

  for (const draftName of usedDraftPoseNames(scene)) {
    let savedName = "";
    do {
      savedName = `${targetSceneName}_${String(sequence).padStart(2, "0")}`;
      sequence += 1;
    } while (reserved.has(savedName));
    reserved.add(savedName);
    assignments[draftName] = savedName;
    const draft = draftPoses[draftName];
    poses.push({
      name: savedName,
      description: draft.description,
      positions: structuredClone(draft.positions),
      source: "user",
      remote_revision: undefined,
    });
  }

  return {
    poses,
    scene: {
      ...structuredClone(scene),
      name: targetSceneName,
      draftPoses: undefined,
      timeline: scene.timeline.map((event) => (
        event.type === "goto_pose" && assignments[event.pose]
          ? { ...event, pose: assignments[event.pose] }
          : { ...event }
      )),
    },
  };
}
