import { JOINT_NAMES } from "../types";
import type {
  CompiledTrajectoryPreview,
  JointPositions,
  LightPreview,
  LightingEffectName,
  SceneDefinition,
  TrackTiming,
} from "../types";

export type SceneTrajectoryPreviews = Record<string, CompiledTrajectoryPreview>;

export const EFFECT_PREVIEWS: Record<LightingEffectName, LightPreview> = {
  warm_idle_breathe: { red: 32, green: 12, blue: 1, white: 78 },
  attentive_focus: { red: 28, green: 18, blue: 4, white: 120 },
  thinking_drift: { red: 24, green: 8, blue: 18, white: 70 },
  speaking_energy: { red: 42, green: 16, blue: 2, white: 135 },
  acknowledge_pulse: { red: 48, green: 18, blue: 1, white: 170 },
  curious_sweep: { red: 22, green: 18, blue: 10, white: 108 },
  delight_spark: { red: 68, green: 24, blue: 4, white: 210 },
  settle_glow: { red: 25, green: 9, blue: 1, white: 86 },
  off: { red: 0, green: 0, blue: 0, white: 0 },
};

export function sampleCompiledTrajectory(
  trajectory: CompiledTrajectoryPreview,
  elapsed: number,
): JointPositions {
  const time = Math.max(0, Math.min(elapsed, trajectory.duration_seconds));
  let sample = trajectory.samples[0];
  for (const candidate of trajectory.samples) {
    if (candidate.time_from_start > time + 1e-9) break;
    sample = candidate;
  }
  return Object.fromEntries(
    JOINT_NAMES.map((name, index) => [name, Number(sample.positions[index])]),
  ) as JointPositions;
}

export function markerTime(trajectory: CompiledTrajectoryPreview | null, name: string): number | null {
  return trajectory?.markers.find((marker) => marker.name === name)?.time_seconds ?? null;
}

export function sceneMarkerTime(
  scene: SceneDefinition,
  trajectories: SceneTrajectoryPreviews,
  name: string,
): number | null {
  for (const event of [...scene.motion].sort((left, right) => left.at - right.at)) {
    const local = markerTime(trajectories[event.id] ?? null, name);
    if (local !== null) return event.at + local;
  }
  return null;
}

export function triggerTime(
  timing: TrackTiming,
  scene: SceneDefinition,
  trajectories: SceneTrajectoryPreviews,
): number | null {
  if (timing.at !== undefined) return timing.at;
  return timing.on_marker ? sceneMarkerTime(scene, trajectories, timing.on_marker) : null;
}

export function sceneDuration(
  scene: SceneDefinition,
  trajectories: SceneTrajectoryPreviews,
): number {
  const motionEnd = scene.motion.reduce((end, event) => {
    const duration = trajectories[event.id]?.duration_seconds ?? 0;
    return Math.max(end, event.at + duration);
  }, 0);
  const lightEnd = scene.lighting.reduce((end, event) => {
    const start = triggerTime(event, scene, trajectories);
    return start === null ? end : Math.max(end, start + (event.duration ?? 0.8));
  }, 0);
  const audioEnd = scene.audio.reduce(
    (end, event) => Math.max(end, triggerTime(event, scene, trajectories) ?? 0),
    0,
  );
  return Math.max(1, motionEnd, lightEnd, audioEnd);
}

export function sampleSceneTrajectory(
  scene: SceneDefinition,
  trajectories: SceneTrajectoryPreviews,
  elapsed: number,
): JointPositions | null {
  let active: { at: number; trajectory: CompiledTrajectoryPreview } | null = null;
  for (const event of scene.motion) {
    const trajectory = trajectories[event.id];
    if (trajectory && event.at <= elapsed && (!active || event.at >= active.at)) {
      active = { at: event.at, trajectory };
    }
  }
  return active
    ? sampleCompiledTrajectory(active.trajectory, elapsed - active.at)
    : null;
}

export function sceneMarkers(
  scene: SceneDefinition,
  trajectories: SceneTrajectoryPreviews,
): Array<{ name: string; time_seconds: number; motion_event_id: string }> {
  return scene.motion.flatMap((event) =>
    (trajectories[event.id]?.markers ?? []).map((marker) => ({
      name: marker.name,
      time_seconds: event.at + marker.time_seconds,
      motion_event_id: event.id,
    })),
  );
}

export function validateSceneMotionSchedule(
  scene: SceneDefinition,
  trajectories: SceneTrajectoryPreviews,
): void {
  let previousEnd = 0;
  for (const event of [...scene.motion].sort((left, right) => left.at - right.at)) {
    const trajectory = trajectories[event.id];
    if (!trajectory) throw new Error(`No Rust preview is available for ${event.play}.`);
    if (event.at < previousEnd - 1e-9) {
      throw new Error(`Motion ${event.play} overlaps the preceding clip. Move it to ${previousEnd.toFixed(2)} s or later.`);
    }
    previousEnd = event.at + trajectory.duration_seconds;
  }
}

export function sampleSceneLight(
  scene: SceneDefinition,
  elapsed: number,
  trajectories: SceneTrajectoryPreviews,
): LightPreview {
  let effect: LightingEffectName = "off";
  let latest = -1;
  for (const event of scene.lighting) {
    const at = triggerTime(event, scene, trajectories);
    if (at !== null && at <= elapsed && at >= latest) {
      latest = at;
      effect = event.effect;
    }
  }
  return EFFECT_PREVIEWS[effect];
}
