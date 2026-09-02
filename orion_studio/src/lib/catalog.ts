import { load } from "js-yaml";

import posesYaml from "../../../motion/config/poses.yaml?raw";
import calibration from "../../../simulation/mujoco/config/servo_calibration.json";
import modelReference from "../../../simulation/mujoco/config/model_reference.json";
import orionUrdf from "../../../description/urdf/orion.urdf?raw";
import { JOINT_NAMES, LIGHTING_EFFECTS, MOTION_STYLES } from "../types";
import type {
  JointLimit,
  JointPositions,
  MotionDefinition,
  MotionKeyframe,
  PoseDefinition,
  ProjectCatalog,
  SceneAudioEvent,
  SceneDefinition,
  SceneLightingEvent,
  SceneMotionClip,
  StoredMotionDocument,
  StoredPoseDocument,
  StoredSceneDocument,
} from "../types";

const userPoseFiles = import.meta.glob("../../../motion/user/poses/**/*.yaml", {
  eager: true, query: "?raw", import: "default",
}) as Record<string, string>;
const motionFiles = import.meta.glob("../../../motion/motions/**/*.yaml", {
  eager: true, query: "?raw", import: "default",
}) as Record<string, string>;
const sceneFiles = import.meta.glob("../../../scenes/**/*.yaml", {
  eager: true, query: "?raw", import: "default",
}) as Record<string, string>;
const cueFiles = import.meta.glob("../../../audio/cues/*.wav", {
  eager: true, query: "?url", import: "default",
}) as Record<string, string>;
const meshFiles = import.meta.glob("../../../description/meshes/*.stl", {
  eager: true, query: "?url", import: "default",
}) as Record<string, string>;

function semanticFileName(path: string): string {
  return path.split("/").at(-1)?.replace(/\.[^.]+$/, "") ?? path;
}

function requireVersionTwo(document: { format_version?: number }, path: string): void {
  if (document.format_version !== 2) throw new Error(`${path} must use format_version 2 (v2 required).`);
}

function loadPoses(): Record<string, PoseDefinition> {
  const poses: Record<string, PoseDefinition> = {};
  const documents: Array<[string, StoredPoseDocument, PoseDefinition["source"]]> = [
    ["motion/config/poses.yaml", load(posesYaml) as StoredPoseDocument, "built_in"],
    ...Object.entries(userPoseFiles).map(([path, yaml]) => [path, load(yaml) as StoredPoseDocument, "user"] as [string, StoredPoseDocument, "user"]),
  ];
  for (const [path, document, source] of documents) {
    requireVersionTwo(document, path);
    if (document.units !== "radians") throw new Error(`${path} must use radians.`);
    for (const [name, pose] of Object.entries(document.poses)) {
      if (poses[name]) throw new Error(`Duplicate Orion pose name: ${name}`);
      if (Object.keys(pose.positions).length !== JOINT_NAMES.length) throw new Error(`Pose '${name}' must contain all Orion joints.`);
      poses[name] = {
        name,
        description: pose.description ?? "",
        tags: pose.tags ?? [],
        idle_profile: pose.idle_profile,
        default_lighting: pose.default_lighting,
        positions: pose.positions,
        source,
      };
    }
  }
  return poses;
}

function loadMotions(): Record<string, MotionDefinition> {
  const motions: Record<string, MotionDefinition> = {};
  for (const [path, yaml] of Object.entries(motionFiles)) {
    const document = load(yaml) as StoredMotionDocument;
    requireVersionTwo(document, path);
    const motion = document.motion;
    if (!MOTION_STYLES.includes(motion.style)) throw new Error(`Motion '${motion.name}' uses an unknown style.`);
    const keyframes: MotionKeyframe[] = motion.keyframes.map((frame) => ({
      pose: frame.pose,
      offsets: frame.offsets,
      duration: Number(frame.duration),
      arrival: frame.arrival,
      hold: Number(frame.hold ?? 0),
      marker: frame.marker,
    }));
    motions[motion.name] = {
      name: motion.name,
      description: motion.description ?? "",
      space: motion.space,
      style: motion.style,
      return_to_anchor: motion.return_to_anchor ?? false,
      keyframes,
      source: path.includes("/user/") ? "user" : "built_in",
    };
  }
  return motions;
}

function withId<T extends object>(value: T, id: string): T & { id: string } {
  return { ...value, id };
}

function loadScenes(): Record<string, SceneDefinition> {
  const scenes: Record<string, SceneDefinition> = {};
  for (const [path, yaml] of Object.entries(sceneFiles).sort(([a], [b]) => a.localeCompare(b))) {
    const document = load(yaml) as StoredSceneDocument;
    requireVersionTwo(document, path);
    const scene = document.scene;
    if (scenes[scene.name]) throw new Error(`Duplicate Orion scene name: ${scene.name}`);
    for (const event of scene.lighting ?? []) {
      if (!LIGHTING_EFFECTS.includes(event.effect)) throw new Error(`Scene '${scene.name}' uses unknown lighting '${event.effect}'.`);
    }
    scenes[scene.name] = {
      format_version: 2,
      name: scene.name,
      description: scene.description ?? "",
      source: path.includes("/scenes/user/") ? "user" : "built_in",
      motion: (scene.motion ?? []).map((event, index) => withId(event, `${scene.name}-motion-${index}`)) as SceneMotionClip[],
      lighting: (scene.lighting ?? []).map((event, index) => withId(event, `${scene.name}-light-${index}`)) as SceneLightingEvent[],
      audio: (scene.audio ?? []).map((event, index) => withId(event, `${scene.name}-audio-${index}`)) as SceneAudioEvent[],
      finish: scene.finish,
    };
  }
  return scenes;
}

function loadJointLimits(): JointLimit[] {
  const radiansPerStep = Math.PI * 2 / calibration.encoder_resolution;
  return JOINT_NAMES.map((name) => {
    const joint = calibration.joints[name];
    const first = joint.safe_min_delta_raw * radiansPerStep / joint.encoder_direction;
    const second = joint.safe_max_delta_raw * radiansPerStep / joint.encoder_direction;
    return { name, lower_rad: Math.min(first, second), upper_rad: Math.max(first, second) };
  });
}

const cueUrls = Object.fromEntries(Object.entries(cueFiles).map(([path, url]) => [semanticFileName(path), url]));
const urdfJointOffsets = Object.fromEntries(
  JOINT_NAMES.map((name) => [name, -Number(modelReference.joint_reference_radians[name])]),
) as JointPositions;

export const projectCatalog: ProjectCatalog = {
  poses: loadPoses(),
  motions: loadMotions(),
  scenes: loadScenes(),
  cues: Object.keys(cueUrls).sort(),
  cueUrls,
  urdf: orionUrdf,
  meshUrls: Object.fromEntries(Object.entries(meshFiles).map(([path, url]) => [path.split("/").at(-1) ?? path, url])),
  urdfJointOffsets,
  jointLimits: loadJointLimits(),
};
