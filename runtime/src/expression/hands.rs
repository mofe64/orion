//! Exclusive Hands-mode ownership. Observations plan; ticks only track ownership.
use super::*;
use crate::expression::curiosity::{CuriosityContext, CuriosityKind};

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
    candidate: Option<([f64; 3], f64)>,
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
        if !self.hands.fresh(now) {
            self.hands.candidate = None;
        }
        self.hands.wanted = true;
        self.hands.received = Some(now);
        if count > 0 {
            self.hands.last_seen = Some(now);
        }
        self.finish_hands(now, core);
        if !self.hands_available(core) || self.hands.run.is_some() {
            return Ok(());
        }
        let (name, target, destination, key, kind) = if !self.hands.entered {
            let target = core.poses().pose("desk_work")?.clone();
            let point = core
                .camera_kinematics()
                .ok_or_else(|| Error::InvalidState("Hands needs V2 geometry".into()))?
                .fk(&target)?
                .light
                .table_point([0., 0., 1.], 0.)?;
            (
                "hands_entry",
                target,
                point,
                "hands:entry".into(),
                MoveKind::Enter,
            )
        } else {
            let Some(point) = point.filter(|_| count > 0) else {
                self.hands.candidate = None;
                return Ok(());
            };
            if self
                .hands
                .point
                .is_some_and(|last| distance(last, point) <= 0.08)
            {
                self.hands.candidate = None;
                return Ok(());
            }
            let since = match self.hands.candidate {
                Some((candidate, since)) if distance(candidate, point) <= 0.04 => since,
                _ => {
                    self.hands.candidate = Some((point, now));
                    return Ok(());
                }
            };
            if now - since < 1.0 {
                return Ok(());
            }
            let anchor = self
                .status
                .active_anchor
                .as_ref()
                .ok_or_else(|| Error::InvalidState("Hands has no anchor".into()))?;
            let target = core.hands_light_target(anchor, point)?;
            (
                "hands_follow",
                target,
                Some(point),
                format!("hands:{:0.0}:{:0.0}", point[0] / 0.08, point[1] / 0.08),
                MoveKind::Follow,
            )
        };
        let definition = core.prepare_hands_motion(name, &target, None)?;
        let context = CuriosityContext {
            enabled: true,
            camera_on: true,
            tracking_on: true,
            powered_and_awake: true,
            character_available: true,
            ..Default::default()
        };
        if self
            .surroundings
            .budget
            .try_spend(now, &key, CuriosityKind::Gesture, context)
            .is_err()
        {
            return Ok(());
        }
        self.preempt_idle_or_thinking(now, core)?;
        if self.hands.previous.is_none() {
            self.hands.previous = self.status.active_anchor.clone();
        }
        let id = core.play_generated_anchored_relative(definition, target.clone(), now)?;
        self.hands.run = Some(HandsRun {
            id,
            target,
            point: destination,
            kind,
        });
        self.status.active_clip = Some(name.into());
        self.reset_timers(now);
        Ok(())
    }
    pub(crate) fn leave_hands<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        self.hands.wanted = false;
        self.hands.candidate = None;
        self.finish_hands(now, core);
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
        let definition =
            core.prepare_hands_motion("hands_return", &previous, owned.map(|id| (id, now)))?;
        let id = if let Some(id) = owned {
            core.extend_character_performance(id, definition, previous.clone(), now)?
        } else {
            self.preempt_idle_or_thinking(now, core)?;
            core.play_generated_anchored_relative(definition, previous.clone(), now)?
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
                self.status.active_anchor = Some(run.target);
                self.status.state = self.idle_state();
                self.hands.entered = run.kind != MoveKind::Return;
                self.hands.point = run.point;
                self.hands.candidate = None;
                if run.kind == MoveKind::Return {
                    self.hands.previous = None;
                }
                core.set_character_comfort(self.hands.entered);
            }
            self.status.active_clip = None;
            self.reset_timers(now);
        }
        if !self.status.enabled || !core.mode().torque_is_enabled() {
            self.hands = HandsState::default();
            core.set_character_comfort(false);
        }
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
        core.set_character_comfort(false);
    }
    pub(super) fn expire_hands<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        if self.hands.owns_anchor() && !self.hands.fresh(now) {
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
    fn hands_waits_for_shift_dwell_and_budget_then_holds_when_hands_disappear() {
        let (mut core, mut character, boot) = fixture();
        character.observe_hands(1, None, 5., &mut core).unwrap();
        advance(&mut core, &mut character, &boot, 5., 12., true);
        let desk = character.status.active_anchor.clone().unwrap();
        let near = character.hands.point.unwrap();
        character
            .observe_hands(1, Some([near[0] + 0.04, near[1], 0.]), 13., &mut core)
            .unwrap();
        assert!(character.hands.candidate.is_none());
        let shifted = [0.14, -0.32, 0.];
        for at in [14., 14.5, 15.] {
            character
                .observe_hands(1, Some(shifted), at, &mut core)
                .unwrap();
        }
        assert!(
            character.hands.run.is_none(),
            "mode entry already spent the shared budget"
        );
        character
            .observe_hands(1, Some(shifted), 65., &mut core)
            .unwrap();
        character
            .observe_hands(1, Some(shifted), 66., &mut core)
            .unwrap();
        assert_eq!(
            character.status.active_clip.as_deref(),
            Some("hands_follow")
        );
        assert_eq!(character.status.active_anchor.as_ref(), Some(&desk));
        advance(&mut core, &mut character, &boot, 66., 74., true);
        assert_ne!(character.status.active_anchor.as_ref(), Some(&desk));
        let held = character.status.active_anchor.clone();
        advance(&mut core, &mut character, &boot, 74., 90., false);
        assert_eq!(character.status.active_anchor, held);
        assert!(character.hands.suppress_idle(90.));
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
        let config = crate::motion::ik::ComfortBoxConfig {
            poses: vec![
                "home",
                "rest",
                "attentive",
                "thinking",
                "curious",
                "delight",
                "look_left",
                "look_right",
                "attention_left",
                "attention_right",
                "lean_reach_low",
                "lean_reach_left",
                "lean_reach_right",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        };
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
}
