//! Hands keeps the current anchor. Camera observations plan; ticks sample splines.
use super::*;
use crate::expression::gaze::{self, GAZE_DEADBAND_RAD, GAZE_UPDATE_SECONDS};

/// A latched tracking failure, independent of camera/detector health.
#[derive(Clone, Debug, Serialize)]
pub struct HandsFailure {
    pub stage: &'static str,
    pub message: String,
    pub recovery_required: bool,
    pub movement: Option<crate::MotionState>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum MoveKind {
    Follow,
    Return,
}
#[derive(Debug)]
struct HandsRun {
    id: u64,
    kind: MoveKind,
}
#[derive(Debug)]
pub(super) struct HandsState {
    wanted: bool,
    blocked: bool,
    previous: Option<JointPositions>,
    received: Option<f64>,
    last_seen: Option<f64>,
    point: Option<[f64; 3]>,
    goal: Option<JointPositions>,
    timed_out: bool,
    last_update: f64,
    maximum_arm_rad: f64,
    run: Option<HandsRun>,
}
impl Default for HandsState {
    fn default() -> Self {
        Self {
            wanted: false,
            blocked: false,
            previous: None,
            received: None,
            last_seen: None,
            point: None,
            goal: None,
            timed_out: false,
            last_update: f64::NEG_INFINITY,
            maximum_arm_rad: 5_f64.to_radians(),
            run: None,
        }
    }
}
impl HandsState {
    pub(super) fn owns_anchor(&self) -> bool {
        self.previous.is_some()
    }
    pub(super) fn blocks_gaze(&self) -> bool {
        self.wanted || self.owns_anchor()
    }
    fn fresh(&self, now: f64) -> bool {
        self.received
            .is_some_and(|at| (0.0..=2.0).contains(&(now - at)))
    }
    pub(super) fn suppress_idle(&self, _now: f64) -> bool {
        self.blocks_gaze()
    }
}
const HANDS_DEADBAND_M: f64 = 0.03;
const HANDS_LOST_SECONDS: f64 = 10.;
impl CharacterCoordinator {
    /// Independent behavior configuration; calibration and physical limits stay unchanged.
    pub fn set_hands_maximum_arm_degrees(&mut self, degrees: f64) -> Result<()> {
        if !degrees.is_finite() || !(0.0..=5.0).contains(&degrees) {
            return Err(Error::InvalidArgument(
                "Hands arm maximum must be between 0 and 5 degrees".into(),
            ));
        }
        self.hands.maximum_arm_rad = degrees.to_radians();
        Ok(())
    }
    fn hands_available<D: RuntimeDriver>(&self, core: &RuntimeCore<D>) -> bool {
        self.surroundings.runtime_allowed
            && self.status.enabled
            && self.gaze.status().enabled
            && !self.gaze.is_stopped()
            && self.work.is_none()
            && !self.work_anchor
            && self.attention.is_none()
            && self.starting_run_id.is_none()
            && !self.foreground_pending
            && self.foreground_scene_run_id.is_none()
            && self.speech_motion_run_id.is_none()
            && !self.hands.blocked
            && (self.status.hands_failure.is_none() || !self.hands.wanted)
            && self.surroundings.reaction.is_none()
            && matches!(
                self.status.state,
                CharacterState::HomeIdle | CharacterState::PoseIdle
            )
            && (core.mode() == RuntimeMode::Holding
                || core.snapshot().motion.as_ref().is_some_and(|m| {
                    Some(m.run_id) == self.active_idle_run_id
                        || Some(m.run_id) == self.gaze.status().run_id
                        || self.hands.run.as_ref().is_some_and(|r| r.id == m.run_id)
                }))
    }
    pub(super) fn observe_hands<D: RuntimeDriver>(
        &mut self,
        count: u8,
        point: Option<[f64; 3]>,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        if !self.hands.wanted {
            self.hands.blocked = false;
            self.hands.timed_out = false;
            self.hands.last_seen = None;
        }
        self.hands.wanted = true;
        self.hands.received = Some(now);
        if count > 0 {
            self.hands.last_seen = Some(now);
            self.hands.timed_out = false;
        }
        self.finish_hands(now, core);
        if let Some(failure) = &self.status.hands_failure {
            return Err(Error::InvalidState(failure.message.clone()));
        }
        if !self.hands_available(core) || self.hands.timed_out {
            return Ok(());
        }
        if self.hands.previous.is_none() {
            self.preempt_idle_or_thinking(now, core)?;
            // The held character pose remains the anchor during all hand aims.
            let held = core
                .snapshot()
                .joints
                .iter()
                .map(|j| (j.name.clone(), j.position))
                .collect::<JointPositions>();
            self.status.active_anchor = Some(held.clone());
            self.hands.previous = Some(held);
        }
        let Some(anchor) = self.hands.previous.as_ref() else {
            return Ok(());
        };
        let point = point.filter(|_| count > 0);
        let changed = match (point, self.hands.point) {
            (Some(a), Some(b)) => (a[0] - b[0]).hypot(a[1] - b[1]) > HANDS_DEADBAND_M,
            (None, None) => self.hands.goal.is_none(),
            _ => true,
        };
        if changed {
            let goal = if let Some(point) = point {
                core.hands_light_target(anchor, point, self.hands.maximum_arm_rad)?
            } else {
                // Keep the neck direction and arms. Positive V2 head pitch looks down.
                let mut goal = anchor.clone();
                let limits = core.driver().joint_limits()?;
                let limit = limits
                    .iter()
                    .find(|l| l.name == "head_pitch_joint")
                    .unwrap();
                goal.insert(
                    "head_pitch_joint".into(),
                    (anchor["head_pitch_joint"] + 30_f64.to_radians()).clamp(
                        limit.lower_rad + 3_f64.to_radians(),
                        limit.upper_rad - 3_f64.to_radians(),
                    ),
                );
                goal
            };
            self.hands.goal = Some(goal);
            self.hands.point = point;
        }
        self.update_hands(now, core, false)
    }
    fn update_hands<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
        returning: bool,
    ) -> Result<()> {
        let result = self.plan_hand_step(now, core, returning);
        if let Err(error) = &result {
            let stage = if returning { "return" } else { "follow" };
            let movement = self
                .status
                .hands_failure
                .as_ref()
                .and_then(|f| f.movement.clone());
            self.fail_hands(
                stage,
                format!("Hand tracking paused: {error}. Go to rest, then resume Orion."),
                movement,
            );
        }
        result
    }
    fn plan_hand_step<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
        returning: bool,
    ) -> Result<()> {
        if !self.hands_available(core)
            || (!returning && now - self.hands.last_update + 1e-9 < GAZE_UPDATE_SECONDS)
        {
            return Ok(());
        }
        if self
            .hands
            .run
            .as_ref()
            .is_some_and(|r| r.kind == MoveKind::Return)
        {
            return Ok(());
        }
        let Some(goal) = (if returning {
            self.hands.previous.clone()
        } else {
            self.hands.goal.clone()
        }) else {
            return Ok(());
        };
        let replacing = self.hands.run.as_ref().map(|r| r.id);
        let state = if let Some(id) = replacing {
            let Some(state) = core.character_commanded_state(id, now) else {
                return Ok(());
            };
            state
        } else if let Some(state) = core.completed_hands_command()? {
            state
        } else {
            let snapshot = core.snapshot();
            crate::TrajectorySample {
                positions: snapshot
                    .joints
                    .iter()
                    .map(|j| (j.name.clone(), j.position))
                    .collect(),
                velocities: snapshot
                    .joints
                    .iter()
                    .map(|j| (j.name.clone(), j.velocity))
                    .collect(),
                accelerations: snapshot
                    .joints
                    .iter()
                    .map(|j| (j.name.clone(), 0.))
                    .collect(),
            }
        };
        let mut target = goal.clone();
        if !returning {
            gaze::smooth_head(&state.positions, &goal, &mut target);
            // Use the same angular deadband as face gaze, including tilted neck FK.
            let model = core
                .camera_kinematics()
                .ok_or_else(|| Error::InvalidState("Hands needs V2 geometry".into()))?;
            let current = model
                .fk(&state.positions)?
                .camera
                .rotate_vector([0., 0., 1.]);
            let desired = model.fk(&goal)?.camera.rotate_vector([0., 0., 1.]);
            let error = current
                .iter()
                .zip(desired)
                .map(|(a, b)| a * b)
                .sum::<f64>()
                .clamp(-1., 1.)
                .acos();
            if error <= GAZE_DEADBAND_RAD {
                for name in ["head_roll_joint", "head_pitch_joint"] {
                    target.insert(name.into(), state.positions[name]);
                }
            }
            if target
                .iter()
                .all(|(n, q)| (q - state.positions[n]).abs() < 1e-6)
            {
                return Ok(());
            }
        }
        let Some(definition) =
            gaze::hand_step(&state, &target, &core.driver().joint_limits()?, returning)?
        else {
            return Ok(());
        };
        let id = if let Some(id) = replacing {
            core.extend_character_performance(id, definition, target.clone(), now)?
        } else {
            self.preempt_idle_or_thinking(now, core)?;
            core.play_generated_anchored_relative(definition, target.clone(), now)?
        };
        self.hands.run = Some(HandsRun {
            id,
            kind: if returning {
                MoveKind::Return
            } else {
                MoveKind::Follow
            },
        });
        self.hands.last_update = now;
        self.status.active_clip = Some("hands_track".into());
        self.reset_timers(now);
        Ok(())
    }
    pub(crate) fn leave_hands<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        self.hands.wanted = false;
        self.finish_hands(now, core);
        if let Some(failure) = &self.status.hands_failure
            && failure.stage == "return"
        {
            return Err(Error::InvalidState(failure.message.clone()));
        }
        if self.hands.previous.is_none() {
            return Ok(());
        }
        if let Err(error) = self.update_hands(now, core, true) {
            let movement = self
                .status
                .hands_failure
                .as_ref()
                .and_then(|f| f.movement.clone());
            self.fail_hands(
                "return",
                format!("Hands return was rejected: {error}. Go to rest, then resume Orion."),
                movement,
            );
            return Err(error);
        }
        Ok(())
    }
    pub(super) fn preempt_hands<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        if let Some(run) = self.hands.run.take() {
            if core
                .snapshot()
                .motion
                .as_ref()
                .is_some_and(|motion| motion.run_id == run.id)
            {
                checked(core.handle_command("stop", now))?;
            }
            self.status.active_clip = None;
        }
        Ok(())
    }
    pub(super) fn finish_hands<D: RuntimeDriver>(&mut self, now: f64, core: &mut RuntimeCore<D>) {
        if let Some(run) = self.hands.run.as_ref()
            && let Some(phase) = terminal_phase(core, run.id)
        {
            let run = self.hands.run.take().unwrap();
            if phase == MovementPhase::Completed {
                self.status.hands_failure = None;
                self.status.state = self.idle_state();
                if run.kind == MoveKind::Return {
                    self.hands.previous = None;
                    self.hands.goal = None;
                    self.hands.point = None;
                }
            } else {
                let movement = [
                    core.snapshot().motion.as_ref(),
                    core.snapshot().last_motion.as_ref(),
                ]
                .into_iter()
                .flatten()
                .find(|m| m.run_id == run.id)
                .cloned();
                let detail = movement
                    .as_ref()
                    .and_then(|m| m.error.as_deref())
                    .unwrap_or(phase.as_str());
                let stage = if run.kind == MoveKind::Return {
                    "return"
                } else {
                    "follow"
                };
                self.fail_hands(stage, format!("Hand tracking paused: {detail}. Choose Idle, Faces or Off to return; if recovery fails, go to rest."), movement);
            }
            self.status.active_clip = None;
            self.reset_timers(now);
        }
        if !self.status.enabled || !core.mode().torque_is_enabled() {
            let maximum_arm_rad = self.hands.maximum_arm_rad;
            self.hands = HandsState {
                maximum_arm_rad,
                ..HandsState::default()
            };
            self.status.hands_failure = None;
        }
    }
    fn fail_hands(
        &mut self,
        stage: &'static str,
        message: String,
        movement: Option<crate::MotionState>,
    ) {
        if self
            .status
            .hands_failure
            .as_ref()
            .is_none_or(|failure| failure.message != message)
        {
            eprintln!("oriond: {message}");
        }
        self.status.last_rejection = Some(message.clone());
        self.status.hands_failure = Some(HandsFailure {
            stage,
            message,
            recovery_required: true,
            movement,
        });
    }
    pub(super) fn hands_foreground_override<D: RuntimeDriver>(
        &mut self,
        _core: &mut RuntimeCore<D>,
    ) {
        self.hands.blocked = self.hands.wanted;
        self.hands.previous = None;
        self.hands.goal = None;
        self.status.hands_failure = None;
    }
    pub(super) fn expire_hands<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        if self.hands.owns_anchor()
            && (!self.hands.fresh(now)
                || !self.hands.wanted
                || self
                    .hands
                    .last_seen
                    .is_some_and(|at| now - at >= HANDS_LOST_SECONDS))
        {
            let wanted = self.hands.wanted;
            if self
                .status
                .hands_failure
                .as_ref()
                .is_none_or(|f| f.stage != "return")
            {
                self.leave_hands(now, core)?;
            }
            self.hands.wanted = wanted;
            self.hands.timed_out = true;
        } else if self.hands.owns_anchor()
            && !self.hands.timed_out
            && self.status.hands_failure.is_none()
        {
            self.update_hands(now, core, false)?;
        }
        Ok(())
    }
    pub(super) fn hands_run_active(&self) -> bool {
        self.hands.run.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expression::gaze::tests::{FollowingDriver, following};
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    fn fixture() -> (
        RuntimeCore<FollowingDriver>,
        CharacterCoordinator,
        Arc<AtomicU64>,
    ) {
        let (mut core, _, boot) = following();
        core.configure_comfort_box(
            crate::HardwareVersion::V2
                .profile()
                .comfort_box
                .clone()
                .unwrap(),
        )
        .unwrap();
        let mut character = CharacterCoordinator::new(1);
        character.set_gaze_enabled(true);
        character.start(0., &mut core).unwrap();
        advance(&mut core, &mut character, &boot, 0., 5., None);
        character.set_curiosity_available(true);
        (core, character, boot)
    }
    fn advance(
        core: &mut RuntimeCore<FollowingDriver>,
        c: &mut CharacterCoordinator,
        boot: &AtomicU64,
        from: f64,
        to: f64,
        observation: Option<(u8, Option<[f64; 3]>)>,
    ) {
        for step in (from * 50.) as u64 + 1..=(to * 50.) as u64 {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            if let Some(anchor) = &c.hands.previous {
                for name in ["shoulder_pitch_joint", "elbow_pitch_joint"] {
                    assert!(
                        (core.driver().positions[name] - anchor[name]).abs()
                            <= c.hands.maximum_arm_rad + 1e-8,
                        "arm bound at {now}: {name}"
                    );
                }
            }
            if step % 25 == 0
                && let Some((count, point)) = observation
            {
                c.observe_hands(count, point, now, core).unwrap();
            }
            c.tick(now, core, false, None, false, None, None).unwrap();
        }
    }
    fn assert_held_arm(core: &RuntimeCore<FollowingDriver>, anchor: &JointPositions) {
        for name in [
            "base_yaw_joint",
            "shoulder_pitch_joint",
            "elbow_pitch_joint",
        ] {
            assert!(
                (core.driver().positions[name] - anchor[name]).abs() < 1e-9,
                "{name}"
            );
        }
    }
    #[test]
    fn head_only_hand_target_keeps_the_current_arm_and_anchor() {
        let (mut core, mut c, boot) = fixture();
        let anchor = c.status.active_anchor.clone().unwrap();
        let point = [0.08, -0.35, 0.];
        let target = core
            .hands_light_target(&anchor, point, c.hands.maximum_arm_rad)
            .unwrap();
        for name in [
            "base_yaw_joint",
            "shoulder_pitch_joint",
            "elbow_pitch_joint",
        ] {
            assert_eq!(target[name], anchor[name]);
        }
        c.observe_hands(1, Some(point), 5., &mut core).unwrap();
        advance(&mut core, &mut c, &boot, 5., 10., Some((1, Some(point))));
        assert_held_arm(&core, &anchor);
        assert_eq!(c.status.active_anchor.as_ref(), Some(&anchor));
        assert_ne!(
            core.driver().positions["head_pitch_joint"],
            anchor["head_pitch_joint"]
        );
        assert!(c.can_auto_rest(&core));
    }
    #[test]
    fn no_hand_search_tilts_only_the_head_and_holds() {
        let (mut core, mut c, boot) = fixture();
        let anchor = c.status.active_anchor.clone().unwrap();
        c.observe_hands(0, None, 5., &mut core).unwrap();
        advance(&mut core, &mut c, &boot, 5., 20., Some((0, None)));
        assert_held_arm(&core, &anchor);
        assert_eq!(
            core.driver().positions["head_roll_joint"],
            anchor["head_roll_joint"]
        );
        assert!(
            core.driver().positions["head_pitch_joint"]
                > anchor["head_pitch_joint"] + 20_f64.to_radians()
        );
        assert_eq!(c.status.active_anchor.as_ref(), Some(&anchor));
        assert!(c.status.hands_failure.is_none());
    }
    #[test]
    fn fallback_search_is_bounded_and_returns_when_head_reach_resumes() {
        let (mut core, mut c, boot) = fixture();
        let anchor = c.status.active_anchor.clone().unwrap();
        let mut found = None;
        'outer: for x in [0.35, 0.45, 0.55, -0.35, -0.45, -0.55] {
            for y in [-0.02, -0.01, -0.005, -0.001, -0.05] {
                let point = [x, y, 0.];
                let target = core
                    .hands_light_target(&anchor, point, c.hands.maximum_arm_rad)
                    .unwrap();
                let amount = ["shoulder_pitch_joint", "elbow_pitch_joint"]
                    .into_iter()
                    .map(|n| (target[n] - anchor[n]).abs())
                    .fold(0_f64, f64::max);
                assert!(amount <= c.hands.maximum_arm_rad + 1e-9);
                if amount > 0.005 {
                    found = Some(point);
                    break 'outer;
                }
            }
        }
        let point = found.expect("desk point must exercise the bounded fallback");
        eprintln!("Hands fallback fixture: {point:?}");
        c.observe_hands(1, Some(point), 5., &mut core).unwrap();
        advance(&mut core, &mut c, &boot, 5., 10., Some((1, Some(point))));
        for name in ["shoulder_pitch_joint", "elbow_pitch_joint"] {
            assert!(
                (core.driver().positions[name] - anchor[name]).abs()
                    <= c.hands.maximum_arm_rad + 1e-6
            );
        }
        let near = [0., -0.35, 0.];
        c.observe_hands(1, Some(near), 10., &mut core).unwrap();
        advance(&mut core, &mut c, &boot, 10., 14., Some((1, Some(near))));
        assert_held_arm(&core, &anchor);
        c.observe_hands(1, Some(point), 14., &mut core).unwrap();
        advance(&mut core, &mut c, &boot, 14., 18., Some((1, Some(point))));
        c.observe_hands(0, None, 18., &mut core).unwrap();
        advance(&mut core, &mut c, &boot, 18., 22., Some((0, None)));
        assert_held_arm(&core, &anchor);
    }
    #[test]
    fn hand_loss_timeout_and_mode_exit_restore_the_held_pose() {
        for stale_camera in [false, true] {
            let (mut core, mut c, boot) = fixture();
            let anchor = c.status.active_anchor.clone().unwrap();
            let point = [0.08, -0.35, 0.];
            c.observe_hands(1, Some(point), 5., &mut core).unwrap();
            advance(&mut core, &mut c, &boot, 5., 9., Some((1, Some(point))));
            advance(
                &mut core,
                &mut c,
                &boot,
                9.,
                24.,
                if stale_camera { None } else { Some((0, None)) },
            );
            for (n, q) in &anchor {
                assert!((core.driver().positions[n] - q).abs() < 1e-9, "{n}");
            }
            assert!(!c.hands.owns_anchor());
            assert!(c.hands.timed_out);
            c.observe_hands(1, Some(point), 24., &mut core).unwrap();
            advance(&mut core, &mut c, &boot, 24., 28., Some((1, Some(point))));
            c.leave_hands(28., &mut core).unwrap();
            advance(&mut core, &mut c, &boot, 28., 32., None);
            assert!(!c.hands.blocks_gaze());
            assert_eq!(c.status.active_anchor.as_ref(), Some(&anchor));
            for (n, q) in &anchor {
                assert!((core.driver().positions[n] - q).abs() < 1e-9);
            }
            c.observe_hands(0, None, 32., &mut core).unwrap();
            advance(&mut core, &mut c, &boot, 32., 44., Some((0, None)));
            assert!(!c.hands.timed_out);
            assert!(
                core.driver().positions["head_pitch_joint"]
                    > anchor["head_pitch_joint"] + 20_f64.to_radians()
            );
        }
    }
    #[test]
    fn stop_and_foreground_cannot_be_overridden_by_hands_metadata() {
        let (mut core, mut c, boot) = fixture();
        c.forget_gaze_for_external_stop(gaze::GazeStopCommand::Stop);
        c.observe_hands(1, Some([0., -0.3, 0.]), 5., &mut core)
            .unwrap();
        assert!(c.hands.run.is_none());
        c.resume_gaze(&core).unwrap();
        c.observe_hands(0, None, 6., &mut core).unwrap();
        advance(&mut core, &mut c, &boot, 6., 10., Some((0, None)));
        c.preempt_for_foreground(10., &mut core).unwrap();
        c.observe_hands(1, Some([0.15, -0.3, 0.]), 11., &mut core)
            .unwrap();
        assert!(c.hands.run.is_none());
        assert!(c.hands.blocked);
        assert!(!c.hands.owns_anchor());
    }
    #[test]
    fn failed_follow_latches_diagnostics_until_a_valid_return() {
        let (mut core, mut c, boot) = fixture();
        let anchor = c.status.active_anchor.clone();
        c.observe_hands(0, None, 5., &mut core).unwrap();
        core.driver().offset_feedback(0.07, 0.);
        for step in 251..=700 {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            if step % 25 == 0 {
                let _ = c.observe_hands(0, None, now, &mut core);
            }
            c.tick(now, &mut core, false, None, false, None, None)
                .unwrap();
        }
        let failure = c.status.hands_failure.as_ref().unwrap();
        assert_eq!(failure.stage, "follow");
        assert_eq!(
            failure.movement.as_ref().unwrap().state,
            MovementPhase::TimedOut
        );
        assert!(
            failure
                .movement
                .as_ref()
                .unwrap()
                .completion_diagnostic
                .is_some()
        );
        assert_eq!(c.status.active_anchor, anchor);
        core.driver().offset_feedback(0., 0.);
        core.tick(14.02).unwrap();
        c.leave_hands(14.02, &mut core).unwrap();
        advance(&mut core, &mut c, &boot, 14.02, 19., None);
        assert!(c.status.hands_failure.is_none());
        assert!(!c.hands.blocks_gaze());
    }
    #[test]
    fn completed_steps_keep_command_continuity_and_reject_later_drift() {
        let (mut core, mut c, boot) = fixture();
        c.observe_hands(0, None, 5., &mut core).unwrap();
        advance(&mut core, &mut c, &boot, 5., 9., Some((0, None)));
        let held = core.driver().positions.clone();
        core.driver().offset_feedback(0.02, 0.02);
        core.tick(9.02).unwrap();
        c.observe_hands(1, Some([0.08, -0.35, 0.]), 9.02, &mut core)
            .unwrap();
        let id = c.hands.run.as_ref().unwrap().id;
        let state = core.character_commanded_state(id, 9.02).unwrap();
        for (name, q) in &held {
            assert!((state.positions[name] - q).abs() < 1e-9);
        }
        core.driver().offset_feedback(0., 0.);
        advance(
            &mut core,
            &mut c,
            &boot,
            9.02,
            14.,
            Some((1, Some([0.08, -0.35, 0.]))),
        );
        core.driver().offset_feedback(0.07, 0.);
        core.tick(14.02).unwrap();
        assert!(c.leave_hands(14.02, &mut core).is_err());
        assert_eq!(c.status.hands_failure.as_ref().unwrap().stage, "return");
        assert!(c.hands.owns_anchor());
        assert!(
            c.queue_gaze_target(
                crate::expression::gaze::GazeTarget {
                    track_id: 1,
                    capture_boottime_ns: boot.load(Ordering::Relaxed),
                    azimuth: 0.,
                    elevation: 0.,
                    confidence: 0.95,
                    category: Some("face".into()),
                    range: None
                },
                14.02,
                &mut core
            )
            .is_err()
        );
        core.driver().offset_feedback(0., 0.);
        core.tick(15.).unwrap();
        assert!(c.leave_hands(15., &mut core).is_err());
        checked(core.handle_command("disable", 15.1)).unwrap();
        c.finish_hands(15.1, &mut core);
        assert!(c.status.hands_failure.is_none());
        assert!(!c.hands.blocks_gaze());
    }
    #[test]
    fn hands_captures_the_current_pose_including_an_existing_turn() {
        let (mut core, mut c, _) = fixture();
        checked(core.handle_command("goto look_left 1", 5.)).unwrap();
        for tick in 251..=400 {
            core.tick(tick as f64 * 0.02).unwrap();
        }
        let held = core.driver().positions.clone();
        c.observe_hands(0, None, 8., &mut core).unwrap();
        assert_eq!(c.hands.previous.as_ref(), Some(&held));
        assert_eq!(c.status.active_anchor.as_ref(), Some(&held));
        let goal = c.hands.goal.as_ref().unwrap();
        assert_eq!(goal["base_yaw_joint"], held["base_yaw_joint"]);
        assert_eq!(goal["shoulder_pitch_joint"], held["shoulder_pitch_joint"]);
        assert_eq!(goal["elbow_pitch_joint"], held["elbow_pitch_joint"]);
    }
    #[test]
    fn rapid_hand_redirections_keep_every_sample_inside_the_configured_arm_bound() {
        let (mut core, mut c, boot) = fixture();
        c.set_hands_maximum_arm_degrees(1.).unwrap();
        for segment in 0..40 {
            let from = 5. + segment as f64 * 0.16;
            let point = match segment % 3 {
                0 => [0.35, -0.001, 0.],
                1 => [-0.35, -0.001, 0.],
                _ => [0., -0.35, 0.],
            };
            c.observe_hands(1, Some(point), from, &mut core).unwrap();
            advance(&mut core, &mut c, &boot, from, from + 0.16, None);
        }
    }
    #[test]
    fn configured_fallback_maximum_rejects_invalid_values() {
        let mut c = CharacterCoordinator::new(1);
        for degrees in [-1., 5.1, f64::NAN, f64::INFINITY] {
            assert!(c.set_hands_maximum_arm_degrees(degrees).is_err());
        }
        c.set_hands_maximum_arm_degrees(0.).unwrap();
        assert_eq!(c.hands.maximum_arm_rad, 0.);
    }
}
