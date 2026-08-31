pub mod audio;
pub mod calibration;
pub mod daemon;
pub mod driver;
pub mod error;
pub mod lighting;
pub mod motion;
pub mod mujoco;
pub mod pose;
pub mod scene;
pub mod socket;
pub mod speech;
pub mod state;
pub mod trajectory;
pub mod transport;

pub use audio::{
    AlsaAudioDevice, AudioCommand, AudioDevice, CueLibrary, ORION_AMIXER_PATH, ORION_APLAY_PATH,
    ORION_AUDIO_CARD, ORION_AUDIO_PCM_DEVICE, RecordingAudioDevice, UnavailableAudioDevice,
    configure_respeaker_v2_mixer,
};
pub use calibration::{JointCalibration, load_calibration_file};
pub use daemon::{CompletionCriteria, OBSERVE_FREQUENCY_HZ, RuntimeCore};
pub use driver::{
    JointLimit, JointServoProfile, RuntimeDriver, ServoProfiles, Sts3215Driver,
    make_orion_servo_profiles,
};
pub use error::{Error, Result};
pub use lighting::{
    LightingDevice, ORION_LIGHT_GPIO_BCM, ORION_LIGHT_HEIGHT, ORION_LIGHT_PIXEL_COUNT,
    ORION_LIGHT_WIDTH, ORION_WHITE_TEMPERATURE_K, PI5_NEOPIXEL_DEVICE_PATH, Pi5NeoPixelDevice,
    RecordingLightingDevice, Rgbw8,
};
pub use motion::{MotionDefinition, MotionKeyframe, MotionLibrary, MotionSequence};
pub use mujoco::{MujocoDriver, SimulationMetrics};
pub use pose::{JointPositions, PoseLibrary};
pub use scene::{
    SCENE_FORMAT_VERSION, SceneAction, SceneCoordinator, SceneDefinition, SceneEvent, SceneLibrary,
    SceneMotion, SceneMotionDevice, ScenePhase, ScenePlayer, SceneStatus, parse_scene_document,
};
pub use socket::{UnixCommandServer, request_daemon};
pub use speech::{
    DEFAULT_TTS_SOCKET_PATH, MAX_SPEECH_TEXT_BYTES, SpeechCoordinator, SpeechPhase, SpeechStatus,
};
pub use state::{JointState, MotionState, MovementPhase, RuntimeMode, StateSnapshot};
pub use trajectory::JointTrajectory;
pub use transport::{Register, RustypotTransport, Sts3215RawState, Sts3215Transport};

pub const BUILD_REVISION: &str = env!("ORION_BUILD_REVISION");

pub const ORION_JOINT_NAMES: [&str; 5] = [
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
];
