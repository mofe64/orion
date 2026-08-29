pub mod calibration;
pub mod daemon;
pub mod driver;
pub mod error;
pub mod motion;
pub mod mujoco;
pub mod pose;
pub mod socket;
pub mod state;
pub mod trajectory;
pub mod transport;

pub use calibration::{JointCalibration, load_calibration_file};
pub use daemon::{OBSERVE_FREQUENCY_HZ, RuntimeCore};
pub use driver::{
    JointServoProfile, RuntimeDriver, ServoProfiles, Sts3215Driver, make_orion_servo_profiles,
};
pub use error::{Error, Result};
pub use motion::{MotionDefinition, MotionKeyframe, MotionLibrary, MotionSequence};
pub use mujoco::{MujocoDriver, SimulationMetrics};
pub use pose::{JointPositions, PoseLibrary};
pub use socket::{UnixCommandServer, request_daemon};
pub use state::{JointState, RuntimeMode, StateSnapshot};
pub use trajectory::JointTrajectory;
pub use transport::{Register, RustypotTransport, Sts3215RawState, Sts3215Transport};

pub const ORION_JOINT_NAMES: [&str; 5] = [
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
];
