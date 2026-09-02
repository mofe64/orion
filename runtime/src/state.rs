use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub const STATE_SCHEMA_VERSION: u32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeMode {
    Observe,
    Configured,
    Holding,
    Moving,
}

impl RuntimeMode {
    pub fn profile_is_applied(self) -> bool {
        self != Self::Observe
    }

    pub fn torque_is_enabled(self) -> bool {
        matches!(self, Self::Holding | Self::Moving)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Configured => "configured",
            Self::Holding => "holding",
            Self::Moving => "moving",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JointState {
    pub name: String,
    #[serde(rename = "position_rad")]
    pub position: f64,
    #[serde(rename = "velocity_rad_s")]
    pub velocity: f64,
    pub current_ma: f64,
    pub voltage_v: f64,
    pub temperature_c: f64,
    pub status: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MovementPhase {
    Executing,
    Settling,
    Completed,
    TimedOut,
    Cancelled,
}

impl MovementPhase {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::TimedOut | Self::Cancelled)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Executing => "executing",
            Self::Settling => "settling",
            Self::Completed => "completed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MotionState {
    pub run_id: u64,
    pub name: String,
    pub state: MovementPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyframe: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyframe_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyframe_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reached_markers: Vec<String>,
    pub progress: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_position_error_rad: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_velocity_rad_s: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StateSnapshot {
    pub schema_version: u32,
    pub robot: &'static str,
    pub build_revision: &'static str,
    pub mode: RuntimeMode,
    pub profile_applied: bool,
    pub torque_enabled: bool,
    pub sequence: u64,
    pub sampled_at_unix_ns: i64,
    pub update_hz: f64,
    pub motion: Option<MotionState>,
    pub last_motion: Option<MotionState>,
    pub joints: Vec<JointState>,
}

impl StateSnapshot {
    pub fn new(
        mode: RuntimeMode,
        sequence: u64,
        update_hz: f64,
        joints: Vec<JointState>,
    ) -> Result<Self> {
        let sampled_at_unix_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| Error::Runtime(format!("System clock precedes Unix epoch: {error}")))?
            .as_nanos()
            .try_into()
            .map_err(|_| Error::Runtime("Unix timestamp does not fit i64 nanoseconds.".into()))?;
        Ok(Self {
            schema_version: STATE_SCHEMA_VERSION,
            robot: "orion",
            build_revision: crate::BUILD_REVISION,
            mode,
            profile_applied: mode.profile_is_applied(),
            torque_enabled: mode.torque_is_enabled(),
            sequence,
            sampled_at_unix_ns,
            update_hz,
            motion: None,
            last_motion: None,
            joints,
        })
    }

    pub fn with_motion(mut self, motion: MotionState) -> Self {
        self.motion = Some(motion);
        self
    }

    pub fn to_json(&self) -> Result<String> {
        if !self.update_hz.is_finite() {
            return Err(Error::Runtime(
                "Cannot serialize non-finite Orion state field: update_hz".into(),
            ));
        }
        if let Some(motion) = &self.motion
            && !motion.progress.is_finite()
        {
            return Err(Error::Runtime(
                "Cannot serialize non-finite Orion state field: motion_progress".into(),
            ));
        }
        for (field, motion) in [
            ("motion", self.motion.as_ref()),
            ("last_motion", self.last_motion.as_ref()),
        ] {
            if let Some(motion) = motion {
                for (measurement, value) in [
                    ("max_position_error_rad", motion.max_position_error_rad),
                    ("max_velocity_rad_s", motion.max_velocity_rad_s),
                ] {
                    if value.is_some_and(|value| !value.is_finite()) {
                        return Err(Error::Runtime(format!(
                            "Cannot serialize non-finite Orion state field: {field}.{measurement}"
                        )));
                    }
                }
            }
        }
        for joint in &self.joints {
            for (field, value) in [
                ("position", joint.position),
                ("velocity", joint.velocity),
                ("current_ma", joint.current_ma),
                ("voltage_v", joint.voltage_v),
                ("temperature_c", joint.temperature_c),
            ] {
                if !value.is_finite() {
                    return Err(Error::Runtime(format!(
                        "Cannot serialize non-finite Orion state field: {}.{field}",
                        joint.name
                    )));
                }
            }
        }
        Ok(serde_json::to_string(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_versioned_observe_mode_contract() {
        let mut snapshot = StateSnapshot::new(
            RuntimeMode::Observe,
            42,
            50.0,
            vec![
                JointState {
                    name: "base_yaw_joint".into(),
                    position: -0.078,
                    velocity: 0.0,
                    current_ma: 0.0,
                    voltage_v: 6.2,
                    temperature_c: 29.0,
                    status: 0,
                },
                JointState {
                    name: "head_\"pitch_joint".into(),
                    position: 0.124,
                    velocity: 0.1,
                    current_ma: 13.0,
                    voltage_v: 6.1,
                    temperature_c: 30.0,
                    status: 2,
                },
            ],
        )
        .unwrap();
        snapshot.sampled_at_unix_ns = 123_456_789;
        let json = snapshot.to_json().unwrap();

        assert!(json.contains("\"schema_version\":3"));
        assert!(json.contains("\"robot\":\"orion\""));
        assert!(json.contains(&format!("\"build_revision\":\"{}\"", crate::BUILD_REVISION)));
        assert!(json.contains("\"mode\":\"observe\""));
        assert!(json.contains("\"profile_applied\":false"));
        assert!(json.contains("\"torque_enabled\":false"));
        assert!(json.contains("\"sequence\":42"));
        assert!(json.contains("\"sampled_at_unix_ns\":123456789"));
        assert!(json.contains("\"motion\":null"));
        assert!(json.contains("\"last_motion\":null"));
        assert!(json.contains("\"name\":\"head_\\\"pitch_joint\""));
        assert!(json.contains("\"position_rad\":-0.078"));
        assert!(json.contains("\"status\":2"));
    }

    #[test]
    fn derives_lifecycle_flags_from_mode() {
        assert!(!RuntimeMode::Observe.profile_is_applied());
        assert!(!RuntimeMode::Observe.torque_is_enabled());
        assert!(RuntimeMode::Configured.profile_is_applied());
        assert!(!RuntimeMode::Configured.torque_is_enabled());
        assert!(RuntimeMode::Holding.torque_is_enabled());
        assert!(RuntimeMode::Moving.torque_is_enabled());
    }

    #[test]
    fn identifies_terminal_movement_phases() {
        assert!(!MovementPhase::Executing.is_terminal());
        assert!(!MovementPhase::Settling.is_terminal());
        assert!(MovementPhase::Completed.is_terminal());
        assert!(MovementPhase::TimedOut.is_terminal());
        assert!(MovementPhase::Cancelled.is_terminal());
    }
}
