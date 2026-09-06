import { describe, expect, it } from "vitest";
import type { CompiledTrajectoryPreview } from "../types";
import { projectCatalog } from "./catalog";
import { deleteMovementComponent, movementComponents } from "./movementComponents";

describe("split movement components", () => {
  const motion = structuredClone(projectCatalog.motions.look_at_left_expressive);
  motion.keyframes[3].hold = .4;
  it("replaces one movement with ordered pose and nonzero delay components", () => {
    const parts = movementComponents(motion);
    expect(parts.map(part => part.component.kind)).toEqual(["pose", "pose", "pose", "pose", "delay"]);
    expect(parts.map(part => part.component.index)).toEqual([0, 1, 2, 3, 3]);
    expect(parts[4].duration).toBe(.4);
    expect(parts.every(part => part.offset === null)).toBe(true);
  });
  it("uses compiled segment boundaries and preserves delay lengths", () => {
    const two = { ...motion, keyframes: [motion.keyframes[0], motion.keyframes[3]] };
    const sample = (time_from_start: number, keyframe_index: number) => ({ time_from_start, keyframe_index, positions: [], velocities: [], accelerations: [], keyframe: "", reached_markers: [] });
    const trajectory: CompiledTrajectoryPreview = { format_version: 2, compiler: "orion-runtime", motion_name: "test", space: "absolute", style: "attentive", joint_names: [], duration_seconds: 2, control_rate_hz: 50, peak_velocity_rad_s: 1, amplitude_scale: 1, markers: [], samples: [sample(0, 0), sample(.8, 1), sample(2, 1)] };
    const parts = movementComponents(two, trajectory);
    expect(parts.map(part => part.offset)).toEqual([0, .8, 1.6]);
    expect(parts[0].duration).toBeCloseTo(.8);
    expect(parts[1].duration).toBeCloseTo(.8);
    expect(parts[2].duration).toBeCloseTo(.4);
  });
  it("deletes a delay without deleting its pose or changing other transitions", () => {
    const result = deleteMovementComponent(motion, { index: 3, kind: "delay" });
    expect(result.keyframes).toHaveLength(4);
    expect(result.keyframes[3].hold).toBe(0);
    expect(result.keyframes[3].pose).toBe(motion.keyframes[3].pose);
    expect(result.keyframes[1]).toEqual(motion.keyframes[1]);
    expect(motion.keyframes[3].hold).toBe(.4);
  });
  it("deletes only the selected pose and its attached delay", () => {
    const result = deleteMovementComponent(motion, { index: 3, kind: "pose" });
    expect(result.keyframes.slice(0, 2)).toEqual(motion.keyframes.slice(0, 2));
    expect(result.keyframes[2]).toEqual({ ...motion.keyframes[2], arrival: "settle" });
    expect(motion.keyframes).toHaveLength(4);
  });
});
