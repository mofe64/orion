import type { CompiledTrajectoryPreview, MotionDefinition } from "../types";

export interface MovementComponent { index: number; kind: "pose" | "delay" }
export function movementComponents(motion: MotionDefinition, trajectory?: CompiledTrajectoryPreview) {
  let start = 0;
  return motion.keyframes.flatMap((frame, index) => {
    const next = trajectory?.samples.find(sample => sample.keyframe_index === index + 1)?.time_from_start;
    const end = trajectory ? next ?? trajectory.duration_seconds : null;
    const hold = frame.arrival === "settle" ? frame.hold : 0;
    const duration = end === null ? null : Math.max(0, end - start - hold);
    const result = [{ component: { index, kind: "pose" } as MovementComponent, label: frame.pose ?? `Relative pose ${index + 1}`, offset: trajectory ? start : null, duration }];
    if (hold > 0) result.push({ component: { index, kind: "delay" }, label: "Delay", offset: end === null ? null : end - hold, duration: hold });
    start = end ?? start + frame.duration + hold;
    return result;
  });
}

export function deleteMovementComponent(motion: MotionDefinition, component: MovementComponent): MotionDefinition {
  const keyframes = component.kind === "delay"
    ? motion.keyframes.map((frame, index) => index === component.index ? { ...frame, hold: 0 } : frame)
    : motion.keyframes.filter((_, index) => index !== component.index);
  // Removing the last pose makes its predecessor the endpoint, which must settle.
  if (component.kind === "pose" && component.index === motion.keyframes.length - 1 && keyframes.length) {
    keyframes[keyframes.length - 1] = { ...keyframes[keyframes.length - 1], arrival: "settle" };
  }
  return { ...motion, keyframes };
}

/** Reorder inside the compiled movement so all components keep one continuous path. */
export function moveMovementComponent(motion: MotionDefinition, component: MovementComponent, time: number, trajectory: CompiledTrajectoryPreview): MotionDefinition {
  const parts = movementComponents(motion,trajectory).filter(part => part.component.kind === "pose");
  const keyframes = motion.keyframes.map(frame => ({ ...frame }));
  const destination = parts.filter(part => (part.offset ?? 0) < time).length;
  if (component.kind === "delay") {
    const hold = keyframes[component.index].hold;
    keyframes[component.index].hold = 0;
    const target = Math.max(0,Math.min(keyframes.length-1,destination-1));
    keyframes[target] = { ...keyframes[target], arrival: "settle", hold: keyframes[target].hold+hold };
  } else {
    const [frame] = keyframes.splice(component.index,1);
    keyframes.splice(Math.max(0,destination-(destination>component.index ? 1 : 0)),0,frame);
  }
  keyframes[keyframes.length-1].arrival = "settle";
  // A relative movement must still end at its anchor after reordering.
  if (motion.space === "anchor_relative" && Object.values(keyframes.at(-1)!.offsets ?? {}).some(value => value !== 0)) {
    keyframes.push({ offsets: {}, duration: .5, arrival: "settle", hold: 0 });
  }
  return { ...motion, keyframes };
}
