import type { ProjectCatalog, SceneDefinition } from "../types";

/** Reserve room for owner scenes without filling Home with the entire library. */
export function quickExpressions(catalog: ProjectCatalog, limit = 6): SceneDefinition[] {
  const count = Math.max(0, Math.floor(limit));
  const scenes = Object.values(catalog.scenes).sort((a,b) => a.name.localeCompare(b.name));
  const builtIns = scenes.filter(scene => scene.source === "built_in" && scene.name !== "return_home" && !scene.name.includes("deployment"));
  const published = scenes.filter(scene => scene.source === "user");
  const featured = builtIns.slice(0,Math.min(3,count));
  return [...featured,...published,...builtIns.slice(featured.length)].slice(0,count);
}
