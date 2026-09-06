import type { SceneDefinition, StoredSceneDocument } from "../types";

/** Strip Studio identities while preserving the authored v2 parallel tracks. */
export function buildSceneDocument(scene: SceneDefinition, name = scene.name): StoredSceneDocument {
  return {
    format_version: 2,
    ...(Object.keys(scene.custom_poses ?? {}).length || Object.keys(scene.custom_motions ?? {}).length ? { studio: { poses: scene.custom_poses ?? {}, motions: scene.custom_motions ?? {} } } : {}),
    scene: {
      name,
      description: scene.description,
      motion: scene.motion.map(({ id: _id, after_previous: _automatic, show_parts: _parts, ...event }) => event),
      lighting: scene.lighting.map(({ id: _id, after_previous: _after, delay: _delay, resolved_at, ...event }) => resolved_at === undefined ? event : { ...event, at: resolved_at, on_marker: undefined }),
      audio: scene.audio.map(({ id: _id, duration: _duration, after_previous: _after, delay: _delay, resolved_at, ...event }) => resolved_at === undefined ? event : { ...event, at: resolved_at, on_marker: undefined }),
      finish: scene.finish,
    },
  };
}
