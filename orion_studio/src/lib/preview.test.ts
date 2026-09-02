import { describe, expect, it } from "vitest";

import type { CompiledTrajectoryPreview, SceneDefinition } from "../types";
import {
  markerTime,
  sampleCompiledTrajectory,
  sampleSceneLight,
  sampleSceneTrajectory,
  sceneDuration,
  sceneMarkers,
  validateSceneMotionSchedule,
} from "./preview";

const trajectory: CompiledTrajectoryPreview = {
  format_version: 2, compiler: "orion-runtime", motion_name: "test", space: "absolute",
  style: "attentive", joint_names: ["base_yaw_joint", "shoulder_pitch_joint", "elbow_pitch_joint", "head_roll_joint", "head_pitch_joint"],
  duration_seconds: 1, control_rate_hz: 50, peak_velocity_rad_s: 2, amplitude_scale: 1,
  markers: [{ name: "notice", time_seconds: 0.6 }],
  samples: [
    { time_from_start: 0, positions: [0, 0, 0, 0, 0], velocities: [0, 0, 0, 0, 0], accelerations: [0, 0, 0, 0, 0], keyframe_index: 0, keyframe: "start", reached_markers: [] },
    { time_from_start: 0.5, positions: [0.5, 0, 0, 0, 0], velocities: [1, 0, 0, 0, 0], accelerations: [0, 0, 0, 0, 0], keyframe_index: 0, keyframe: "through", reached_markers: [] },
    { time_from_start: 1, positions: [1, 0, 0, 0, 0], velocities: [0, 0, 0, 0, 0], accelerations: [0, 0, 0, 0, 0], keyframe_index: 1, keyframe: "settle", reached_markers: ["notice"] },
  ],
};

const scene: SceneDefinition = {
  format_version: 2, name: "test", description: "", source: "draft",
  motion: [{ id: "m", at: 0.2, play: "test" }],
  lighting: [{ id: "l", on_marker: "notice", effect: "acknowledge_pulse", duration: 0.8 }],
  audio: [{ id: "a", on_marker: "notice", cue: "notice_warm" }],
  finish: { anchor: "final_pose", lighting: "pose_default" },
};
const trajectories = { m: trajectory };

describe("Rust trajectory preview", () => {
  it("samples the exact 50 Hz document without inventing interpolation", () => {
    expect(sampleCompiledTrajectory(trajectory, 0.49).base_yaw_joint).toBe(0);
    expect(sampleCompiledTrajectory(trajectory, 0.5).base_yaw_joint).toBe(0.5);
    expect(sampleCompiledTrajectory(trajectory, 99).base_yaw_joint).toBe(1);
  });

  it("retimes marker tracks from compiler output", () => {
    expect(markerTime(trajectory, "notice")).toBe(0.6);
    expect(sceneDuration(scene, trajectories)).toBeCloseTo(1.6);
    expect(sampleSceneLight(scene, 0.79, trajectories).white).toBe(0);
    expect(sampleSceneLight(scene, 0.8, trajectories).white).toBe(170);
  });

  it("previews every sequential scene clip on the shared scene clock", () => {
    const second = { ...trajectory, motion_name: "second", duration_seconds: 0.5 };
    const sequence: SceneDefinition = {
      ...scene,
      motion: [
        { id: "m", at: 0.2, play: "test" },
        { id: "m2", at: 1.2, play: "second" },
      ],
    };
    const compiled = { m: trajectory, m2: second };

    validateSceneMotionSchedule(sequence, compiled);
    expect(sceneDuration(sequence, compiled)).toBeCloseTo(1.7);
    expect(sampleSceneTrajectory(sequence, compiled, 0.1)).toBeNull();
    expect(sampleSceneTrajectory(sequence, compiled, 0.7)?.base_yaw_joint).toBe(0.5);
    expect(sampleSceneTrajectory(sequence, compiled, 1.3)?.base_yaw_joint).toBe(0);
    const markers = sceneMarkers(sequence, compiled);
    expect(markers.map(({ name, motion_event_id }) => ({ name, motion_event_id }))).toEqual([
      { name: "notice", motion_event_id: "m" },
      { name: "notice", motion_event_id: "m2" },
    ]);
    expect(markers[0].time_seconds).toBeCloseTo(0.8);
    expect(markers[1].time_seconds).toBeCloseTo(1.8);
  });

  it("rejects a preview schedule when Rust-compiled clip durations overlap", () => {
    const overlap: SceneDefinition = {
      ...scene,
      motion: [
        { id: "m", at: 0.2, play: "test" },
        { id: "m2", at: 1.1, play: "test" },
      ],
    };
    expect(() => validateSceneMotionSchedule(overlap, { m: trajectory, m2: trajectory }))
      .toThrow(/overlaps the preceding clip/);
  });
});
