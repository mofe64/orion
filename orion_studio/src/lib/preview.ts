import type {
  JointPositions,
  LightPreview,
  MotionDefinition,
  ProjectCatalog,
  SceneDefinition,
  SceneEvent,
} from "../types";
import { JOINT_NAMES } from "../types";

export const OFF_LIGHT: LightPreview = { red: 0, green: 0, blue: 0, white: 0 };

export function quinticBlend(progress: number): number {
  const value = Math.max(0, Math.min(1, progress));
  return value * value * value * (10 + value * (-15 + 6 * value));
}

export function interpolatePositions(
  start: JointPositions,
  target: JointPositions,
  progress: number,
): JointPositions {
  const blend = quinticBlend(progress);
  return Object.fromEntries(
    JOINT_NAMES.map((name) => [name, start[name] + (target[name] - start[name]) * blend]),
  ) as JointPositions;
}

export function motionDuration(motion: MotionDefinition | undefined): number {
  return motion?.keyframes.reduce((total, frame) => total + frame.duration + frame.hold, 0) ?? 0;
}

export function eventDuration(event: SceneEvent, catalog: ProjectCatalog): number {
  switch (event.type) {
    case "play_motion":
      return motionDuration(catalog.motions[event.motion]);
    case "goto_pose":
      return event.duration_seconds;
    case "light":
      return event.transition_seconds;
    case "audio":
      return 0.42;
    case "scene":
      return sceneDuration(catalog.scenes[event.scene], catalog);
  }
}

export function isDelayEvent(event: SceneEvent): boolean {
  return event.type === "goto_pose" && event.id.includes(":delay:");
}

export function eventEndPose(event: SceneEvent, catalog: ProjectCatalog): string | null {
  if (event.type === "goto_pose") return event.pose;
  if (event.type === "play_motion") {
    return catalog.motions[event.motion]?.keyframes.at(-1)?.pose ?? null;
  }
  if (event.type === "scene") {
    const nested = catalog.scenes[event.scene];
    if (!nested) return null;
    const movement = expandSceneTimeline(nested, catalog)
      .filter((part) => part.type === "goto_pose" || part.type === "play_motion")
      .at(-1);
    return movement ? eventEndPose(movement, catalog) : null;
  }
  return null;
}

export function sceneDuration(
  scene: SceneDefinition | undefined,
  catalog: ProjectCatalog,
  ancestors = new Set<string>(),
): number {
  if (!scene || ancestors.has(scene.name)) return 0;
  const nextAncestors = new Set(ancestors).add(scene.name);
  return Math.max(
    1,
    ...scene.timeline.map((event) => event.at + (
      event.type === "scene"
        ? sceneDuration(catalog.scenes[event.scene], catalog, nextAncestors)
        : eventDuration(event, catalog)
    )),
  );
}

/** Expand Studio-only scene clips into the runtime's existing semantic events. */
export function expandSceneTimeline(
  scene: SceneDefinition,
  catalog: ProjectCatalog,
  ancestors = new Set<string>(),
): SceneEvent[] {
  if (ancestors.has(scene.name)) return [];
  const nextAncestors = new Set(ancestors).add(scene.name);
  const expanded: SceneEvent[] = [];
  for (const event of scene.timeline) {
    if (event.type !== "scene") {
      expanded.push({ ...event });
      continue;
    }
    const nested = catalog.scenes[event.scene];
    if (!nested) continue;
    for (const child of expandSceneTimeline(nested, catalog, nextAncestors)) {
      expanded.push({
        ...child,
        id: `${event.id}:${child.id}`,
        at: event.at + child.at,
      });
    }
  }
  return expanded.sort((left, right) => left.at - right.at);
}

/**
 * Convert a composite movement clip into editable scene parts. Motion holds
 * remain visible as gaps because each following pose begins after the hold.
 */
