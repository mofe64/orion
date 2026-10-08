import { load } from "js-yaml";

import posesYaml from "../../../motion/config/v1/poses.yaml?raw";
import posesYamlV2 from "../../../motion/config/v2/poses.yaml?raw";
import calibration from "../../../simulation/mujoco/config/servo_calibration.json";
import modelReference from "../../../simulation/mujoco/config/model_reference.json";
import orionUrdf from "../../../description/urdf/orion.urdf?raw";
import orionUrdfV2 from "../../../description/urdf/orion-v2.urdf?raw";
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

type Files = Record<string, string>;
// Vite needs literal glob patterns, so each hardware version is listed explicitly.
const FILES = {
  v1: {
    userPoses: import.meta.glob("../../../motion/user/poses/v1/**/*.yaml", { eager: true, query: "?raw", import: "default" }) as Files,
    motions: import.meta.glob("../../../motion/motions/v1/**/*.yaml", { eager: true, query: "?raw", import: "default" }) as Files,
    scenes: import.meta.glob("../../../scenes/v1/**/*.yaml", { eager: true, query: "?raw", import: "default" }) as Files,
    meshes: import.meta.glob("../../../description/meshes/*.stl", { eager: true, query: "?url", import: "default" }) as Files,
  },
  v2: {
    userPoses: import.meta.glob("../../../motion/user/poses/v2/**/*.yaml", { eager: true, query: "?raw", import: "default" }) as Files,
    motions: import.meta.glob("../../../motion/motions/v2/**/*.yaml", { eager: true, query: "?raw", import: "default" }) as Files,
    scenes: import.meta.glob("../../../scenes/v2/**/*.yaml", { eager: true, query: "?raw", import: "default" }) as Files,
    meshes: import.meta.glob("../../../simulation/mujoco/v2/meshes/*.stl", { eager: true, query: "?url", import: "default" }) as Files,
  },
};
const cueFiles = import.meta.glob("../../../audio/cues/*.wav", {
  eager: true, query: "?url", import: "default",
}) as Record<string, string>;

function semanticFileName(path: string): string {
  return path.split("/").at(-1)?.replace(/\.[^.]+$/, "") ?? path;
}

function requireVersionTwo(document: { format_version?: number }, path: string): void {
  if (document.format_version !== 2) throw new Error(`${path} must use format_version 2 (v2 required).`);
}

function loadPoses(builtIn: string, builtInPath: string, userPoseFiles: Files): Record<string, PoseDefinition> {
  const poses: Record<string, PoseDefinition> = {};
  const documents: Array<[string, StoredPoseDocument, PoseDefinition["source"]]> = [
    [builtInPath, load(builtIn) as StoredPoseDocument, "built_in"],
    ...Object.entries(userPoseFiles).filter(([path]) => !path.includes("/_scene_owned/")).map(([path, yaml]) => [path, load(yaml) as StoredPoseDocument, "user"] as [string, StoredPoseDocument, "user"]),
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

function loadMotions(motionFiles: Files): Record<string, MotionDefinition> {
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

function loadScenes(sceneFiles: Files): Record<string, SceneDefinition> {
  const scenes: Record<string, SceneDefinition> = {};
  for (const [path, yaml] of Object.entries(sceneFiles).sort(([a], [b]) => a.localeCompare(b))) {
    const document = load(yaml) as StoredSceneDocument;
    requireVersionTwo(document, path);
    const scene = document.scene;
    if (scenes[scene.name]) throw new Error(`Duplicate Orion scene name: ${scene.name}`);
    for (const event of scene.lighting ?? []) {
      if (!(LIGHTING_EFFECTS as readonly string[]).includes(event.effect) && !["constant","pulse","breathe","fade"].includes(event.effect)) throw new Error(`Scene '${scene.name}' uses unknown lighting '${event.effect}'.`);
    }
    scenes[scene.name] = {
      format_version: 2,
      custom_poses: document.studio?.poses,
      custom_motions: document.studio?.motions,
      name: scene.name,
      description: scene.description ?? "",
      source: /\/scenes\/v[12]\/user\//.test(path) ? "user" : "built_in",
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

function meshUrls(files: Files): Record<string, string> {
  return Object.fromEntries(Object.entries(files).map(([path, url]) => [path.split("/").at(-1) ?? path, url]));
}

/** V2 has no tracked calibration; use the model's limits until a lamp reports its own. */
function urdfJointLimits(urdf: string): JointLimit[] {
  return JOINT_NAMES.map((name) => {
    const joint = urdf.match(new RegExp(`<joint name="${name}"[\\s\\S]*?<limit lower="([^"]+)" upper="([^"]+)"`));
    if (!joint) throw new Error(`The V2 robot description is missing ${name}.`);
    return { name, lower_rad: Number(joint[1]), upper_rad: Number(joint[2]) };
  });
}

export type HardwareVersion = "v1" | "v2";

export const catalogs: Record<HardwareVersion, ProjectCatalog> = {
  v1: {
    poses: loadPoses(posesYaml, "motion/config/v1/poses.yaml", FILES.v1.userPoses),
    motions: loadMotions(FILES.v1.motions),
    scenes: loadScenes(FILES.v1.scenes),
    cues: Object.keys(cueUrls).sort(),
    cueUrls,
    urdf: orionUrdf,
    meshUrls: meshUrls(FILES.v1.meshes),
    urdfJointOffsets,
    jointLimits: loadJointLimits(),
  },
  v2: {
    poses: loadPoses(posesYamlV2, "motion/config/v2/poses.yaml", FILES.v2.userPoses),
    motions: loadMotions(FILES.v2.motions),
    scenes: loadScenes(FILES.v2.scenes),
    cues: Object.keys(cueUrls).sort(),
    cueUrls,
    urdf: orionUrdfV2,
    meshUrls: meshUrls(FILES.v2.meshes),
    // The V2 calibration zero is the CAD zero pose, so no model offset applies.
    urdfJointOffsets: Object.fromEntries(JOINT_NAMES.map((name) => [name, 0])) as JointPositions,
    jointLimits: urdfJointLimits(orionUrdfV2),
  },
};

/** The bundled catalogue and 3D model for the hardware a lamp reports. */
export function catalogForHardware(hardware: string | undefined): ProjectCatalog {
  return catalogs[hardware === "v2" ? "v2" : "v1"];
}

export const projectCatalog: ProjectCatalog = catalogs.v1;
