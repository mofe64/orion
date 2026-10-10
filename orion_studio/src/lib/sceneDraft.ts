import type { SceneDefinition } from "../types";
import { discardDraft, saveDraft } from "./drafts";
import { renameScene } from "./scenePoses";

export function sceneName(value: string): string {
  return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "");
}

export function sceneNameProblem(scene: SceneDefinition, value: string, scenes: Record<string, SceneDefinition>): string | null {
  const name = sceneName(value);
  if (!name) return "Give your scene a name first.";
  if (name !== scene.name && scenes[name]) return "That scene name is already in use. Choose another name.";
  if (scenes[name]?.source === "built_in") return "Choose a name for your own scene.";
  return null;
}

export function saveNamedScene(scene: SceneDefinition, name: string): SceneDefinition {
  const saved = name === scene.name ? scene : renameScene(scene, name);
  // Store the renamed copy before removing the old draft so a failed write
  // cannot erase the owner's work.
  saveDraft("scene", saved);
  if (name !== scene.name && scene.source === "draft") discardDraft("scene", scene.name);
  return saved;
}

export interface DraftSaveResult { scene: SceneDefinition; failed: boolean }
export function sceneSaveLabel(scene: SceneDefinition, result: DraftSaveResult | null): string {
  if (result?.scene !== scene) return "Saving…";
  return result.failed ? "Couldn't save" : "Saved on this computer";
}

export function publishDisabledReason(connected: boolean, timingPending: boolean): string | null {
  return !connected ? "Connect Orion to publish." : timingPending ? "Orion is still calculating timing. Wait for the preview to be ready." : null;
}
