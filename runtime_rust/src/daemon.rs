use serde_json::json;

use crate::driver::RuntimeDriver;
use crate::motion::{MotionLibrary, MotionSequence};
use crate::pose::{JointPositions, PoseLibrary};
use crate::state::{MotionState, RuntimeMode, StateSnapshot};
use crate::trajectory::JointTrajectory;
use crate::{Error, Result};

pub const OBSERVE_FREQUENCY_HZ: f64 = 50.0;

/// Hardware-independent implementation of the C++ `oriond` command and
/// movement state machine. Time is supplied by the caller for deterministic
/// tests and by the service loop from a monotonic clock.
pub struct RuntimeCore<D: RuntimeDriver> {
    driver: D,
    poses: PoseLibrary,
    motions: MotionLibrary,
    mode: RuntimeMode,
    trajectory: Option<JointTrajectory>,
    motion_sequence: Option<MotionSequence>,
    movement_started_at: f64,
    sequence: u64,
    snapshot: StateSnapshot,
}

impl<D: RuntimeDriver> RuntimeCore<D> {
    pub fn new(mut driver: D, poses: PoseLibrary, motions: MotionLibrary) -> Result<Self> {
        let snapshot = StateSnapshot::new(
            RuntimeMode::Observe,
            1,
            OBSERVE_FREQUENCY_HZ,
            driver.read()?,
        )?;
        Ok(Self {
            driver,
            poses,
            motions,
            mode: RuntimeMode::Observe,
            trajectory: None,
            motion_sequence: None,
            movement_started_at: 0.0,
            sequence: 1,
            snapshot,
        })
    }

    pub fn tick(&mut self, now_seconds: f64) -> Result<()> {
        if !now_seconds.is_finite() {
            return Err(Error::InvalidArgument(
                "Runtime time must be finite.".into(),
            ));
        }
        // Preserve C++ behavior: telemetry is sampled before this cycle's goal
        // is written, then published with the movement metadata for this tick.
        let states = self.driver.read()?;
        let mut motion = None;
        let elapsed = now_seconds - self.movement_started_at;

        if let Some(sequence) = &self.motion_sequence {
            self.driver.write(&sequence.sample(elapsed)?)?;
            motion = Some(MotionState {
                name: sequence.name().to_owned(),
                keyframe: Some(sequence.keyframe_name(elapsed)?.to_owned()),
                keyframe_index: Some(sequence.keyframe_index(elapsed)?),
                keyframe_count: Some(sequence.keyframe_count()),
                progress: sequence.progress(elapsed)?,
            });
            if sequence.complete(elapsed)? {
                self.motion_sequence = None;
                self.mode = RuntimeMode::Holding;
                motion = None;
            }
        } else if let Some(trajectory) = &self.trajectory {
            self.driver.write(&trajectory.sample(elapsed)?)?;
            motion = Some(MotionState {
                name: trajectory.name().to_owned(),
                keyframe: None,
                keyframe_index: None,
                keyframe_count: None,
                progress: trajectory.progress(elapsed)?,
            });
            if trajectory.complete(elapsed)? {
                self.trajectory = None;
                self.mode = RuntimeMode::Holding;
                motion = None;
            }
        }

        self.sequence += 1;
        self.snapshot = StateSnapshot::new(self.mode, self.sequence, OBSERVE_FREQUENCY_HZ, states)?;
        self.snapshot.motion = motion;
        Ok(())
    }

    pub fn handle_command(&mut self, command: &str, now_seconds: f64) -> String {
        match self.handle_command_inner(command.trim(), now_seconds) {
            Ok(response) => response,
            Err(error) => json!({"ok": false, "error": error.to_string()}).to_string(),
        }
    }

