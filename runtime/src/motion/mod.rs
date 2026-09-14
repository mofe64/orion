// Declare submodules for motion
pub mod calibration;
pub mod library;
pub use library as motion;
pub mod pose;
mod shared;
pub mod style;
pub mod trajectory;

// Make frequently used types accessible directly through motion module
pub use pose::{JointPositions, PoseDefinition, PoseLibrary};
pub use trajectory::{CompiledTrajectory, TrajectoryWaypoint, WaypointArrival};

pub use library::{
    KeyframeArrival, MOTION_FORMAT_VERSION, MotionDefinition, MotionKeyframe, MotionLibrary,
    MotionSequence, MotionSpace,
};
