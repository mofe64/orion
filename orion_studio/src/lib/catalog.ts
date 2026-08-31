import { load } from "js-yaml";

import posesYaml from "../../../motion/config/poses.yaml?raw";
import motionLimitsYaml from "../../../motion/config/motion_limits.yaml?raw";
import orionUrdf from "../../../description/urdf/orion.urdf?raw";
import { JOINT_NAMES } from "../types";
import type {
  JointLimit,
  JointPositions,
  MotionDefinition,
  PoseDefinition,
  ProjectCatalog,
  SceneDefinition,
  SceneEvent,
} from "../types";

const userPoseFiles = import.meta.glob("../../../motion/user/poses/**/*.yaml", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

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
  format_version: number;
  units: string;
  poses: Record<string, { description?: string; positions: JointPositions }>;
}

interface MotionLimitsYaml {
  joints: Record<string, { operational_position: { lower: number; upper: number } }>;
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
  const poses: Record<string, PoseDefinition> = {};
  const documents: Array<[string, PoseYaml, PoseDefinition["source"]]> = [
    ["motion/config/poses.yaml", load(posesYaml) as PoseYaml, "built_in"],
    ...Object.entries(userPoseFiles).map(
      ([path, yaml]): [string, PoseYaml, PoseDefinition["source"]] => [
        path,
        load(yaml) as PoseYaml,
        "user",
      ],
    ),
  ];
  for (const [path, document, source] of documents) {
    if (document.format_version !== 1 || document.units !== "radians") {
      throw new Error(`Studio cannot load pose document: ${path}`);
    }
    for (const [name, pose] of Object.entries(document.poses)) {
      if (poses[name]) throw new Error(`Duplicate Orion pose name in Studio catalog: ${name}`);
      poses[name] = {
        name,
        description: pose.description ?? "",
        positions: pose.positions,
        source,
      };
    }
  }
  return poses;
}

function loadMotions(): Record<string, MotionDefinition> {
  return Object.fromEntries(
    Object.entries(motionFiles).map(([path, yaml]) => {
      const document = load(yaml) as MotionYaml;
      const motion: MotionDefinition = {
        name: document.motion.name,
        description: document.motion.description ?? "",
        keyframes: document.motion.keyframes.map((keyframe) => ({
          pose: keyframe.pose,
          duration: Number(keyframe.duration),
          hold: Number(keyframe.hold ?? 0),
        })),
        source: path.includes("/user/") ? "user" : "built_in",
      };
      return [motion.name, motion];
    }),
  );
}

function loadJointLimits(): JointLimit[] {
  const document = load(motionLimitsYaml) as MotionLimitsYaml;
  return JOINT_NAMES.map((name) => ({
    name,
    lower_rad: Number(document.joints[name].operational_position.lower),
    upper_rad: Number(document.joints[name].operational_position.upper),
  }));
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
  jointLimits: loadJointLimits(),
};
