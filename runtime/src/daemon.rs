use serde_json::json;

use crate::driver::RuntimeDriver;
use crate::motion::{MotionDefinition, MotionLibrary, MotionSequence};
use crate::pose::{JointPositions, PoseLibrary};
use crate::state::{JointState, MotionState, MovementPhase, RuntimeMode, StateSnapshot};
use crate::trajectory::JointTrajectory;
use crate::{Error, Result};

pub const OBSERVE_FREQUENCY_HZ: f64 = 50.0;
pub const COMPLETION_POSITION_TOLERANCE_RAD: f64 = 0.05;
pub const COMPLETION_VELOCITY_TOLERANCE_RAD_S: f64 = 0.05;
pub const COMPLETION_SETTLE_DURATION_SECONDS: f64 = 0.25;
pub const COMPLETION_SETTLE_TIMEOUT_SECONDS: f64 = 2.0;

#[derive(Clone, Copy, Debug)]
pub struct CompletionCriteria {
    pub position_tolerance_rad: f64,
    pub velocity_tolerance_rad_s: f64,
    pub settle_duration_seconds: f64,
    pub settle_timeout_seconds: f64,
}

impl Default for CompletionCriteria {
    fn default() -> Self {
        Self {
            position_tolerance_rad: COMPLETION_POSITION_TOLERANCE_RAD,
            velocity_tolerance_rad_s: COMPLETION_VELOCITY_TOLERANCE_RAD_S,
            settle_duration_seconds: COMPLETION_SETTLE_DURATION_SECONDS,
            settle_timeout_seconds: COMPLETION_SETTLE_TIMEOUT_SECONDS,
        }
    }
}

struct ActiveMovement {
    status: MotionState,
    target: JointPositions,
    settling_started_at: Option<f64>,
    within_tolerance_since: Option<f64>,
}

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
    completion: CompletionCriteria,
    next_run_id: u64,
    active_movement: Option<ActiveMovement>,
    last_movement: Option<MotionState>,
    sequence: u64,
    snapshot: StateSnapshot,
}

impl<D: RuntimeDriver> RuntimeCore<D> {
    pub fn new(driver: D, poses: PoseLibrary, motions: MotionLibrary) -> Result<Self> {
        Self::with_completion_criteria(driver, poses, motions, CompletionCriteria::default())
    }

    pub fn with_completion_criteria(
        mut driver: D,
        poses: PoseLibrary,
        motions: MotionLibrary,
        completion: CompletionCriteria,
    ) -> Result<Self> {
        validate_completion_criteria(completion)?;
        validate_motion_assets(&driver, &poses, &motions)?;
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
            completion,
            next_run_id: 1,
            active_movement: None,
            last_movement: None,
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
        let elapsed = now_seconds - self.movement_started_at;
        let mut entered_settling = false;

        if let Some(sequence) = &self.motion_sequence {
            self.driver.write(&sequence.sample(elapsed)?)?;
            let active = self.active_movement.as_mut().ok_or_else(|| {
                Error::Runtime("Motion sequence has no active movement lifecycle.".into())
            })?;
            active.status.keyframe = Some(sequence.keyframe_name(elapsed)?.to_owned());
            active.status.keyframe_index = Some(sequence.keyframe_index(elapsed)?);
            active.status.keyframe_count = Some(sequence.keyframe_count());
            active.status.reached_markers = sequence.reached_markers(elapsed);
            active.status.progress = sequence.progress(elapsed)?;
            if sequence.complete(elapsed)? {
                self.motion_sequence = None;
                entered_settling = true;
            }
        } else if let Some(trajectory) = &self.trajectory {
            self.driver.write(&trajectory.sample(elapsed)?)?;
            let active = self.active_movement.as_mut().ok_or_else(|| {
                Error::Runtime("Joint trajectory has no active movement lifecycle.".into())
            })?;
            active.status.progress = trajectory.progress(elapsed)?;
            if trajectory.complete(elapsed)? {
                self.trajectory = None;
                entered_settling = true;
            }
        }

        if entered_settling {
            self.enter_settling(now_seconds)?;
        } else if self
            .active_movement
            .as_ref()
            .is_some_and(|movement| matches!(movement.status.state, MovementPhase::Settling))
        {
            self.update_settling(now_seconds, &states)?;
        }

        self.sequence += 1;
        self.publish_snapshot(states)?;
        Ok(())
    }

