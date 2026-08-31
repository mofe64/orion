export const JOINT_NAMES = [
  "base_yaw_joint",
  "shoulder_pitch_joint",
  "elbow_pitch_joint",
  "head_roll_joint",
  "head_pitch_joint",
] as const;

export type JointName = (typeof JOINT_NAMES)[number];
export type JointPositions = Record<JointName, number>;

export interface JointLimit {
  name: JointName;
  lower_rad: number;
  upper_rad: number;
}

export interface PoseDefinition {
  name: string;
  description: string;
  positions: JointPositions;
  source: "built_in" | "user";
  remote_revision?: string;
}

export interface MotionKeyframe {
  pose: string;
  duration: number;
  hold: number;
}

export interface MotionDefinition {
  name: string;
  description: string;
  keyframes: MotionKeyframe[];
  source: "built_in" | "user";
  remote_revision?: string;
}

export interface StoredPoseDocument {
  format_version: 1;
  units: "radians";
  poses: Record<string, {
    description: string;
    positions: JointPositions;
  }>;
}

export interface StoredMotionDocument {
  format_version: 1;
  motion: {
    name: string;
    description: string;
    keyframes: MotionKeyframe[];
  };
}

export interface BaseSceneEvent {
  id: string;
  at: number;
}

export interface PlayMotionEvent extends BaseSceneEvent {
  type: "play_motion";
  motion: string;
}

export interface GotoPoseEvent extends BaseSceneEvent {
  type: "goto_pose";
  pose: string;
  duration_seconds: number;
}

export interface LightEvent extends BaseSceneEvent {
  type: "light";
  red: number;
  green: number;
  blue: number;
  white: number;
  transition_seconds: number;
}

export interface AudioEvent extends BaseSceneEvent {
  type: "audio";
  cue: string;
}

export type SceneEvent = PlayMotionEvent | GotoPoseEvent | LightEvent | AudioEvent;

export interface SceneDefinition {
  format_version: 1;
  name: string;
  description: string;
  source: "built_in" | "user" | "draft";
  remote_revision?: string;
  timeline: SceneEvent[];
}

export interface StoredSceneDocument {
  format_version: 1;
  scene: {
    name: string;
    description: string;
    timeline: Array<Omit<SceneEvent, "id">>;
  };
}

export interface LightPreview {
  red: number;
  green: number;
  blue: number;
  white: number;
}

export interface ProjectCatalog {
  poses: Record<string, PoseDefinition>;
  motions: Record<string, MotionDefinition>;
  scenes: Record<string, SceneDefinition>;
  cues: string[];
  cueUrls: Record<string, string>;
  urdf: string;
  meshUrls: Record<string, string>;
  jointLimits: JointLimit[];
}

export interface RunStatus {
  run_id: number;
  name?: string;
  text?: string;
  state: string;
  error?: string;
}

export interface GatewayStatus {
  api_version: number;
  runtime: {
    schema_version: number;
    robot: string;
    build_revision: string;
    mode: string;
    torque_enabled: boolean;
    motion: RunStatus | null;
    last_motion: RunStatus | null;
  };
  scene: { active: RunStatus | null; last: RunStatus | null };
  speech: { active: RunStatus | null; last: RunStatus | null };
}

export interface GatewayCapabilities {
  api_version: number;
  capabilities: {
    goto: string[];
    motion: string[];
    scene: string[];
    speech: { max_text_bytes: number };
    cancel: string[];
    scene_publish: { format_version: number; max_body_bytes: number };
    scene_library: { read: boolean; create: boolean; update: "revision" };
    joint_limits: JointLimit[];
    pose_library: { read: boolean; create: boolean; update: false };
    motion_library: { read: boolean; create: boolean; update: false };
    movement_lifecycle: Array<"prepare" | "release">;
  };
}
