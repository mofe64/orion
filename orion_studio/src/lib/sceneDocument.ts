import type { SceneDefinition, StoredSceneDocument } from "../types";

/** Strip Studio identities while preserving the authored v2 parallel tracks. */
export function buildSceneDocument(scene: SceneDefinition, name = scene.name): StoredSceneDocument {
  return {
    format_version: 2,
    scene: {
      name,
      description: scene.description,
      motion: scene.motion.map(({ id: _id, ...event }) => event),
      lighting: scene.lighting.map(({ id: _id, ...event }) => event),
      audio: scene.audio.map(({ id: _id, ...event }) => event),
      finish: scene.finish,
    },
  };
}
