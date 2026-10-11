//! Exclusive Hands-mode ownership. Observations plan; ticks only track ownership.
use super::*;

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
    Enter,
    Follow,
    Return,
}
#[derive(Debug)]
struct HandsRun {
    id: u64,
    target: JointPositions,
    point: Option<[f64; 3]>,
    kind: MoveKind,
}
#[derive(Debug, Default)]
pub(super) struct HandsState {
    wanted: bool,
    blocked: bool,
    previous: Option<JointPositions>,
    entered: bool,
    received: Option<f64>,
    last_seen: Option<f64>,
    point: Option<[f64; 3]>,
    return_attempt_at: Option<f64>,
    run: Option<HandsRun>,
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
    pub(super) fn suppress_idle(&self, now: f64) -> bool {
        self.blocks_gaze() && (!self.fresh(now) || self.last_seen.is_none_or(|at| now - at >= 10.0))
    }
}
/// Hand-point changes smaller than this are palm jitter, not a work shift.
const HANDS_DEADBAND_M: f64 = 0.03;
fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
impl CharacterCoordinator {
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
                || core.snapshot().motion.as_ref().is_some_and(|motion| {
                    Some(motion.run_id) == self.active_idle_run_id
                        || Some(motion.run_id) == self.gaze.status().run_id
                        || self
                            .hands
                            .run
                            .as_ref()
                            .is_some_and(|run| run.id == motion.run_id)
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
        }
        self.hands.wanted = true;
        self.hands.received = Some(now);
        if count > 0 {
            self.hands.last_seen = Some(now);
        }
        self.finish_hands(now, core);
        if let Some(failure) = &self.status.hands_failure {
            return Err(Error::InvalidState(failure.message.clone()));
        }
        if !self.hands_available(core) {
            return Ok(());
        }
        if !self.hands.entered {
            if self.hands.run.is_some() {
                return Ok(());
            }
            // Entering is the one authored gesture: anticipation, arm lead and
            // a settle at the desk-facing pose.
            let target = core.poses().pose("desk_work")?.clone();
            let point = core
                .camera_kinematics()
                .ok_or_else(|| Error::InvalidState("Hands needs V2 geometry".into()))?
                .fk(&target)?
                .light
                .table_point([0., 0., 1.], 0.)?;
            let definition = core.prepare_hands_motion("hands_entry", &target, None)?;
            self.preempt_idle_or_thinking(now, core)?;
            if self.hands.previous.is_none() {
                self.hands.previous = self.status.active_anchor.clone();
            }
            let id = core.play_generated_anchored_relative(definition, target.clone(), now)?;
            self.hands.run = Some(HandsRun {
                id,
                target,
                point,
                kind: MoveKind::Enter,
            });
            self.status.active_clip = Some("hands_entry".into());
            self.reset_timers(now);
            return Ok(());
        }
        // After entry the light follows continuously, like face gaze: each
        // fresh hand point redirects the running step from its commanded state.
        let Some(point) = point.filter(|_| count > 0) else {
            return Ok(());
        };
        let pursued = self
            .hands
            .run
            .as_ref()
            .map_or(self.hands.point, |run| run.point);
        if pursued.is_some_and(|last| distance(last, point) <= HANDS_DEADBAND_M) {
            return Ok(());
        }
        let replacing = match &self.hands.run {
            None => None,
            Some(run)
                if run.kind == MoveKind::Follow
                    && core.snapshot().motion.as_ref().is_some_and(|motion| {
                        motion.run_id == run.id && motion.state == MovementPhase::Executing
                    }) =>
            {
                Some(run.id)
            }
            // Entry, return and measured settling finish before following.
            Some(_) => return Ok(()),
        };
        let anchor = self
            .status
            .active_anchor
            .clone()
            .ok_or_else(|| Error::InvalidState("Hands has no anchor".into()))?;
        let target = core.hands_light_target(&anchor, point)?;
        let definition = core.prepare_hands_track(&target, replacing.map(|id| (id, now)))?;
        let id = if let Some(id) = replacing {
            core.extend_character_performance(id, definition, target.clone(), now)?
        } else {
            self.preempt_idle_or_thinking(now, core)?;
            core.play_generated_anchored_relative(definition, target.clone(), now)?
        };
        self.hands.run = Some(HandsRun {
            id,
            target,
            point: Some(point),
            kind: MoveKind::Follow,
        });
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
        let Some(previous) = self.hands.previous.clone() else {
            return Ok(());
        };
        if self
            .hands
            .run
            .as_ref()
            .is_some_and(|run| run.kind == MoveKind::Return)
            || !self.hands_available(core)
        {
            return Ok(());
        }
        let owned = self.hands.run.as_ref().and_then(|run| {
            core.snapshot()
                .motion
                .as_ref()
                .filter(|motion| {
                    motion.run_id == run.id && motion.state == MovementPhase::Executing
                })
                .map(|_| run.id)
        });
        // Settling must finish before replacing a run. A later off-mode heartbeat
        // retries this recovery even if there are no camera frames at all.
        if self.hands.run.is_some() && owned.is_none() {
            return Ok(());
        }
        if self
            .hands
            .return_attempt_at
            .is_some_and(|at| now - at < 0.5)
        {
            return Ok(());
        }
        self.hands.return_attempt_at = Some(now);
        let recovery = (|| {
            let definition =
                core.prepare_hands_motion("hands_return", &previous, owned.map(|id| (id, now)))?;
            if let Some(id) = owned {
                core.extend_character_performance(id, definition, previous.clone(), now)
            } else {
                self.preempt_idle_or_thinking(now, core)?;
                core.play_generated_anchored_relative(definition, previous.clone(), now)
            }
        })();
        let id = match recovery {
            Ok(id) => id,
            Err(error) => {
                let movement = self
                    .status
                    .hands_failure
                    .as_ref()
                    .and_then(|failure| failure.movement.clone());
                self.fail_hands(
                    "return",
                    format!("Hands return was rejected: {error}. Go to rest, then resume Orion."),
                    movement,
                );
                return Err(error);
            }
        };
        self.hands.run = Some(HandsRun {
            id,
            target: previous,
            point: None,
            kind: MoveKind::Return,
        });
        self.status.active_clip = Some("hands_return".into());
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
                self.status.active_anchor = Some(run.target);
                self.status.state = self.idle_state();
                self.hands.entered = run.kind != MoveKind::Return;
                self.hands.point = run.point;
                if run.kind == MoveKind::Return {
                    self.hands.previous = None;
                }
                core.set_character_comfort(self.hands.entered);
            } else {
                let movement = [
                    core.snapshot().motion.as_ref(),
                    core.snapshot().last_motion.as_ref(),
                ]
                .into_iter()
                .flatten()
                .find(|motion| motion.run_id == run.id)
                .cloned();
                let detail = movement
                    .as_ref()
                    .and_then(|motion| motion.error.as_deref())
                    .unwrap_or(phase.as_str());
                let stage = match run.kind {
                    MoveKind::Enter => "entry",
                    MoveKind::Follow => "follow",
                    MoveKind::Return => "return",
                };
                let recovery = if run.kind == MoveKind::Return {
                    "Go to rest, then resume Orion."
                } else {
                    "Choose Faces or Off to request a return; if recovery fails, go to rest."
                };
                self.fail_hands(
                    stage,
                    format!("Hand tracking paused: {detail}. {recovery}"),
                    movement,
                );
            }
            self.status.active_clip = None;
            self.reset_timers(now);
        }
        if !self.status.enabled || !core.mode().torque_is_enabled() {
            self.hands = HandsState::default();
            self.status.hands_failure = None;
            core.set_character_comfort(false);
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
        core: &mut RuntimeCore<D>,
    ) {
        // Explicit motion replaces the user's work anchor; stale camera evidence
        // must not undo it. A mode change is required to enter Hands again.
        self.hands.blocked = self.hands.wanted;
        self.hands.previous = None;
        self.hands.entered = false;
        self.status.hands_failure = None;
        core.set_character_comfort(false);
    }
    pub(super) fn expire_hands<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        if self.hands.owns_anchor()
            && !self.hands.fresh(now)
            && self
                .status
                .hands_failure
                .as_ref()
                .is_none_or(|failure| failure.stage != "return")
        {
            self.leave_hands(now, core)?;
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
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let profile: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("hardware/profiles/v2.json")).unwrap(),
        )
        .unwrap();
        core.configure_comfort_box(serde_json::from_value(profile["comfort_box"].clone()).unwrap())
            .unwrap();
        let mut character = CharacterCoordinator::new(1);
        character.set_gaze_enabled(true);
        character.start(0., &mut core).unwrap();
        advance(&mut core, &mut character, &boot, 0., 5., false);
        character.set_curiosity_available(true);
        (core, character, boot)
    }
    fn advance(
        core: &mut RuntimeCore<FollowingDriver>,
        character: &mut CharacterCoordinator,
        boot: &AtomicU64,
        from: f64,
        to: f64,
        present: bool,
    ) {
        for step in (from * 50.) as u64 + 1..=(to * 50.) as u64 {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            if character.hands.wanted && step % 25 == 0 {
                character
                    .observe_hands(u8::from(present), None, now, core)
                    .unwrap();
            }
            character
                .tick(now, core, false, None, false, None, None)
                .unwrap();
        }
    }
    #[test]
    fn hands_enters_desk_anchor_and_returns_only_when_higher_priority_releases() {
        let (mut core, mut character, boot) = fixture();
        let original = character.status.active_anchor.clone().unwrap();
        character.observe_hands(1, None, 5., &mut core).unwrap();
        assert_eq!(character.status.active_clip.as_deref(), Some("hands_entry"));
        assert_eq!(character.status.active_anchor.as_ref(), Some(&original));
        advance(&mut core, &mut character, &boot, 5., 12., true);
        let desk = core.poses().pose("desk_work").unwrap().clone();
        assert_eq!(character.status.active_anchor.as_ref(), Some(&desk));
        assert!(character.hands.entered);
        assert!(
            character.can_auto_rest(&core),
            "Hands alone must not defer automatic rest"
        );
        let point = core
            .camera_kinematics()
            .unwrap()
            .fk(&desk)
            .unwrap()
            .camera
            .table_point([0., 0., 1.], 0.)
            .unwrap()
            .unwrap();
        assert!((0.25..0.4).contains(&-point[1]));
        character.set_curiosity_available(false);
        character.leave_hands(12., &mut core).unwrap();
        assert!(character.hands.run.is_none());
        character.set_curiosity_available(true);
        character.leave_hands(13., &mut core).unwrap();
        advance(&mut core, &mut character, &boot, 13., 21., false);
        assert!(!character.hands.owns_anchor());
        assert_eq!(character.status.active_anchor.as_ref(), Some(&original));
    }
    #[test]
    fn hands_follows_continuously_and_redirects_the_running_step() {
        let (mut core, mut character, boot) = fixture();
        character.observe_hands(1, None, 5., &mut core).unwrap();
        advance(&mut core, &mut character, &boot, 5., 12., true);
        let desk = character.status.active_anchor.clone().unwrap();
        let near = character.hands.point.unwrap();
        // Palm jitter inside the deadband does not move the light.
        character
            .observe_hands(1, Some([near[0] + 0.02, near[1], 0.]), 13., &mut core)
            .unwrap();
        assert!(character.hands.run.is_none());
        // A real shift starts following at once; no dwell or budget.
        character
            .observe_hands(1, Some([0.08, -0.32, 0.]), 13.2, &mut core)
            .unwrap();
        let first = character.hands.run.as_ref().unwrap();
        let (id, first_target) = (first.id, first.target.clone());
        assert_eq!(character.status.active_clip.as_deref(), Some("hands_track"));
        // The next point redirects the same run from its commanded state.
        advance(&mut core, &mut character, &boot, 13.2, 13.4, true);
        let shifted = [0.14, -0.32, 0.];
        character
            .observe_hands(1, Some(shifted), 13.4, &mut core)
            .unwrap();
        let run = character.hands.run.as_ref().unwrap();
        assert_eq!(run.id, id);
        assert_ne!(run.target, first_target);
        assert_eq!(run.point, Some(shifted));
        assert_eq!(character.status.active_anchor.as_ref(), Some(&desk));
        advance(&mut core, &mut character, &boot, 13.4, 20., true);
        assert_ne!(character.status.active_anchor.as_ref(), Some(&desk));
        assert_eq!(character.hands.point, Some(shifted));
        let held = character.status.active_anchor.clone();
        advance(&mut core, &mut character, &boot, 20., 36., false);
        assert_eq!(character.status.active_anchor, held);
        assert!(character.hands.suppress_idle(36.));
        assert_eq!(core.mode(), RuntimeMode::Holding);
    }
    #[test]
    fn stop_and_foreground_cannot_be_overridden_by_hands_metadata() {
        let (mut core, mut character, boot) = fixture();
        character.forget_gaze_for_external_stop(crate::expression::gaze::GazeStopCommand::Stop);
        character
            .observe_hands(1, Some([0., -0.3, 0.]), 5., &mut core)
            .unwrap();
        assert!(character.hands.run.is_none());
        character.resume_gaze(&core).unwrap();
        character.observe_hands(1, None, 6., &mut core).unwrap();
        advance(&mut core, &mut character, &boot, 6., 13., true);
        character.preempt_for_foreground(13., &mut core).unwrap();
        character
            .observe_hands(1, Some([0.15, -0.3, 0.]), 70., &mut core)
            .unwrap();
        assert!(character.hands.run.is_none());
        assert!(character.hands.blocked);
        assert!(!character.hands.owns_anchor());
    }
    #[test]
    fn stale_inference_during_entry_returns_the_original_anchor_without_a_jump() {
        let (mut core, mut character, boot) = fixture();
        let original = character.status.active_anchor.clone().unwrap();
        character.observe_hands(1, None, 5., &mut core).unwrap();
        let mut previous = core
            .snapshot()
            .joints
            .iter()
            .map(|j| (j.name.clone(), j.position))
            .collect::<JointPositions>();
        for step in 251..=850 {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
            for joint in &core.snapshot().joints {
                assert!((joint.position - previous[&joint.name]).abs() < 0.04);
                previous.insert(joint.name.clone(), joint.position);
            }
        }
        assert!(!character.hands.owns_anchor());
        assert_eq!(character.status.active_anchor.as_ref(), Some(&original));
        for joint in &core.snapshot().joints {
            assert!((joint.position - original[&joint.name]).abs() < 0.01);
        }
    }
    #[test]
    fn generated_expression_at_a_work_anchor_stays_in_the_coupled_envelope() {
        let (mut core, mut character, boot) = fixture();
        character.observe_hands(1, None, 5., &mut core).unwrap();
        advance(&mut core, &mut character, &boot, 5., 12., true);
        let anchor = character.status.active_anchor.clone().unwrap();
        let config = crate::HardwareVersion::V2
            .profile()
            .comfort_box
            .as_ref()
            .unwrap()
            .clone();
        let comfort = crate::motion::ik::ComfortBox::from_config(
            &config,
            core.poses(),
            &core.driver().joint_limits().unwrap(),
        )
        .unwrap();
        let mut definition = core.motions().motion("idle_micro_glance").unwrap().clone();
        definition.keyframes[0]
            .target
            .insert("shoulder_pitch_joint".into(), 1.5);
        definition.keyframes[0]
            .target
            .insert("elbow_pitch_joint".into(), 1.5);
        definition.keyframes[0]
            .target
            .insert("head_pitch_joint".into(), 1.5);
        let run = core
            .play_generated_anchored_relative(definition, anchor.clone(), 12.)
            .unwrap();
        for step in 601..=1100 {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            let positions = core
                .snapshot()
                .joints
                .iter()
                .map(|j| (j.name.clone(), j.position))
                .collect();
            assert!(comfort.contains(&positions));
        }
        assert_eq!(terminal_phase(&core, run), Some(MovementPhase::Completed));
        for joint in &core.snapshot().joints {
            assert!((joint.position - anchor[&joint.name]).abs() < 0.01);
        }
    }

    #[test]
    fn hands_reach_does_not_compound_after_completed_targets() {
        let (core, _, _) = fixture();
        let reference = core.poses().pose("desk_work").unwrap();
        let first = core
            .hands_light_target(reference, [0.08, -0.32, 0.])
            .unwrap();
        let other = core.hands_light_target(&first, [-0.08, -0.32, 0.]).unwrap();
        let repeated = core.hands_light_target(&other, [0.08, -0.32, 0.]).unwrap();
        for joint in ["shoulder_pitch_joint", "elbow_pitch_joint"] {
            assert!(
                (first[joint] - repeated[joint]).abs() < 1e-8,
                "same work point compounded {joint}: {} -> {}",
                first[joint],
                repeated[joint]
            );
        }
    }

    #[test]
    fn completed_hands_steps_keep_command_continuity_despite_bounded_feedback_bias() {
        let (mut core, mut character, boot) = fixture();
        character.observe_hands(1, None, 5., &mut core).unwrap();
        advance(&mut core, &mut character, &boot, 5., 12., true);
        character
            .observe_hands(1, Some([0.08, -0.32, 0.]), 13., &mut core)
            .unwrap();
        advance(&mut core, &mut character, &boot, 13., 18., true);
        let previous = character.status.active_anchor.clone().unwrap();
        // Hardware completed with a small positive shoulder/elbow offset.
        // Starting the next command at that feedback would briefly command
        // the offset itself, even though the servo still holds `previous`.
        core.driver().offset_feedback(0.04, 0.03);
        core.tick(18.02).unwrap();
        character
            .observe_hands(1, Some([-0.08, -0.32, 0.]), 18.02, &mut core)
            .unwrap();
        let run = character.hands.run.as_ref().unwrap().id;
        let start = core.character_commanded_state(run, 18.02).unwrap();
        for (joint, q) in &previous {
            assert!(
                (start.positions[joint] - q).abs() < 1e-9,
                "completed Hands command jumped at {joint}: {} -> {}",
                q,
                start.positions[joint]
            );
        }
        advance(&mut core, &mut character, &boot, 18.02, 23., true);
        let previous = character.status.active_anchor.clone().unwrap();
        character
            .observe_hands(1, Some([0.08, -0.32, 0.]), 23., &mut core)
            .unwrap();
        let run = character.hands.run.as_ref().unwrap().id;
        let start = core.character_commanded_state(run, 23.).unwrap();
        for (joint, q) in &previous {
            assert!((start.positions[joint] - q).abs() < 1e-9);
        }
    }

    #[test]
    fn completed_hands_command_does_not_hide_subsequent_feedback_drift() {
        let (mut core, mut character, boot) = fixture();
        character.observe_hands(1, None, 5., &mut core).unwrap();
        advance(&mut core, &mut character, &boot, 5., 12., true);
        // A completed command can be reused only while the measured arm is
        // still within the original completion limits, even inside comfort.
        core.driver().offset_feedback(-0.06, 0.);
        core.tick(12.02).unwrap();
        let result = character.observe_hands(1, Some([0.08, -0.32, 0.]), 12.02, &mut core);
        assert!(result.is_err());
        assert!(core.snapshot().motion.is_none());
    }

    #[test]
    fn timed_out_follow_latches_diagnostics_and_recovers_only_after_a_valid_return() {
        let (mut core, mut character, boot) = fixture();
        character.observe_hands(1, None, 5., &mut core).unwrap();
        advance(&mut core, &mut character, &boot, 5., 12., true);
        let committed = character.status.active_anchor.clone();
        character
            .observe_hands(1, Some([0.08, -0.32, 0.]), 13., &mut core)
            .unwrap();
        let run = character.hands.run.as_ref().unwrap().id;
        // Unlike the perfect following fixture, emulate a servo that stops
        // 0.07 rad away from every commanded shoulder goal.
        core.driver().offset_feedback(0.07, 0.);
        for step in 651..=1000 {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            if step % 25 == 0 {
                let result = character.observe_hands(1, None, now, &mut core);
                if result.is_err() {
                    assert!(character.status.hands_failure.is_some());
                }
            }
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
        }
        let failure = character.status.hands_failure.as_ref().unwrap();
        assert_eq!(failure.stage, "follow");
        let failed = failure.movement.as_ref().unwrap();
        assert_eq!(failed.run_id, run);
        assert_eq!(failed.state, MovementPhase::TimedOut);
        let shoulder = failed
            .completion_diagnostic
            .as_ref()
            .unwrap()
            .joints
            .iter()
            .find(|j| j.name == "shoulder_pitch_joint")
            .unwrap();
        assert!((shoulder.position_error_rad + 0.07).abs() < 1e-9);
        assert_eq!(character.status.active_anchor, committed);
        assert!(character.hands.run.is_none());
        assert!(core.snapshot().motion.is_none());
        assert!(
            character
                .observe_hands(1, Some([0.14, -0.32, 0.]), 20., &mut core)
                .is_err()
        );
        // A later healthy measurement permits the ordinary validated return;
        // the timeout itself never commits a new anchor or resets Stop.
        core.driver().offset_feedback(0., 0.);
        core.tick(20.02).unwrap();
        character.leave_hands(20.02, &mut core).unwrap();
        advance(&mut core, &mut character, &boot, 20.02, 28., false);
        assert!(character.status.hands_failure.is_none());
        assert!(!character.hands.blocks_gaze());
        assert!(
            character
                .queue_gaze_target(
                    crate::expression::gaze::GazeTarget {
                        track_id: 1,
                        capture_boottime_ns: boot.load(Ordering::Relaxed),
                        azimuth: 0.,
                        elevation: 0.,
                        confidence: 0.95,
                        category: Some("face".into()),
                        range: None,
                    },
                    28.,
                    &mut core
                )
                .unwrap()
        );
    }

    #[test]
    fn captured_frozen_pose_reports_failed_return_and_refuses_face_targets() {
        let (mut core, mut character, boot) = fixture();
        character.observe_hands(1, None, 5., &mut core).unwrap();
        advance(&mut core, &mut character, &boot, 5., 12., true);
        let held = core.driver().positions.clone();
        // Exact Ariadne shoulder/elbow feedback from the frozen session.
        core.driver().offset_feedback(
            1.2118448224296563 - held["shoulder_pitch_joint"],
            -0.35588354278946877 - held["elbow_pitch_joint"],
        );
        core.tick(12.02).unwrap();
        let error = character.leave_hands(12.02, &mut core).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Measured pose exceeds comfort recovery tolerance")
        );
        let failure = character.status.hands_failure.as_ref().unwrap();
        assert_eq!(failure.stage, "return");
        assert!(failure.recovery_required);
        assert!(failure.message.contains("Go to rest"));
        assert!(character.hands.owns_anchor());
        assert!(core.snapshot().motion.is_none());
        // Faces must receive an explicit rejection, never a false success.
        let error = character
            .queue_gaze_target(
                crate::expression::gaze::GazeTarget {
                    track_id: 1,
                    capture_boottime_ns: boot.load(Ordering::Relaxed),
                    azimuth: 0.,
                    elevation: 0.,
                    confidence: 0.95,
                    category: Some("face".into()),
                    range: None,
                },
                12.02,
                &mut core,
            )
            .unwrap_err();
        assert!(error.to_string().contains("Hands return was rejected"));
        assert!(character.gaze_status().track_id.is_none());
        assert!(character.can_auto_rest(&core));
        // A failed return is latched. Off-mode heartbeats must not schedule
        // repeated recovery moves even if feedback later becomes admissible.
        core.driver().offset_feedback(0., 0.);
        core.tick(13.02).unwrap();
        character.expire_hands(13.02, &mut core).unwrap();
        assert!(character.leave_hands(13.02, &mut core).is_err());
        assert!(core.snapshot().motion.is_none());
        assert_eq!(
            character.status.hands_failure.as_ref().unwrap().stage,
            "return"
        );
        checked(core.handle_command("disable", 13.1)).unwrap();
        character.finish_hands(13.1, &mut core);
        assert!(character.status.hands_failure.is_none());
        assert!(!character.hands.blocks_gaze());
    }
}