    pub fn handle_command(&mut self, command: &str, now_seconds: f64) -> String {
        match self.handle_command_inner(command.trim(), now_seconds) {
            Ok(response) => response,
            Err(error) => json!({"ok": false, "error": error.to_string()}).to_string(),
        }
    }

    fn handle_command_inner(&mut self, command: &str, now_seconds: f64) -> Result<String> {
        if command == "pose list" {
            return Ok(json!({"ok": true, "poses": self.poses.names()}).to_string());
        }
        if command == "motion list" {
            return Ok(json!({"ok": true, "motions": self.motions.names()}).to_string());
        }
        if command == "joint limits" {
            return Ok(json!({"ok": true, "joints": self.driver.joint_limits()?}).to_string());
        }
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
            self.publish_snapshot(states)?;
            return Ok(json!({"ok": true, "command": "enable", "mode": "holding"}).to_string());
        }
        if command == "disable" {
            if !self.mode.torque_is_enabled() {
                return Ok(command_error("disable", "torque is not enabled"));
            }
            if self.active_movement.is_some() {
                self.finish_active_movement(MovementPhase::Cancelled)?;
            }
            self.trajectory = None;
            self.motion_sequence = None;
            self.driver.deactivate()?;
            self.mode = RuntimeMode::Configured;
            self.refresh_snapshot()?;
            return Ok(json!({"ok": true, "command": "disable", "mode": "configured"}).to_string());
        }
        if let Some(arguments) = command.strip_prefix("goto ") {
            if self.mode == RuntimeMode::Moving {
                return Ok(self.busy_error("goto"));
            }
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
            let limits = self.driver.joint_limits()?;
            self.trajectory = Some(JointTrajectory::with_start_velocity_calibrated(
                fields[0],
                self.driver.clamp_positions_to_safe_range(&start)?,
                self.measured_velocities(),
                target,
                duration,
                &limits,
            )?);
            self.motion_sequence = None;
            self.movement_started_at = now_seconds;
            self.mode = RuntimeMode::Moving;
            let run_id = self.begin_movement(fields[0], self.poses.pose(fields[0])?.clone())?;
            return Ok(format!(
                "{{\"ok\":true,\"command\":\"goto\",\"run_id\":{run_id},\"pose\":{},\"state\":\"executing\",\"mode\":\"moving\",\"duration_seconds\":{duration:.6}}}",
                serde_json::to_string(fields[0])?
            ));
        }
        if let Some(arguments) = command.strip_prefix("play ") {
            if self.mode == RuntimeMode::Moving {
                return Ok(self.busy_error("play"));
            }
            if self.mode != RuntimeMode::Holding {
                return Ok(command_error("play", "enable holding torque before moving"));
            }
            let fields: Vec<_> = arguments.split_whitespace().collect();
            if fields.len() != 1 {
                return Ok(command_error("play", "expected play MOTION"));
            }
            let definition = self.motions.motion(fields[0])?.clone();
            let start = self
                .driver
                .clamp_positions_to_safe_range(&self.measured_positions())?;
            let start_velocity = self.measured_velocities();
            let limits = self.driver.joint_limits()?;
            let amplitude_scale = definition.uniform_amplitude_scale(&start, &limits)?;
            let targets = definition.resolved_targets_with_scale(&start, amplitude_scale)?;
            for target in &targets {
                self.driver.validate_positions(target)?;
            }
            let sequence = MotionSequence::compile_scaled_calibrated(
                &definition,
                start.clone(),
                start_velocity,
                start,
                amplitude_scale,
                &limits,
            )?;
            let duration = sequence.duration_seconds();
            let keyframes = sequence.keyframe_count();
            let target = targets
                .last()
                .cloned()
                .ok_or_else(|| Error::InvalidArgument("Motion has no final keyframe.".into()))?;
            self.motion_sequence = Some(sequence);
            self.trajectory = None;
            self.movement_started_at = now_seconds;
            self.mode = RuntimeMode::Moving;
            let run_id = self.begin_movement(fields[0], target)?;
            return Ok(format!(
                "{{\"ok\":true,\"command\":\"play\",\"run_id\":{run_id},\"motion\":{},\"state\":\"executing\",\"mode\":\"moving\",\"duration_seconds\":{duration:.6},\"keyframes\":{keyframes}}}",
                serde_json::to_string(fields[0])?
            ));
        }
        if command == "stop" {
            if self.mode != RuntimeMode::Moving {
                return Ok(command_error("stop", "no movement is active"));
            }
            self.trajectory = None;
            self.motion_sequence = None;
            self.finish_active_movement(MovementPhase::Cancelled)?;
            self.refresh_snapshot()?;
            return Ok(json!({
                "ok": true,
                "command": "stop",
                "mode": "holding",
                "last_motion": self.last_movement.as_ref(),
            })
            .to_string());
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

    fn measured_velocities(&self) -> JointPositions {
        self.snapshot
            .joints
            .iter()
            .map(|joint| (joint.name.clone(), joint.velocity))
            .collect()
    }

    fn refresh_snapshot(&mut self) -> Result<()> {
        self.sequence += 1;
        let states = self.driver.read()?;
        self.publish_snapshot(states)
    }

    fn begin_movement(&mut self, name: &str, target: JointPositions) -> Result<u64> {
        let run_id = self.next_run_id;
        self.next_run_id = self
            .next_run_id
            .checked_add(1)
            .ok_or_else(|| Error::Runtime("Orion movement run ID overflowed.".into()))?;
        self.active_movement = Some(ActiveMovement {
            status: MotionState {
                run_id,
                name: name.to_owned(),
                state: MovementPhase::Executing,
                keyframe: None,
                keyframe_index: None,
                keyframe_count: None,
                reached_markers: Vec::new(),
                progress: 0.0,
                max_position_error_rad: None,
                max_velocity_rad_s: None,
            },
            target,
            settling_started_at: None,
            within_tolerance_since: None,
        });
        self.sync_lifecycle_to_snapshot();
        Ok(run_id)
    }

    fn enter_settling(&mut self, now_seconds: f64) -> Result<()> {
        let movement = self.active_movement.as_mut().ok_or_else(|| {
            Error::Runtime("Cannot settle without an active movement lifecycle.".into())
        })?;
        movement.status.state = MovementPhase::Settling;
        movement.status.progress = 1.0;
        movement.status.keyframe = None;
        movement.status.keyframe_index = None;
        movement.settling_started_at = Some(now_seconds);
        movement.within_tolerance_since = None;
        Ok(())
    }

    fn update_settling(&mut self, now_seconds: f64, states: &[JointState]) -> Result<()> {
        let terminal = {
            let movement = self.active_movement.as_mut().ok_or_else(|| {
                Error::Runtime("Cannot update settling without an active movement.".into())
            })?;
            let (max_position_error, max_velocity) = final_measurements(states, &movement.target)?;
            movement.status.max_position_error_rad = Some(max_position_error);
            movement.status.max_velocity_rad_s = Some(max_velocity);

            let within_tolerance = max_position_error <= self.completion.position_tolerance_rad
                && max_velocity <= self.completion.velocity_tolerance_rad_s;
            let settling_started_at = movement
                .settling_started_at
                .ok_or_else(|| Error::Runtime("Settling movement has no start time.".into()))?;
            let settled = if within_tolerance {
                let within_since = movement.within_tolerance_since.get_or_insert(now_seconds);
                now_seconds - *within_since >= self.completion.settle_duration_seconds
            } else {
                movement.within_tolerance_since = None;
                false
            };
            if settled {
                Some(MovementPhase::Completed)
            } else if now_seconds - settling_started_at >= self.completion.settle_timeout_seconds {
                Some(MovementPhase::TimedOut)
            } else {
                None
            }
        };

        if let Some(terminal) = terminal {
            self.finish_active_movement(terminal)?;
        }
        Ok(())
    }

    fn finish_active_movement(&mut self, phase: MovementPhase) -> Result<()> {
        if !phase.is_terminal() {
            return Err(Error::InvalidArgument(
                "Movement can only finish in a terminal state.".into(),
            ));
        }
        let mut movement = self.active_movement.take().ok_or_else(|| {
            Error::Runtime("Cannot finish without an active movement lifecycle.".into())
        })?;
        movement.status.state = phase;
        self.last_movement = Some(movement.status);
        self.mode = RuntimeMode::Holding;
        self.sync_lifecycle_to_snapshot();
        Ok(())
    }

    fn busy_error(&self, command: &str) -> String {
        let run_id = self
            .active_movement
            .as_ref()
            .map(|movement| movement.status.run_id);
        json!({
            "ok": false,
            "command": command,
            "error": "motion already active",
            "active_run_id": run_id,
        })
        .to_string()
    }

    fn publish_snapshot(&mut self, states: Vec<JointState>) -> Result<()> {
        self.snapshot = StateSnapshot::new(self.mode, self.sequence, OBSERVE_FREQUENCY_HZ, states)?;
        self.sync_lifecycle_to_snapshot();
        Ok(())
    }

    fn sync_lifecycle_to_snapshot(&mut self) {
        self.snapshot.mode = self.mode;
        self.snapshot.profile_applied = self.mode.profile_is_applied();
        self.snapshot.torque_enabled = self.mode.torque_is_enabled();
        self.snapshot.motion = self
            .active_movement
            .as_ref()
            .map(|movement| movement.status.clone());
        self.snapshot.last_motion = self.last_movement.clone();
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

    pub fn poses(&self) -> &PoseLibrary {
        &self.poses
    }

    pub fn motions(&self) -> &MotionLibrary {
        &self.motions
    }

    /// Start a character-owned relative clip around an immutable anchor. The
    /// measured state still supplies the interruption blend's position and
    /// velocity, while every authored offset is resolved from `anchor`.
    pub fn play_anchored_relative(
        &mut self,
        name: &str,
        anchor: JointPositions,
        now_seconds: f64,
    ) -> Result<u64> {
        let definition = self.motions.motion(name)?.clone();
        self.play_generated_anchored_relative(definition, anchor, now_seconds)
    }

    /// Compile and start a character-owned relative performance assembled at
    /// runtime. Speech uses this to turn phrase gestures into one continuous
    /// spline while retaining the same calibration and anchor guarantees as
    /// authored relative clips.
    pub fn play_generated_anchored_relative(
        &mut self,
        definition: MotionDefinition,
        anchor: JointPositions,
        now_seconds: f64,
    ) -> Result<u64> {
        if self.mode != RuntimeMode::Holding || self.active_movement.is_some() {
            return Err(Error::InvalidState(
                "Anchored character motion requires idle holding torque.".into(),
            ));
        }
        if definition.space != crate::motion::MotionSpace::AnchorRelative
            || !definition.return_to_anchor
        {
            return Err(Error::InvalidArgument(format!(
                "Character clip '{}' must be anchor-relative and return to anchor.",
                definition.name
            )));
        }
        let final_keyframe = definition.keyframes.last().ok_or_else(|| {
            Error::InvalidArgument(format!(
                "Character clip '{}' has no final keyframe.",
                definition.name
            ))
        })?;
        if final_keyframe.arrival != crate::motion::KeyframeArrival::Settle
            || final_keyframe
                .target
                .values()
                .any(|offset| offset.abs() > 1e-12)
        {
            return Err(Error::InvalidArgument(format!(
                "Character clip '{}' must finish with one zero-offset settle.",
                definition.name
            )));
        }
        let start = self
            .driver
            .clamp_positions_to_safe_range(&self.measured_positions())?;
        let limits = self.driver.joint_limits()?;
        let amplitude_scale = definition.uniform_amplitude_scale(&anchor, &limits)?;
        let targets = definition.resolved_targets_with_scale(&anchor, amplitude_scale)?;
        for target in &targets {
            self.driver.validate_positions(target)?;
        }
        let sequence = MotionSequence::compile_scaled_calibrated(
            &definition,
            start,
            self.measured_velocities(),
            anchor,
            amplitude_scale,
            &limits,
        )?;
        let target = targets
            .last()
            .cloned()
            .ok_or_else(|| Error::InvalidArgument("Character clip has no final target.".into()))?;
        self.motion_sequence = Some(sequence);
        self.trajectory = None;
        self.movement_started_at = now_seconds;
        self.mode = RuntimeMode::Moving;
        self.begin_movement(&definition.name, target)
    }

    pub fn replace_motion_assets(
        &mut self,
        poses: PoseLibrary,
        motions: MotionLibrary,
    ) -> Result<()> {
        if self.active_movement.is_some() || self.mode == RuntimeMode::Moving {
            return Err(Error::InvalidState(
                "Cannot reload pose or motion assets while movement is active.".into(),
            ));
        }
        validate_motion_assets(&self.driver, &poses, &motions)?;
        self.poses = poses;
        self.motions = motions;
        Ok(())
    }
}

fn validate_motion_assets<D: RuntimeDriver>(
    driver: &D,
    poses: &PoseLibrary,
    motions: &MotionLibrary,
) -> Result<()> {
    for (name, positions) in poses.iter() {
        driver.validate_positions(positions).map_err(|error| {
            Error::OutOfRange(format!(
                "Pose '{name}' is outside the active driver limits: {error}"
            ))
        })?;
    }
    for (name, motion) in motions.iter() {
        if motion.space == crate::motion::MotionSpace::AnchorRelative {
            continue;
        }
        for keyframe in &motion.keyframes {
            driver.validate_positions(&keyframe.target).map_err(|error| {
                Error::OutOfRange(format!(
                    "Motion '{name}' keyframe '{}' is outside the active driver limits: {error}",
                    keyframe.pose_name.as_deref().unwrap_or("relative")
                ))
            })?;
        }
    }
    Ok(())
}

fn validate_completion_criteria(criteria: CompletionCriteria) -> Result<()> {
    let values = [
        criteria.position_tolerance_rad,
        criteria.velocity_tolerance_rad_s,
        criteria.settle_duration_seconds,
        criteria.settle_timeout_seconds,
    ];
    if values
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
        || criteria.settle_timeout_seconds < criteria.settle_duration_seconds
    {
        return Err(Error::InvalidArgument(
            "Completion tolerances and durations must be finite, non-negative, and the timeout must cover the settle duration."
                .into(),
        ));
    }
    Ok(())
}

fn final_measurements(states: &[JointState], target: &JointPositions) -> Result<(f64, f64)> {
    let mut max_position_error: f64 = 0.0;
    let mut max_velocity: f64 = 0.0;
    for (name, target_position) in target {
        let state = states
            .iter()
            .find(|state| state.name == name.as_str())
            .ok_or_else(|| Error::Runtime(format!("Missing measured state for joint: {name}")))?;
        max_position_error = max_position_error.max((state.position - target_position).abs());
        max_velocity = max_velocity.max(state.velocity.abs());
    }
    Ok((max_position_error, max_velocity))
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
        follow_writes: bool,
        velocity_rad_s: f64,
    }

    impl FakeDriver {
        fn new() -> Self {
            Self::with_feedback(true)
        }

        fn stuck() -> Self {
            Self::with_feedback(false)
        }

        fn with_feedback(follow_writes: bool) -> Self {
            Self {
                positions: ORION_JOINT_NAMES
                    .iter()
                    .map(|name| ((*name).to_owned(), 0.0))
                    .collect(),
                writes: Vec::new(),
                active: false,
                configured: false,
                follow_writes,
                velocity_rad_s: 0.0,
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
                    velocity: self.velocity_rad_s,
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
            if self.follow_writes {
                self.positions = positions.clone();
            }
            self.writes.push(positions.clone());
            Ok(())
        }
        fn joint_limits(&self) -> Result<Vec<crate::driver::JointLimit>> {
            Ok(ORION_JOINT_NAMES
                .iter()
                .map(|name| crate::driver::JointLimit {
                    name: (*name).to_owned(),
                    lower_rad: -3.0,
                    upper_rad: 3.0,
                })
                .collect())
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

    fn core_with_driver(driver: FakeDriver) -> RuntimeCore<FakeDriver> {
        let root = env!("CARGO_MANIFEST_DIR");
        let poses = PoseLibrary::load(
            format!("{root}/../motion/config/poses.yaml"),
            &ORION_JOINT_NAMES,
        )
        .unwrap();
        let motions = MotionLibrary::load(format!("{root}/../motion/motions"), &poses).unwrap();
        RuntimeCore::new(driver, poses, motions).unwrap()
    }

    fn core() -> RuntimeCore<FakeDriver> {
        core_with_driver(FakeDriver::new())
    }

    fn activate(core: &mut RuntimeCore<FakeDriver>) {
        core.handle_command("configure", 0.0);
        core.handle_command("enable", 0.0);
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
    fn lists_the_loaded_semantic_motion_assets() {
        let mut core = core();
        let poses: serde_json::Value =
            serde_json::from_str(&core.handle_command("pose list", 0.0)).unwrap();
        let motions: serde_json::Value =
            serde_json::from_str(&core.handle_command("motion list", 0.0)).unwrap();
        let limits: serde_json::Value =
            serde_json::from_str(&core.handle_command("joint limits", 0.0)).unwrap();

        assert_eq!(poses["ok"], true);
        assert!(
            poses["poses"]
                .as_array()
                .unwrap()
                .contains(&serde_json::Value::String("home".into()))
        );
        assert_eq!(motions["ok"], true);
        assert!(
            motions["motions"]
                .as_array()
                .unwrap()
                .contains(&serde_json::Value::String("look_at_left".into()))
        );
        assert_eq!(
            limits["joints"].as_array().unwrap().len(),
            ORION_JOINT_NAMES.len()
        );
        assert_eq!(limits["joints"][0]["name"], "base_yaw_joint");
        assert_eq!(limits["joints"][0]["lower_rad"], -3.0);
        assert_eq!(limits["joints"][0]["upper_rad"], 3.0);
    }

    #[test]
    fn assigns_run_id_and_completes_only_after_measured_settling() {
        let mut core = core();
        activate(&mut core);
        let response = core.handle_command("goto home 2", 10.0);
        assert!(response.contains("\"duration_seconds\":2.000000"));
        assert!(response.contains("\"run_id\":1"));
        assert!(response.contains("\"state\":\"executing\""));
        assert_eq!(core.mode(), RuntimeMode::Moving);
        core.tick(11.0).unwrap();
        let motion = core.snapshot().motion.as_ref().unwrap();
        assert_eq!(motion.progress, 0.5);
        assert_eq!(motion.state, MovementPhase::Executing);
        core.tick(12.0).unwrap();
        assert_eq!(core.mode(), RuntimeMode::Moving);
        assert_eq!(
            core.snapshot().motion.as_ref().unwrap().state,
            MovementPhase::Settling
        );
        core.tick(12.01).unwrap();
        assert_eq!(core.mode(), RuntimeMode::Moving);
        core.tick(12.26).unwrap();
        assert_eq!(core.mode(), RuntimeMode::Holding);
        assert!(core.snapshot().motion.is_none());
        let completed = core.snapshot().last_motion.as_ref().unwrap();
        assert_eq!(completed.run_id, 1);
        assert_eq!(completed.state, MovementPhase::Completed);
        assert_eq!(completed.max_position_error_rad, Some(0.0));
        assert_eq!(completed.max_velocity_rad_s, Some(0.0));
        assert_eq!(core.driver().writes.len(), 2);
    }

    #[test]
    fn times_out_when_feedback_never_reaches_the_target() {
        let mut core = core_with_driver(FakeDriver::stuck());
        activate(&mut core);
        assert!(
            core.handle_command("goto home 1", 5.0)
                .contains("\"run_id\":1")
        );
        core.tick(6.0).unwrap();
        assert_eq!(
            core.snapshot().motion.as_ref().unwrap().state,
            MovementPhase::Settling
        );
        core.tick(8.0).unwrap();
        assert_eq!(core.mode(), RuntimeMode::Holding);
        assert_eq!(
            core.snapshot().last_motion.as_ref().unwrap().state,
            MovementPhase::TimedOut
        );
        assert!(
            core.snapshot()
                .last_motion
                .as_ref()
                .unwrap()
                .max_position_error_rad
                .unwrap()
                > COMPLETION_POSITION_TOLERANCE_RAD
        );
    }

    #[test]
    fn requires_low_measured_velocity_before_completion() {
        let mut driver = FakeDriver::new();
        driver.velocity_rad_s = COMPLETION_VELOCITY_TOLERANCE_RAD_S * 2.0;
        let mut core = core_with_driver(driver);
        activate(&mut core);
        core.handle_command("goto home 1", 5.0);
        core.tick(6.0).unwrap();
        core.tick(6.1).unwrap();
        assert_eq!(
            core.snapshot().motion.as_ref().unwrap().state,
            MovementPhase::Settling
        );
        core.tick(8.0).unwrap();
        let result = core.snapshot().last_motion.as_ref().unwrap();
        assert_eq!(result.state, MovementPhase::TimedOut);
        assert!(result.max_velocity_rad_s.unwrap() > COMPLETION_VELOCITY_TOLERANCE_RAD_S);
    }

    #[test]
    fn runs_authored_motion_and_reports_keyframes() {
        let mut core = core();
        activate(&mut core);
        let response = core.handle_command("play look_at_right", 5.0);
        assert!(response.contains("\"keyframes\":1"));
        assert!(response.contains("\"run_id\":1"));
        core.tick(5.25).unwrap();
        let motion = core.snapshot().motion.as_ref().unwrap();
        assert_eq!(motion.name, "look_at_right");
        assert_eq!(motion.keyframe_index, Some(0));
        assert_eq!(motion.keyframe_count, Some(1));
        assert!(core.handle_command("stop", 5.25).contains("\"ok\":true"));
        assert_eq!(core.mode(), RuntimeMode::Holding);
        assert_eq!(
            core.snapshot().last_motion.as_ref().unwrap().state,
            MovementPhase::Cancelled
        );
    }

    #[test]
    fn reports_the_active_run_when_a_second_motion_is_rejected() {
        let mut core = core();
        activate(&mut core);
        core.handle_command("goto home 2", 0.0);
        let response = core.handle_command("play look_at_right", 0.1);
        assert!(response.contains("motion already active"));
        assert!(response.contains("\"active_run_id\":1"));
        assert!(!response.contains("enable holding torque"));
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