export function splitSceneEvent(event: SceneEvent, catalog: ProjectCatalog): SceneEvent[] {
  if (event.type === "play_motion") {
    const motion = catalog.motions[event.motion];
    if (!motion) return [event];
    let cursor = event.at;
    const parts: SceneEvent[] = [];
    motion.keyframes.forEach((keyframe, index) => {
      parts.push({
        id: `${event.id}:pose:${index}`,
        at: cursor,
        type: "goto_pose" as const,
        pose: keyframe.pose,
        duration_seconds: keyframe.duration,
      });
      cursor += keyframe.duration;
      if (keyframe.hold > 0) {
        parts.push({
          id: `${event.id}:delay:${index}`,
          at: cursor,
          type: "goto_pose",
          pose: keyframe.pose,
          duration_seconds: keyframe.hold,
        });
        cursor += keyframe.hold;
      }
    });
    return parts;
  }
  if (event.type === "scene") {
    const nested = catalog.scenes[event.scene];
    if (!nested) return [event];
    const positioned = expandSceneTimeline(nested, catalog).map((part, index) => ({
      ...part,
      id: `${event.id}:part:${index}`,
      at: event.at + part.at,
    }));
    return positioned.flatMap((part) => (
      part.type === "play_motion" ? splitSceneEvent(part, catalog) : [part]
    ));
  }
  return [event];
}

function sampleMotion(
  motion: MotionDefinition,
  poses: ProjectCatalog["poses"],
  start: JointPositions,
  elapsed: number,
): JointPositions {
  let cursor = 0;
  let from = start;
  for (const keyframe of motion.keyframes) {
    const target = poses[keyframe.pose]?.positions;
    if (!target) continue;
    if (elapsed <= cursor + keyframe.duration) {
      return interpolatePositions(from, target, (elapsed - cursor) / keyframe.duration);
    }
    cursor += keyframe.duration;
    if (elapsed <= cursor + keyframe.hold) return target;
    cursor += keyframe.hold;
    from = target;
  }
  return from;
}

export function sampleScenePose(
  scene: SceneDefinition,
  catalog: ProjectCatalog,
  time: number,
): JointPositions {
  const baseline = catalog.poses.attentive?.positions ?? Object.fromEntries(
    JOINT_NAMES.map((name) => [name, 0]),
  ) as JointPositions;
  let current = baseline;

  for (const event of expandSceneTimeline(scene, catalog)) {
    if (event.at > time) break;
    if (event.type === "goto_pose") {
      const target = catalog.poses[event.pose]?.positions;
      if (!target) continue;
      const elapsed = time - event.at;
      current = elapsed < event.duration_seconds
        ? interpolatePositions(current, target, elapsed / event.duration_seconds)
        : target;
    }
    if (event.type === "play_motion") {
      const motion = catalog.motions[event.motion];
      if (motion) current = sampleMotion(motion, catalog.poses, current, time - event.at);
    }
  }
  return current;
}

function interpolateLight(start: LightPreview, target: LightPreview, progress: number): LightPreview {
  const value = Math.max(0, Math.min(1, progress));
  return {
    red: Math.round(start.red + (target.red - start.red) * value),
    green: Math.round(start.green + (target.green - start.green) * value),
    blue: Math.round(start.blue + (target.blue - start.blue) * value),
    white: Math.round(start.white + (target.white - start.white) * value),
  };
}

export function sampleSceneLight(
  scene: SceneDefinition,
  time: number,
  catalog?: ProjectCatalog,
): LightPreview {
  let current = OFF_LIGHT;
  const timeline = catalog ? expandSceneTimeline(scene, catalog) : scene.timeline;
  for (const event of timeline) {
    if (event.type !== "light" || event.at > time) continue;
    const target = {
      red: event.red,
      green: event.green,
      blue: event.blue,
      white: event.white,
    };
    const elapsed = time - event.at;
    current = event.transition_seconds > 0 && elapsed < event.transition_seconds
      ? interpolateLight(current, target, elapsed / event.transition_seconds)
      : target;
  }
  return current;
}
