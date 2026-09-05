import { JOINT_NAMES, type MotionDefinition, type PoseDefinition, type SceneDefinition } from "../types";

export type DraftAsset = SceneDefinition | MotionDefinition | PoseDefinition;
export type DraftKind = "scene" | "motion" | "pose";
const prefix = "orion-studio:draft:v1:";
const systemSceneReset = "orion-studio:migration:2026-09-05-system-scene-reset";

/** One-time, user-requested reset; other assets and future edits are preserved. */
export function resetAcknowledgementDrafts(): void {
  if (localStorage.getItem(systemSceneReset)) return;
  discardDraft("scene", "acknowledge_left");
  discardDraft("scene", "agreement");
  localStorage.setItem(systemSceneReset, "done");
}

const record = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
const number = (value: unknown) => typeof value === "number" && Number.isFinite(value);
const timing = (value: Record<string, unknown>) => (value.at === undefined || number(value.at)) && (value.on_marker === undefined || typeof value.on_marker === "string");
const events = (value: unknown, valid: (event: Record<string, unknown>) => boolean) => Array.isArray(value) && value.every(event => record(event) && typeof event.id === "string" && valid(event));

export function readDraft<T extends DraftAsset>(kind: DraftKind, original: T): T {
  try {
    const value = JSON.parse(localStorage.getItem(prefix + kind + ":" + original.name) ?? "null");
    if (!value || value.name !== original.name || typeof value.description !== "string") return structuredClone(original);
    if (kind === "scene" && (
      !events(value.motion, event => number(event.at) && typeof event.play === "string") ||
      !events(value.lighting, event => timing(event) && typeof event.effect === "string") ||
      !events(value.audio, event => timing(event) && typeof event.cue === "string") ||
      !record(value.finish) || value.finish.anchor !== "final_pose" || typeof value.finish.lighting !== "string"
    )) return structuredClone(original);
    if (kind === "motion" && (!Array.isArray(value.keyframes) || !value.keyframes.every((frame: unknown) => record(frame) &&
      number(frame.duration) && number(frame.hold) && ["through", "settle"].includes(String(frame.arrival)) &&
      (frame.pose === undefined || typeof frame.pose === "string") &&
      (frame.offsets === undefined || (record(frame.offsets) && Object.values(frame.offsets).every(number)))
    ))) return structuredClone(original);
    if (kind === "pose" && (!record(value.positions) || !JOINT_NAMES.every(joint => number(value.positions[joint])) ||
      !Array.isArray(value.tags) || !value.tags.every((tag: unknown) => typeof tag === "string"))) return structuredClone(original);
    return value as T;
  } catch { return structuredClone(original); }
}

export function saveDraft(kind: DraftKind, value: DraftAsset): void {
  localStorage.setItem(prefix + kind + ":" + value.name, JSON.stringify(value));
}
export function discardDraft(kind: DraftKind, name: string): void {
  localStorage.removeItem(prefix + kind + ":" + name);
}
