//! Bounded, observed social responses; planning occurs on metadata commands.
use super::surroundings::SeenFace;
use super::*;
use crate::expression::curiosity::{CuriosityContext, CuriosityKind};

#[derive(Debug, Default)]
pub(super) struct MutualPolicy {
    track: Option<u64>,
    since: Option<f64>,
    last: Option<f64>,
    away: Option<f64>,
    fired: bool,
}
impl MutualPolicy {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
    fn observe(&mut self, faces: &[SeenFace], now: f64, pinned: Option<u64>) -> Option<u64> {
        if self.last.is_some_and(|last| now - last > 2.0) {
            self.clear();
        }
        self.last = Some(now);
        let face = pinned
            .or(self.track)
            .and_then(|id| faces.iter().find(|face| face.track_id == id))
            .or_else(|| {
                if pinned.is_none() {
                    faces.iter().find(|face| face.frontal)
                } else {
                    None
                }
            });
        if let Some(face) = face.filter(|face| face.frontal) {
            if self.track != Some(face.track_id) || self.since.is_none() {
                self.track = Some(face.track_id);
                self.since = Some(now);
                self.fired = false;
            }
            self.away = None;
            if !self.fired && now - self.since.unwrap() >= 1.5 {
                self.fired = true;
                return Some(face.track_id);
            }
        } else {
            self.since = None;
            self.away.get_or_insert(now);
        }
        None
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Notice,
    Hi,
    Away,
    Returning,
}
#[derive(Debug)]
pub(super) struct Reaction {
    track: u64,
    started: f64,
    deadline: f64,
    run: Option<u64>,
    stage: Stage,
    centre: JointPositions,
}

impl CharacterCoordinator {
    fn reaction_ready<D: RuntimeDriver>(&self, core: &RuntimeCore<D>) -> bool {
        self.curiosity_available()
            && (core.mode() == RuntimeMode::Holding
                || core.snapshot().motion.as_ref().is_some_and(|motion| {
                    Some(motion.run_id) == self.gaze.status().run_id
                        || Some(motion.run_id) == self.active_idle_run_id
                }))
    }
    pub(super) fn observe_mutual<D: RuntimeDriver>(
        &mut self,
        faces: &[SeenFace],
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        if !self.surroundings.settings.mutual_gaze
            || !self.surroundings.runtime_allowed
            || !self.status.enabled
            || !self.gaze.status().enabled
        {
            self.surroundings.mutual.clear();
            return self.settle_reaction(now, core);
        }
        let pinned = self
            .surroundings
            .reaction
            .as_ref()
            .map(|reaction| reaction.track)
            .or(self.gaze.status().track_id);
        let candidate = self.surroundings.mutual.observe(faces, now, pinned);
        if let Some(reaction) = self.surroundings.reaction.as_ref() {
            if now >= reaction.deadline {
                return self.settle_reaction(now, core);
            }
            if matches!(reaction.stage, Stage::Away | Stage::Returning) {
                return Ok(());
            }
            let stage = if self
                .surroundings
                .mutual
                .away
                .is_some_and(|at| now - at >= 0.6)
            {
                Some((Stage::Away, "mutual_gaze_away"))
            } else if reaction.stage == Stage::Notice
                && now - reaction.started >= 2.0
                && faces
                    .iter()
                    .any(|face| face.track_id == reaction.track && face.frontal)
            {
                Some((Stage::Hi, "mutual_gaze_hi"))
            } else {
                None
            };
            if let Some((stage, clip)) = stage {
                let centre = reaction.centre.clone();
                let run = reaction.run;
                let anchor = self.status.active_anchor.clone().unwrap();
                let definition = self.social_clip(clip, &centre, &anchor, core)?;
                let replacing = run.filter(|id| {
                    core.snapshot().motion.as_ref().is_some_and(|motion| {
                        motion.run_id == *id && motion.state == MovementPhase::Executing
                    })
                });
                // Wait through the measured settling phase rather than stealing it.
                if replacing.is_none() && core.mode() != RuntimeMode::Holding {
                    return Ok(());
                }
                core.validate_curiosity_motion(
                    &definition,
                    &anchor,
                    replacing.map(|id| (id, now)),
                )?;
                let next = if let Some(id) = replacing {
                    core.extend_character_performance(id, definition, anchor, now)?
                } else {
                    core.play_generated_anchored_relative(definition, anchor, now)?
                };
                let reaction = self.surroundings.reaction.as_mut().unwrap();
                reaction.run = Some(next);
                reaction.stage = stage;
                self.status.active_clip = Some(clip.into());
            }
            return Ok(());
        }
        let Some(track) = candidate else {
            return Ok(());
        };
        if !self.reaction_ready(core) {
            return Ok(());
        }
        let anchor = self
            .status
            .active_anchor
            .clone()
            .ok_or_else(|| Error::InvalidState("Social response needs an anchor".into()))?;
        let centre: JointPositions = core
            .snapshot()
            .joints
            .iter()
            .map(|joint| (joint.name.clone(), joint.position))
            .collect();
        let mut definition = self.social_clip("mutual_gaze_notice", &centre, &anchor, core)?;
        if self.rng.index(2) == 0 {
            // Sometimes just meet the look; variation never creates a trigger.
            for frame in &mut definition.keyframes {
                if frame.marker.as_deref() == Some("curious") {
                    frame.target.insert(
                        "head_roll_joint".into(),
                        centre["head_roll_joint"] - anchor["head_roll_joint"],
                    );
                }
            }
        }
        core.validate_curiosity_motion(&definition, &anchor, None)?;
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
            .try_spend(
                now,
                &format!("face:{track}"),
                CuriosityKind::Gesture,
                context,
            )
            .is_err()
        {
            return Ok(());
        }
        self.preempt_idle_or_thinking(now, core)?;
        let run = core.play_generated_anchored_relative(definition, anchor, now)?;
        self.surroundings.reaction = Some(Reaction {
            track,
            started: now,
            deadline: now + 8.0,
            run: Some(run),
            stage: Stage::Notice,
            centre,
        });
        self.status.active_clip = Some("mutual_gaze_notice".into());
        self.reset_timers(now);
        Ok(())
    }
    fn social_clip<D: RuntimeDriver>(
        &self,
        name: &str,
        centre: &JointPositions,
        anchor: &JointPositions,
        core: &RuntimeCore<D>,
    ) -> Result<MotionDefinition> {
        let mut definition = core.motions().motion(name)?.clone();
        let last = definition.keyframes.len() - 1;
        for frame in &mut definition.keyframes[..last] {
            for (joint, position) in centre {
                *frame.target.entry(joint.clone()).or_default() += position - anchor[joint];
            }
        }
        Ok(definition)
    }
    pub(super) fn preempt_reaction<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        if let Some(reaction) = self.surroundings.reaction.take() {
            if reaction.run.is_some_and(|id| {
                core.snapshot()
                    .motion
                    .as_ref()
                    .is_some_and(|motion| motion.run_id == id)
            }) {
                checked(core.handle_command("stop", now))?;
            }
            self.status.active_clip = None;
        }
        Ok(())
    }
    pub(super) fn settle_reaction<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        let Some(reaction) = self.surroundings.reaction.as_ref() else {
            return Ok(());
        };
        if let Some(run) = reaction.run
            && let Some(phase) = terminal_phase(core, run)
        {
            if phase != MovementPhase::Completed
                || matches!(reaction.stage, Stage::Away | Stage::Returning)
            {
                self.surroundings.reaction = None;
                self.status.active_clip = None;
                return Ok(());
            }
            self.surroundings.reaction.as_mut().unwrap().run = None;
        }
        let reaction = self.surroundings.reaction.as_ref().unwrap();
        let invalid = now >= reaction.deadline
            || !self.surroundings.fresh(now)
            || !self.surroundings.runtime_allowed
            || !self.status.enabled
            || !self.gaze.status().enabled
            || !self.surroundings.settings.mutual_gaze;
        if !invalid || reaction.stage == Stage::Returning {
            return Ok(());
        }
        let Some(run) = reaction.run else {
            self.surroundings.reaction = None;
            self.status.active_clip = None;
            return Ok(());
        };
        let Some(motion) = core
            .snapshot()
            .motion
            .as_ref()
            .filter(|motion| motion.run_id == run)
        else {
            self.surroundings.reaction = None;
            return Ok(());
        };
        // A settling trajectory is already converging to the original anchor.
        if motion.state == MovementPhase::Executing
            && let Some(anchor) = self.status.active_anchor.clone()
        {
            let definition = attention_return_motion();
            if let Err(error) = core
                .validate_curiosity_motion(&definition, &anchor, Some((run, now)))
                .and_then(|()| {
                    core.extend_character_performance(run, definition, anchor, now)
                        .map(|_| ())
                })
            {
                // The installed tail was already proved inside the envelope
                // and ends at this anchor. Near a boundary, a longer replacement
                // can overshoot while braking; retain that proven tail instead.
                eprintln!("oriond: retaining validated social return: {error}");
            }
        }
        self.surroundings.reaction.as_mut().unwrap().stage = Stage::Returning;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn face(id: u64, frontal: bool) -> SeenFace {
        SeenFace {
            track_id: id,
            frontal,
            azimuth: 0.0,
            elevation: 0.0,
        }
    }
    #[test]
    fn mutual_requires_continuity_tracks_one_person_and_fires_once_per_look() {
        let mut policy = MutualPolicy::default();
        for at in [0.0, 0.5, 1.0] {
            assert_eq!(policy.observe(&[face(1, true)], at, None), None);
        }
        assert_eq!(policy.observe(&[face(1, true)], 1.5, None), Some(1));
        assert_eq!(policy.observe(&[face(1, true)], 2.0, Some(1)), None);
        policy.observe(&[face(2, true)], 2.5, Some(1));
        assert_eq!(policy.away, Some(2.5));
        assert_eq!(policy.observe(&[face(1, true)], 3.0, Some(1)), None);
        assert_eq!(policy.observe(&[face(1, true)], 6.0, None), None); // Gap resets the dwell.
        assert_eq!(policy.observe(&[face(1, true)], 7.5, None), Some(1));
    }
}

