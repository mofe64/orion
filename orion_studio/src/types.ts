export const JOINT_NAMES = [
  "base_yaw_joint",
  "shoulder_pitch_joint",
  "elbow_pitch_joint",
  "head_roll_joint",
  "head_pitch_joint",
] as const;

export const LIGHTING_EFFECTS = [
  "warm_idle_breathe",
  "attentive_focus",
  "thinking_drift",
  "speaking_energy",
  "acknowledge_pulse",
  "curious_sweep",
  "delight_spark",
  "settle_glow",
  "off",
] as const;

export const MOTION_STYLES = [
  "living_idle",
  "attentive",
  "expressive_turn",
  "speaking_calm",
  "speaking_emphatic",
  "thinking",
  "quick_reaction",
  "return_home",
] as const;

export type JointName = (typeof JOINT_NAMES)[number];
export type JointPositions = Record<JointName, number>;
export type JointOffsets = Partial<Record<JointName, number>>;
export type LightingEffectName = (typeof LIGHTING_EFFECTS)[number];
export type MotionStyleName = (typeof MOTION_STYLES)[number];
export type AssetSource = "built_in" | "user" | "draft";

export interface JointLimit {
  name: JointName;
  lower_rad: number;
  upper_rad: number;
}

export interface PoseDefinition {
  name: string;
  description: string;
  tags: string[];
  idle_profile?: string;
  default_lighting?: LightingEffectName;
  positions: JointPositions;
  source: AssetSource;
  remote_revision?: string;
}

export type MotionSpace = "absolute" | "anchor_relative";
export type KeyframeArrival = "through" | "settle";

export interface MotionKeyframe {
  pose?: string;
  offsets?: JointOffsets;
  duration: number;
  arrival: KeyframeArrival;
  hold: number;
  marker?: string;
}

export interface MotionDefinition {
  name: string;
  description: string;
  space: MotionSpace;
  style: MotionStyleName;
  return_to_anchor: boolean;
  keyframes: MotionKeyframe[];
  source: Exclude<AssetSource, "draft">;
  remote_revision?: string;
}

export interface StoredPoseDocument {
  format_version: 2;
  units: "radians";
  poses: Record<string, {
    description: string;
    tags: string[];
    idle_profile?: string;
    default_lighting?: LightingEffectName;
    positions: JointPositions;
  }>;
}

export interface StoredMotionDocument {
  format_version: 2;
  motion: {
    name: string;
    description: string;
    space: MotionSpace;
    style: MotionStyleName;
    return_to_anchor?: boolean;
    keyframes: Array<Omit<MotionKeyframe, "hold"> & { hold?: number }>;
  };
}

export interface TrackTiming {
  at?: number;
  on_marker?: string;
}

export interface SceneMotionClip {
  id: string;
  at: number;
  play: string;
}

export interface SceneLightingEvent extends TrackTiming {
  id: string;
  effect: LightingEffectName;
  intensity?: number;
  duration?: number;
  transition?: number;
  palette?: string;
}

export interface SceneAudioEvent extends TrackTiming {
  id: string;
  cue: string;
}

export interface SceneFinish {
  anchor: "final_pose";
  lighting: "pose_default" | LightingEffectName;
}

export interface SceneDefinition {
  format_version: 2;
  name: string;
  description: string;
  source: AssetSource;
  remote_revision?: string;
  motion: SceneMotionClip[];
  lighting: SceneLightingEvent[];
  audio: SceneAudioEvent[];
  finish: SceneFinish;
}

export interface StoredSceneDocument {
  format_version: 2;
  scene: {
    name: string;
    description: string;
    motion: Array<Omit<SceneMotionClip, "id">>;
    lighting: Array<Omit<SceneLightingEvent, "id">>;
    audio: Array<Omit<SceneAudioEvent, "id">>;
    finish: SceneFinish;
  };
}

export interface LightPreview {
  red: number;
  green: number;
  blue: number;
  white: number;
}

export interface CompiledTrajectorySample {
  time_from_start: number;
  positions: number[];
  velocities: number[];
  accelerations: number[];
  keyframe_index: number;
  keyframe: string;
  reached_markers: string[];
}

export interface CompiledTrajectoryPreview {
  format_version: 2;
  compiler: "orion-runtime";
  motion_name: string;
  space: MotionSpace;
  style: MotionStyleName;
  joint_names: JointName[];
  duration_seconds: number;
  control_rate_hz: number;
  peak_velocity_rad_s: number;
  amplitude_scale: number;
  markers: Array<{ name: string; time_seconds: number }>;
  samples: CompiledTrajectorySample[];
}

export interface ProjectCatalog {
  poses: Record<string, PoseDefinition>;
  motions: Record<string, MotionDefinition>;
  scenes: Record<string, SceneDefinition>;
  cues: string[];
  cueUrls: Record<string, string>;
  urdf: string;
  meshUrls: Record<string, string>;
  urdfJointOffsets: JointPositions;
  jointLimits: JointLimit[];
}

export interface RunStatus {
  run_id: number;
  name?: string;
  state: string;
  error?: string | null;
  reached_markers?: string[];
}

export interface CharacterStatus {
  enabled: boolean;
  state: "off" | "starting" | "home_idle" | "pose_idle" | "listening" | "thinking" | "speaking" | "foreground_scene" | "settling" | "shutting_down";
  active_anchor: JointPositions | null;
  active_clip: string | null;
  next_idle_category: "micro" | "large" | null;
}

export interface GatewayStatus {
  api_version: 2;
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
  character: CharacterStatus;
}

export interface GatewayCapabilities {
  api_version: 2;
  capabilities: {
    goto: string[];
    motion: string[];
    scene: string[];
    pose_format_version: 2;
    motion_format_version: 2;
    scene_format_version: 2;
    speech: { format: "pcm16_mono_24000_hz"; max_bytes: number; max_seconds: number };
    cancel: Array<"movement" | "scene" | "speech">;
    scene_publish: { format_version: 2; max_body_bytes: number };
    scene_preview: { format_version: 2; max_body_bytes: number; persisted: false };
    scene_library: { read: boolean; create: boolean; update: "revision" };
    joint_limits: JointLimit[];
    pose_library: { read: boolean; create: boolean; update: false };
    motion_library: { read: boolean; create: boolean; update: false };
    movement_lifecycle: Array<"prepare" | "release">;
    character_states: Array<"neutral" | "listening" | "thinking">;
    hardware_profile: {
      variant: "7.4 V STS3215";
      encoder_counts_per_revolution: 4096;
      maximum_no_load_speed_rpm: 52;
      maximum_no_load_speed_rad_s: number;
      rated_torque_kg_cm: 5;
      stall_torque_kg_cm: 19.5;
      runtime_control_hz: 50;
      native_profile_registers: true;
    };
  };
}
