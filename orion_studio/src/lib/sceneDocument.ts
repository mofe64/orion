import { expandSceneTimeline } from "./preview";
import type { ProjectCatalog, SceneDefinition, StoredSceneDocument } from "../types";

/** Compile Studio-only scene clips into Orion's version-1 runtime document. */
export function buildSceneDocument(
  scene: SceneDefinition,
  name: string,
  catalog: ProjectCatalog,
): StoredSceneDocument {
  const timeline = expandSceneTimeline(scene, catalog)
    .filter((event) => event.type !== "scene")
    .map(({ id: _id, ...event }) => event);
  return {
    format_version: 1,
    scene: {
      name,
      description: scene.description,
      timeline,
    },
  };
}