    fn handle_command_inner(&mut self, command: &str, now_seconds: f64) -> Result<String> {
        if command == "status" {
            return self.snapshot.to_json();
        }
        if command == "configure" {
            if self.mode.torque_is_enabled() {
                return Ok(command_error(
                    "configure",
                    "disable torque before configuring",
                ));
            }
            self.driver.apply_servo_profile()?;
            self.mode = RuntimeMode::Configured;
            self.refresh_snapshot()?;
            return Ok(
                json!({"ok": true, "command": "configure", "mode": "configured"}).to_string(),
            );
        }
        if command == "enable" {
            if self.mode != RuntimeMode::Configured {
                return Ok(command_error("enable", "configure before enabling torque"));
            }
            let states = self.driver.activate()?;
            self.mode = RuntimeMode::Holding;
            self.sequence += 1;
            self.snapshot =
                StateSnapshot::new(self.mode, self.sequence, OBSERVE_FREQUENCY_HZ, states)?;
            return Ok(json!({"ok": true, "command": "enable", "mode": "holding"}).to_string());
        }
        if command == "disable" {
            if !self.mode.torque_is_enabled() {
                return Ok(command_error("disable", "torque is not enabled"));
            }
            self.trajectory = None;
            self.motion_sequence = None;
            self.driver.deactivate()?;
            self.mode = RuntimeMode::Configured;
            self.refresh_snapshot()?;
            return Ok(json!({"ok": true, "command": "disable", "mode": "configured"}).to_string());
        }
        if let Some(arguments) = command.strip_prefix("goto ") {
            if self.mode != RuntimeMode::Holding {
                return Ok(command_error("goto", "enable holding torque before moving"));
            }
            let fields: Vec<_> = arguments.split_whitespace().collect();
            if fields.len() != 2 {
                return Ok(command_error("goto", "expected goto POSE SECONDS"));
            }
            let duration: f64 = match fields[1].parse() {
                Ok(value) => value,
                Err(_) => return Ok(command_error("goto", "expected goto POSE SECONDS")),
            };
            let target = self.poses.pose(fields[0])?.clone();
            self.driver.validate_positions(&target)?;
            let start = self.measured_positions();
            self.trajectory = Some(JointTrajectory::new(
                fields[0],
                self.driver.clamp_positions_to_safe_range(&start)?,
                target,
                duration,
            )?);
            self.motion_sequence = None;
            self.movement_started_at = now_seconds;
            self.mode = RuntimeMode::Moving;
            return Ok(format!(
                "{{\"ok\":true,\"command\":\"goto\",\"pose\":{},\"mode\":\"moving\",\"duration_seconds\":{duration:.6}}}",
                serde_json::to_string(fields[0])?
            ));
        }
        if let Some(arguments) = command.strip_prefix("play ") {
            if self.mode != RuntimeMode::Holding {
                return Ok(command_error("play", "enable holding torque before moving"));
            }
            let fields: Vec<_> = arguments.split_whitespace().collect();
            if fields.len() != 1 {
                return Ok(command_error("play", "expected play MOTION"));
            }
            let definition = self.motions.motion(fields[0])?;
            for keyframe in &definition.keyframes {
                self.driver.validate_positions(&keyframe.target)?;
            }
            let start = self.measured_positions();
            let sequence = MotionSequence::new(
                definition,
                self.driver.clamp_positions_to_safe_range(&start)?,
            )?;
            let duration = sequence.duration_seconds();
            let keyframes = sequence.keyframe_count();
            self.motion_sequence = Some(sequence);
            self.trajectory = None;
            self.movement_started_at = now_seconds;
            self.mode = RuntimeMode::Moving;
            return Ok(format!(
                "{{\"ok\":true,\"command\":\"play\",\"motion\":{},\"mode\":\"moving\",\"duration_seconds\":{duration:.6},\"keyframes\":{keyframes}}}",
                serde_json::to_string(fields[0])?
            ));
        }
        if command == "stop" {
            if self.mode != RuntimeMode::Moving {
                return Ok(command_error("stop", "no movement is active"));
            }
            self.trajectory = None;
            self.motion_sequence = None;
            self.mode = RuntimeMode::Holding;
            self.refresh_snapshot()?;
            return Ok(json!({"ok": true, "command": "stop", "mode": "holding"}).to_string());
        }
        Ok(json!({"ok": false, "error": "unknown Orion daemon command"}).to_string())
    }

    fn measured_positions(&self) -> JointPositions {
        self.snapshot
            .joints
            .iter()
            .map(|joint| (joint.name.clone(), joint.position))
            .collect()
    }

    fn refresh_snapshot(&mut self) -> Result<()> {
        self.sequence += 1;
        self.snapshot = StateSnapshot::new(
            self.mode,
            self.sequence,
            OBSERVE_FREQUENCY_HZ,
            self.driver.read()?,
        )?;
        Ok(())
    }

    pub fn mode(&self) -> RuntimeMode {
        self.mode
    }

    pub fn snapshot(&self) -> &StateSnapshot {
        &self.snapshot
    }

    pub fn driver(&self) -> &D {
        &self.driver
    }
}