#[cfg(test)]
mod execution_tests {
    use super::*;
    use std::sync::atomic::Ordering;
    #[test]
    fn social_catalogue_variants_compile_at_v2_powered_anchors() {
        let (core, _, _) = crate::expression::gaze::tests::following();
        let limits = core.driver().joint_limits().unwrap();
        for name in ["mutual_gaze_notice", "mutual_gaze_hi", "mutual_gaze_away"] {
            let definition = core.motions().motion(name).unwrap();
            assert_eq!(definition.space, MotionSpace::AnchorRelative);
            assert!(definition.return_to_anchor);
            for pose in [
                "home",
                "attentive",
                "look_left",
                "look_right",
                "lean_reach_low",
                "lean_reach_left",
                "lean_reach_right",
            ] {
                let anchor = core.poses().pose(pose).unwrap();
                let sequence = crate::MotionSequence::compile_scaled_calibrated(
                    definition,
                    anchor.clone(),
                    anchor.keys().map(|joint| (joint.clone(), 0.0)).collect(),
                    None,
                    anchor.clone(),
                    definition.uniform_amplitude_scale(anchor, &limits).unwrap(),
                    &limits,
                )
                .unwrap();
                for step in 0..=(sequence.duration_seconds() / 0.02).ceil() as usize {
                    let positions = sequence
                        .sample((step as f64 * 0.02).min(sequence.duration_seconds()))
                        .unwrap();
                    for limit in &limits {
                        assert!(
                            (limit.lower_rad - 1e-8..=limit.upper_rad + 1e-8)
                                .contains(&positions[&limit.name])
                        );
                    }
                }
                let end = sequence.sample(sequence.duration_seconds()).unwrap();
                for (joint, position) in anchor {
                    assert!((end[joint] - position).abs() < 1e-8);
                }
            }
        }
    }
    #[test]
    fn mutual_sequence_is_bounded_observed_and_privacy_returns_the_anchor() {
        let (mut core, _, boot) = crate::expression::gaze::tests::following();
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
        let tick = |step: u64, character: &mut CharacterCoordinator, core: &mut RuntimeCore<_>| {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            character.set_curiosity_available(true);
            character
                .tick(now, core, false, None, false, None, None)
                .unwrap();
        };
        for step in 1..=250 {
            tick(step, &mut character, &mut core);
        }
        let anchor = character.status.active_anchor.clone().unwrap();
        let bearing = crate::control::camera_rays::Bearing::from_direction(
            core.camera_kinematics()
                .unwrap()
                .fk(&anchor)
                .unwrap()
                .camera
                .rotate_vector([0.0, 0.0, 1.0]),
        );
        let gaze = |now, character: &mut CharacterCoordinator, core: &mut RuntimeCore<_>| {
            let target = crate::expression::gaze::GazeTarget {
                track_id: 1,
                capture_boottime_ns: boot.load(Ordering::Relaxed),
                azimuth: bearing.azimuth,
                elevation: bearing.elevation,
                confidence: 0.9,
                category: Some("face".into()),
                range: None,
            };
            assert!(character.queue_gaze_target(target, now, core).unwrap());
        };
        let observe = |now,
                       frontal: bool,
                       character: &mut CharacterCoordinator,
                       core: &mut RuntimeCore<_>| {
            let value = serde_json::from_value(serde_json::json!({
                "capture_boottime_ns":boot.load(Ordering::Relaxed), "camera_on":true, "tracking_mode":"faces",
                "settings":{"mutual_gaze":true}, "objects":[], "looking_away":!frontal,
                "faces":[{"track_id":1,"azimuth":bearing.azimuth,"elevation":bearing.elevation,"frontal":frontal}]
            })).unwrap();
            character.observe_surroundings(value, now, core).unwrap();
        };
        gaze(5., &mut character, &mut core);
        observe(5., true, &mut character, &mut core);
        for step in 251..=425 {
            tick(step, &mut character, &mut core);
            if step % 5 == 0 {
                gaze(step as f64 * 0.02, &mut character, &mut core);
            }
            if step % 25 == 0 {
                observe(step as f64 * 0.02, true, &mut character, &mut core);
            }
            if step == 325 {
                assert_eq!(
                    character.status.active_clip.as_deref(),
                    Some("mutual_gaze_notice")
                );
            }
        }
        assert_eq!(
            character.status.active_clip.as_deref(),
            Some("mutual_gaze_hi")
        );
        for step in 426..=550 {
            tick(step, &mut character, &mut core);
            if step % 25 == 0 {
                observe(step as f64 * 0.02, false, &mut character, &mut core);
            }
        }
        assert_eq!(
            character.status.active_clip.as_deref(),
            Some("mutual_gaze_away")
        );
        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
        character.clear_surroundings(11., &mut core).unwrap();
        for step in 551..=750 {
            tick(step, &mut character, &mut core);
        }
        assert!(character.surroundings.reaction.is_none());
        assert_eq!(core.mode(), RuntimeMode::Holding);
        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
        for joint in &core.snapshot().joints {
            assert!((joint.position - anchor[&joint.name]).abs() < 0.01);
        }
        // Toggling preferences/clearing the camera cannot refresh the common budget.
        for step in 751..=850 {
            tick(step, &mut character, &mut core);
            if step % 25 == 0 {
                observe(step as f64 * 0.02, true, &mut character, &mut core);
            }
        }
        assert!(character.surroundings.reaction.is_none());
    }
}
