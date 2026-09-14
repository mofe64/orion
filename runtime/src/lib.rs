pub mod control;
pub mod devices;
pub mod error;
pub mod expression;
pub mod ipc;
pub mod motion;

pub const ORION_JOINT_NAMES: [&str; 5] = [
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
];

pub const BUILD_REVISION: &str = env!("ORION_BUILD_REVISION");

pub mod app;

pub use control::{core as daemon, state};
pub use devices::sts3215::{driver, transport};
pub use devices::{audio, lighting, mujoco};
pub use expression::{character, lamp, scene, speech, voice_feedback};
pub use ipc::socket;
pub use motion::{calibration, pose, style, trajectory};

pub use audio::{
    AlsaAudioDevice, AudioCommand, AudioDevice, CueLibrary, ORION_AMIXER_PATH, ORION_APLAY_PATH,
    ORION_AUDIO_CARD, ORION_AUDIO_PCM_DEVICE, RecordingAudioDevice, UnavailableAudioDevice,
    configure_respeaker_v2_mixer,
};
pub use calibration::{JointCalibration, load_calibration_file};
pub use character::{CharacterCoordinator, CharacterState, CharacterStatus, NextIdleCategory};
pub use daemon::{CompletionCriteria, OBSERVE_FREQUENCY_HZ, RuntimeCore};
pub use driver::{
    JointLimit, JointServoProfile, RuntimeDriver, ServoProfiles, Sts3215Driver,
    make_orion_servo_profiles,
};
pub use error::{Error, Result};
pub use lighting::{
    LIGHTING_EFFECT_NAMES, LightingDevice, ORION_LIGHT_GPIO_BCM, ORION_LIGHT_HEIGHT,
    ORION_LIGHT_PIXEL_COUNT, ORION_LIGHT_WIDTH, ORION_WHITE_TEMPERATURE_K,
    PI5_NEOPIXEL_DEVICE_PATH, Pi5NeoPixelDevice, RecordingLightingDevice, Rgbw8, render_effect,
};
pub use motion::{
    KeyframeArrival, MotionDefinition, MotionKeyframe, MotionLibrary, MotionSequence, MotionSpace,
};
pub use mujoco::{MujocoDriver, SimulationMetrics};
pub use pose::{JointPositions, PoseDefinition, PoseLibrary};
pub use scene::{
    SCENE_FORMAT_VERSION, SceneAction, SceneCoordinator, SceneDefinition, SceneEvent, SceneLibrary,
    SceneMotion, SceneMotionDevice, ScenePhase, ScenePlayer, SceneStatus, parse_scene_document,
};
pub use socket::{UnixCommandServer, request_daemon};
pub use speech::{
    DEFAULT_SPEECH_SPOOL_PATH, MAX_SPEECH_WAV_BYTES, SpeechAnalysis, SpeechCoordinator,
    SpeechPhase, SpeechStatus,
};
pub use state::{JointState, MotionState, MovementPhase, RuntimeMode, StateSnapshot};
pub use style::{MotionStyle, motion_styles};
pub use trajectory::{
    CompiledTrajectory, JointTrajectory, STS3215_MAX_SPEED_RAD_S, TrajectorySample,
    TrajectoryWaypoint, WaypointArrival,
};
pub use transport::{Register, RustypotTransport, Sts3215RawState, Sts3215Transport};
