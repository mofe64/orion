use serde::Serialize;

use crate::control::state::JointState;
use crate::error::Result;
use crate::motion::pose::JointPositions;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JointLimit {
    pub name: String,
    pub lower_rad: f64,
    pub upper_rad: f64,
}

/// The hardware-facing operations used by the daemon state machine.
/// Keeping this boundary in joint-space lets the same daemon logic drive the
/// real STS3215 bus, deterministic tests, and the MuJoCo validation backend.
pub trait RuntimeDriver {
    fn apply_servo_profile(&mut self) -> Result<()>;
    fn activate(&mut self) -> Result<Vec<JointState>>;
    fn deactivate(&mut self) -> Result<()>;
    fn read(&mut self) -> Result<Vec<JointState>>;
    fn write(&mut self, positions_radians: &JointPositions) -> Result<()>;
    fn joint_limits(&self) -> Result<Vec<JointLimit>>;
    fn validate_positions(&self, positions_radians: &JointPositions) -> Result<()>;
    fn clamp_positions_to_safe_range(
        &self,
        positions_radians: &JointPositions,
    ) -> Result<JointPositions>;
}
