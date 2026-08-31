import { load } from "js-yaml";

import posesYaml from "../../../motion/config/poses.yaml?raw";
import orionUrdf from "../../../description/urdf/orion.urdf?raw";
import type {
  JointPositions,
  MotionDefinition,
  PoseDefinition,
  ProjectCatalog,
  SceneDefinition,
  SceneEvent,
} from "../types";

const motionFiles = import.meta.glob("../../../motion/motions/**/*.yaml", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

const sceneFiles = import.meta.glob("../../../scenes/**/*.yaml", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

const cueFiles = import.meta.glob("../../../audio/cues/*.wav", {
  eager: true,
  query: "?url",
  import: "default",
}) as Record<string, string>;

const meshFiles = import.meta.glob("../../../description/meshes/*.stl", {
  eager: true,
  query: "?url",
  import: "default",
}) as Record<string, string>;

interface PoseYaml {
  poses: Record<string, { description?: string; positions: JointPositions }>;
}

interface MotionYaml {
  motion: {
    name: string;
    description?: string;
    keyframes: Array<{ pose: string; duration: number; hold?: number }>;
  };
}

interface SceneYamlEvent {
  at: number;
  type: SceneEvent["type"];
  [key: string]: unknown;
}

interface SceneYaml {
  format_version: number;
  scene: {
    name: string;
    description?: string;
    timeline: SceneYamlEvent[];
  };
}

function semanticFileName(path: string): string {
  return path.split("/").at(-1)?.replace(/\.[^.]+$/, "") ?? path;
}

function assetFileName(path: string): string {
  return path.split("/").at(-1) ?? path;
}

function loadPoses(): Record<string, PoseDefinition> {
  const document = load(posesYaml) as PoseYaml;
  return Object.fromEntries(
    Object.entries(document.poses).map(([name, pose]) => [
      name,
      {
        name,
        description: pose.description ?? "",
        positions: pose.positions,
      },
    ]),
  );
}

function loadMotions(): Record<string, MotionDefinition> {
  return Object.fromEntries(
    Object.values(motionFiles).map((yaml) => {
      const document = load(yaml) as MotionYaml;
      const motion: MotionDefinition = {
        name: document.motion.name,
        description: document.motion.description ?? "",
        keyframes: document.motion.keyframes.map((keyframe) => ({
          pose: keyframe.pose,
          duration: Number(keyframe.duration),
          hold: Number(keyframe.hold ?? 0),
        })),
      };
      return [motion.name, motion];
    }),
  );
}

function loadScenes(): Record<string, SceneDefinition> {
  const scenes: Record<string, SceneDefinition> = {};
  for (const [path, yaml] of Object.entries(sceneFiles).sort(([left], [right]) => left.localeCompare(right))) {
    const document = load(yaml) as SceneYaml;
    if (document.format_version !== 1) {
      throw new Error(`Studio cannot load scene format ${document.format_version}: ${path}`);
    }
    if (scenes[document.scene.name]) {
      throw new Error(`Duplicate Orion scene name in Studio catalog: ${document.scene.name}`);
    }
    scenes[document.scene.name] = {
      format_version: 1,
      name: document.scene.name,
      description: document.scene.description ?? "",
      source: path.includes("/scenes/user/") ? "user" : "built_in",
      timeline: document.scene.timeline.map((event, index) => ({
        ...event,
        id: `${document.scene.name}-${index}`,
        at: Number(event.at),
      })) as SceneEvent[],
    };
  }
  return scenes;
}

function loadMeshes(): Record<string, string> {
  return Object.fromEntries(
    Object.entries(meshFiles).map(([path, url]) => [assetFileName(path), url]),
  );
}

function loadCueUrls(): Record<string, string> {
  return Object.fromEntries(
    Object.entries(cueFiles).map(([path, url]) => [semanticFileName(path), url]),
  );
}

const cueUrls = loadCueUrls();

export const projectCatalog: ProjectCatalog = {
  poses: loadPoses(),
  motions: loadMotions(),
  scenes: loadScenes(),
  cues: Object.keys(cueUrls).sort(),
  cueUrls,
  urdf: orionUrdf,
  meshUrls: loadMeshes(),
};