fn command_error(command: &str, error: &str) -> String {
    json!({"ok": false, "command": command, "error": error}).to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::ORION_JOINT_NAMES;
    use crate::state::JointState;

    struct FakeDriver {
        positions: JointPositions,
        writes: Vec<JointPositions>,
        active: bool,
        configured: bool,
    }

    impl FakeDriver {
        fn new() -> Self {
            Self {
                positions: ORION_JOINT_NAMES
                    .iter()
                    .map(|name| ((*name).to_owned(), 0.0))
                    .collect(),
                writes: Vec::new(),
                active: false,
                configured: false,
            }
        }
    }

    impl RuntimeDriver for FakeDriver {
        fn apply_servo_profile(&mut self) -> Result<()> {
            self.configured = true;
            Ok(())
        }
        fn activate(&mut self) -> Result<Vec<JointState>> {
            if !self.configured {
                return Err(Error::InvalidState("profile required".into()));
            }
            self.active = true;
            self.read()
        }
        fn deactivate(&mut self) -> Result<()> {
            self.active = false;
            Ok(())
        }
        fn read(&mut self) -> Result<Vec<JointState>> {
            Ok(self
                .positions
                .iter()
                .map(|(name, position)| JointState {
                    name: name.clone(),
                    position: *position,
                    velocity: 0.0,
                    current_ma: 0.0,
                    voltage_v: 6.2,
                    temperature_c: 25.0,
                    status: 0,
                })
                .collect())
        }
        fn write(&mut self, positions: &JointPositions) -> Result<()> {
            if !self.active {
                return Err(Error::InvalidState("inactive".into()));
            }
            self.positions = positions.clone();
            self.writes.push(positions.clone());
            Ok(())
        }
        fn validate_positions(&self, positions: &JointPositions) -> Result<()> {
            if positions.len() != ORION_JOINT_NAMES.len()
                || positions
                    .values()
                    .any(|value| !value.is_finite() || value.abs() > 3.0)
            {
                return Err(Error::OutOfRange("unsafe test position".into()));
            }
            Ok(())
        }
        fn clamp_positions_to_safe_range(
            &self,
            positions: &JointPositions,
        ) -> Result<JointPositions> {
            Ok(positions
                .iter()
                .map(|(name, value)| (name.clone(), value.clamp(-3.0, 3.0)))
                .collect::<BTreeMap<_, _>>())
        }
    }

    fn core() -> RuntimeCore<FakeDriver> {
        let root = env!("CARGO_MANIFEST_DIR");
        let poses = PoseLibrary::load(
            format!("{root}/../motion/config/poses.yaml"),
            &ORION_JOINT_NAMES,
        )
        .unwrap();
        let motions = MotionLibrary::load(format!("{root}/../motion/motions"), &poses).unwrap();
        RuntimeCore::new(FakeDriver::new(), poses, motions).unwrap()
    }

    #[test]
    fn enforces_configuration_and_torque_lifecycle() {
        let mut core = core();
        assert!(
            core.handle_command("enable", 0.0)
                .contains("configure before")
        );
        assert_eq!(core.mode(), RuntimeMode::Observe);
        assert!(
            core.handle_command("configure", 0.0)
                .contains("\"ok\":true")
        );
        assert_eq!(core.mode(), RuntimeMode::Configured);
        assert!(core.handle_command("enable", 0.0).contains("\"ok\":true"));
        assert_eq!(core.mode(), RuntimeMode::Holding);
        assert!(
            core.handle_command("configure", 0.0)
                .contains("disable torque")
        );
        assert!(core.handle_command("disable", 0.0).contains("\"ok\":true"));
        assert_eq!(core.mode(), RuntimeMode::Configured);
    }

    #[test]
    fn runs_goto_with_cpp_quintic_semantics() {
        let mut core = core();
        core.handle_command("configure", 0.0);
        core.handle_command("enable", 0.0);
        let response = core.handle_command("goto home 2", 10.0);
        assert!(response.contains("\"duration_seconds\":2.000000"));
        assert_eq!(core.mode(), RuntimeMode::Moving);
        core.tick(11.0).unwrap();
        assert_eq!(core.snapshot().motion.as_ref().unwrap().progress, 0.5);
        core.tick(12.0).unwrap();
        assert_eq!(core.mode(), RuntimeMode::Holding);
        assert!(core.snapshot().motion.is_none());
        assert_eq!(core.driver().writes.len(), 2);
    }

    #[test]
    fn runs_authored_motion_and_reports_keyframes() {
        let mut core = core();
        core.handle_command("configure", 0.0);
        core.handle_command("enable", 0.0);
        let response = core.handle_command("play look_at_right", 5.0);
        assert!(response.contains("\"keyframes\":1"));
        core.tick(5.25).unwrap();
        let motion = core.snapshot().motion.as_ref().unwrap();
        assert_eq!(motion.name, "look_at_right");
        assert_eq!(motion.keyframe_index, Some(0));
        assert_eq!(motion.keyframe_count, Some(1));
        assert!(core.handle_command("stop", 5.25).contains("\"ok\":true"));
        assert_eq!(core.mode(), RuntimeMode::Holding);
    }

    #[test]
    fn returns_protocol_errors_without_changing_mode() {
        let mut core = core();
        assert!(
            core.handle_command("goto home", 0.0)
                .contains("enable holding")
        );
        assert!(
            core.handle_command("nonsense", 0.0)
                .contains("unknown Orion")
        );
        assert_eq!(core.mode(), RuntimeMode::Observe);
    }
}
