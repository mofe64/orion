//! Bounded, observed social responses; planning occurs on metadata commands.
use super::surroundings::{NovelObject, SeenFace};
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
#[derive(Debug, Default)]
pub(super) struct ArrivalPolicy {
    empty_since: Option<f64>,
    last: Option<f64>,
}
impl ArrivalPolicy {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
    fn observe(&mut self, faces: &[SeenFace], now: f64) -> Option<SeenFace> {
        if self.last.is_some_and(|last| now - last > 2.0) {
            self.clear();
        }
        self.last = Some(now);
        if faces.is_empty() {
            self.empty_since.get_or_insert(now);
            return None;
        }
        let empty_since = self.empty_since.take();
        empty_since
            .filter(|since| now - since >= 600.0)
            .map(|_| faces[0].clone())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReactionKind {
    Mutual,
    Arrival,
    Peek,
    Pet,
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
    kind: ReactionKind,
    track: u64,
    started: f64,
    deadline: f64,
    run: Option<u64>,
    stage: Stage,
    centre: JointPositions,
    object: Option<(String, f64, f64, f64)>,
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
            if reaction.kind != ReactionKind::Mutual {
                return Ok(());
            }
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
            kind: ReactionKind::Mutual,
            track,
            started: now,
            deadline: now + 8.0,
            run: Some(run),
            stage: Stage::Notice,
            centre,
            object: None,
        });
        self.status.active_clip = Some("mutual_gaze_notice".into());
        self.reset_timers(now);
        Ok(())
    }
    pub(super) fn observe_arrival<D: RuntimeDriver>(
        &mut self,
        faces: &[SeenFace],
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        if !self.surroundings.settings.arrival_notice {
            self.surroundings.arrival.clear();
            return Ok(());
        }
        let Some(face) = self.surroundings.arrival.observe(faces, now) else {
            return Ok(());
        };
        if self.surroundings.reaction.is_some() || !self.reaction_ready(core) {
            return Ok(());
        }
        let anchor = self
            .status
            .active_anchor
            .clone()
            .ok_or_else(|| Error::InvalidState("Arrival needs an anchor".into()))?;
        let definition = core.prepare_arrival_motion(&anchor, face.azimuth, face.elevation)?;
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
                &format!("face:{}", face.track_id),
                CuriosityKind::Gesture,
                context,
            )
            .is_err()
        {
            return Ok(());
        }
        self.preempt_idle_or_thinking(now, core)?;
        let run = core.play_generated_anchored_relative(definition, anchor.clone(), now)?;
        self.surroundings.reaction = Some(Reaction {
            kind: ReactionKind::Arrival,
            track: face.track_id,
            started: now,
            deadline: now + 8.0,
            run: Some(run),
            stage: Stage::Notice,
            centre: anchor,
            object: None,
        });
        self.status.active_clip = Some("arrival_notice".into());
        self.reset_timers(now);
        Ok(())
    }
    pub(super) fn observe_novelty<D: RuntimeDriver>(
        &mut self,
        object: Option<&NovelObject>,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        if let Some(reaction) = self.surroundings.reaction.as_ref()
            && matches!(reaction.kind, ReactionKind::Peek | ReactionKind::Pet)
        {
            let matching = object.filter(|object| {
                reaction
                    .object
                    .as_ref()
                    .is_some_and(|(id, _, _, _)| id == &object.id)
            });
            if matching.is_none() || !self.surroundings.settings.new_object_peek {
                self.surroundings.reaction.as_mut().unwrap().deadline = now;
                return self.settle_reaction(now, core);
            }
            let object = matching.unwrap();
            if reaction.kind != ReactionKind::Pet
                || reaction.stage == Stage::Returning
                || !(1.7..4.7).contains(&(now - reaction.started))
                || !self.curiosity_available()
            {
                return Ok(());
            }
            let (_, last, azimuth, elevation) = reaction.object.as_ref().unwrap();
            let delta = object.azimuth - azimuth;
            if now - last < 0.5
                || delta
                    .sin()
                    .atan2(delta.cos())
                    .hypot(object.elevation - elevation)
                    < 0.07
            {
                return Ok(());
            }
            let Some(run) = reaction.run.filter(|id| {
                core.snapshot().motion.as_ref().is_some_and(|motion| {
                    motion.run_id == *id && motion.state == MovementPhase::Executing
                })
            }) else {
                return Ok(());
            };
            let anchor = self.status.active_anchor.clone().unwrap();
            let (definition, _) = core.prepare_pet_motion(
                &anchor,
                Some(&reaction.centre),
                object.azimuth,
                object.elevation,
                Some((run, now)),
                (4.7 - (now - reaction.started) - 0.6).max(0.0),
            )?;
            core.extend_character_performance(run, definition, anchor, now)?;
            self.surroundings.reaction.as_mut().unwrap().object =
                Some((object.id.clone(), now, object.azimuth, object.elevation));
            return Ok(());
        }
        let Some(object) = object else {
            return Ok(());
        };
        if !self.surroundings.settings.new_object_peek
            || object.capture_boottime_ns - object.since_boottime_ns < 3_000_000_000
            || self.surroundings.last_novelty.as_ref() == Some(&object.id)
        {
            return Ok(());
        }
        // Consume the observed event even when another owner or focus prevents
        // admission. It must not become a delayed surprise after conversation.
        self.surroundings.last_novelty = Some(object.id.clone());
        if self.surroundings.reaction.is_some() || !self.reaction_ready(core) {
            return Ok(());
        }
        let anchor = self
            .status
            .active_anchor
            .clone()
            .ok_or_else(|| Error::InvalidState("Peek needs an anchor".into()))?;
        let pet = matches!(object.category.as_str(), "cat" | "dog");
        let (definition, centre) = if pet {
            core.prepare_pet_motion(&anchor, None, object.azimuth, object.elevation, None, 3.0)?
        } else {
            let Some(point) = object.point else {
                return Ok(());
            };
            (core.prepare_peek_motion(&anchor, point)?, anchor.clone())
        };
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
            .try_spend(now, &object.interest_key, CuriosityKind::Gesture, context)
            .is_err()
        {
            return Ok(());
        }
        self.preempt_idle_or_thinking(now, core)?;
        let run = core.play_generated_anchored_relative(definition, anchor, now)?;
        self.surroundings.reaction = Some(Reaction {
            kind: if pet {
                ReactionKind::Pet
            } else {
                ReactionKind::Peek
            },
            track: 0,
            started: now,
            deadline: now + 12.0,
            run: Some(run),
            stage: Stage::Notice,
            centre,
            object: Some((object.id.clone(), now, object.azimuth, object.elevation)),
        });
        self.status.active_clip = Some(if pet { "pet_notice" } else { "peek_new_object" }.into());
        self.reset_timers(now);
        Ok(())
    }

    pub(crate) fn curiosity_light_intensity(&self, now: f64) -> f64 {
        self.surroundings
            .reaction
            .as_ref()
            .filter(|reaction| {
                reaction.kind == ReactionKind::Arrival
                    && reaction.stage != Stage::Returning
                    && self.surroundings.runtime_allowed
                    && self.surroundings.fresh(now)
            })
            .map_or(0.55, |reaction| {
                0.55 + 0.25
                    * (std::f64::consts::PI * ((now - reaction.started) / 3.0).clamp(0.0, 1.0))
                        .sin()
                        .powi(2)
            })
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
        core.fit_character_comfort(&mut definition, anchor)?;
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
                || reaction.kind != ReactionKind::Mutual
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
            || !match reaction.kind {
                ReactionKind::Mutual => self.surroundings.settings.mutual_gaze,
                ReactionKind::Arrival => self.surroundings.settings.arrival_notice,
                ReactionKind::Peek | ReactionKind::Pet => {
                    self.surroundings.settings.new_object_peek
                }
            };
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
    fn arrival_requires_ten_minutes_of_observed_emptiness_and_never_replays() {
        let mut policy = ArrivalPolicy::default();
        assert!(policy.observe(&[face(1, true)], 0.0).is_none());
        for second in 1..=600 {
            assert!(policy.observe(&[], second as f64).is_none());
        }
        assert_eq!(policy.observe(&[face(1, true)], 601.0).unwrap().track_id, 1);
        assert!(policy.observe(&[face(2, true)], 602.0).is_none());
        for second in 603..=1202 {
            policy.observe(&[], second as f64);
        }
        // A camera stall is unknown time, not proof of an empty room.
        assert!(policy.observe(&[face(3, true)], 1205.0).is_none());
        for second in 1206..=1806 {
            policy.observe(&[], second as f64);
        }
        policy.clear();
        assert!(policy.observe(&[face(3, true)], 1807.0).is_none());
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
        for name in [
            "arrival_notice",
            "peek_new_object",
            "pet_notice",
            "mutual_gaze_notice",
            "mutual_gaze_hi",
            "mutual_gaze_away",
        ] {
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
    fn arrival_faces_a_real_bearing_lifts_light_and_restores_its_anchor() {
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
        for step in 1..=250 {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
        }
        character.set_curiosity_available(true);
        let anchor = character.status.active_anchor.clone().unwrap();
        let bearing = crate::control::camera_rays::Bearing::from_direction(
            core.camera_kinematics()
                .unwrap()
                .fk(&anchor)
                .unwrap()
                .camera
                .rotate_vector([0.0, 0.0, 1.0]),
        );
        let observe = |now: f64,
                       present: bool,
                       character: &mut CharacterCoordinator,
                       core: &mut RuntimeCore<_>| {
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            let faces = if present {
                serde_json::json!([{"track_id":1, "azimuth":bearing.azimuth + 0.15,
                "elevation":bearing.elevation, "frontal":true}])
            } else {
                serde_json::json!([])
            };
            let value = serde_json::from_value(
                serde_json::json!({"capture_boottime_ns":boot.load(Ordering::Relaxed),
                "camera_on":true,"tracking_mode":"faces","settings":{"arrival_notice":true},
                "objects":[],"faces":faces,"looking_away":null}),
            )
            .unwrap();
            character.observe_surroundings(value, now, core).unwrap();
        };
        for step in 0..1200 {
            observe(5.0 + step as f64 * 0.5, false, &mut character, &mut core);
        }
        assert!(character.surroundings.reaction.is_none());
        observe(605.0, true, &mut character, &mut core);
        assert_eq!(
            character.status.active_clip.as_deref(),
            Some("arrival_notice")
        );
        assert_eq!(character.curiosity_light_intensity(605.0), 0.55);
        assert!((character.curiosity_light_intensity(606.5) - 0.8).abs() < 1e-9);
        let mut reached_attentive = false;
        for step in 1..=500 {
            let now = 605.0 + step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
            if step % 25 == 0 {
                observe(now, true, &mut character, &mut core);
            }
            reached_attentive |= core.snapshot().joints.iter().any(|joint| {
                joint.name == "shoulder_pitch_joint" && joint.position < anchor[&joint.name] - 0.04
            });
        }
        assert!(reached_attentive);
        assert!(character.surroundings.reaction.is_none());
        assert_eq!(character.curiosity_light_intensity(615.), 0.55);
        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
        for joint in &core.snapshot().joints {
            assert!((joint.position - anchor[&joint.name]).abs() < 0.01);
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
    fn novelty_fixture() -> (
        RuntimeCore<crate::expression::gaze::tests::FollowingDriver>,
        CharacterCoordinator,
        std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) {
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
        for step in 1..=250 {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
        }
        character.set_curiosity_available(true);
        (core, character, boot)
    }
    fn novel(category: &str, now: f64, since: f64, id: &str, azimuth: f64) -> NovelObject {
        NovelObject {
            id: id.into(),
            interest_key: format!("object:{category}:0:-2"),
            category: category.into(),
            azimuth,
            elevation: -0.6,
            point: Some([0., -0.28, 0.]),
            capture_boottime_ns: 1_000_000_000 + (now * 1e9) as u64,
            since_boottime_ns: 1_000_000_000 + (since * 1e9) as u64,
        }
    }
    fn observe_object(
        category: &str,
        now: f64,
        since: f64,
        id: &str,
        azimuth: f64,
        character: &mut CharacterCoordinator,
        core: &mut RuntimeCore<crate::expression::gaze::tests::FollowingDriver>,
    ) {
        let value = serde_json::from_value(serde_json::json!({
            "capture_boottime_ns":1_000_000_000 + (now * 1e9) as u64,
            "camera_on":true, "tracking_mode":"faces", "settings":{"new_object_peek":true},
            "objects":[],"faces":[],"looking_away":null,
            "novelty":{"id":id,"interest_key":format!("object:{category}:0:-2"),"category":category,
                "azimuth":azimuth,"elevation":-0.6,"point":[0.,-0.28,0.],
                "capture_boottime_ns":1_000_000_000 + (now * 1e9) as u64,
                "since_boottime_ns":1_000_000_000 + (since * 1e9) as u64}
        }))
        .unwrap();
        character.observe_surroundings(value, now, core).unwrap();
    }
    #[test]
    fn peek_requires_dwell_makes_a_visible_reach_and_returns_without_committing() {
        let (mut core, mut character, boot) = novelty_fixture();
        let anchor = character.status.active_anchor.clone().unwrap();
        let mut peak_shoulder = anchor["shoulder_pitch_joint"];
        for step in 251..=850 {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            if step % 25 == 0 {
                observe_object("book", now, 5., "object:1", 0.06, &mut character, &mut core);
            }
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
            if now < 8. {
                assert!(character.surroundings.reaction.is_none());
            }
            if step == 400 {
                assert_eq!(
                    character.status.active_clip.as_deref(),
                    Some("peek_new_object")
                );
            }
            let shoulder = core
                .snapshot()
                .joints
                .iter()
                .find(|j| j.name == "shoulder_pitch_joint")
                .unwrap()
                .position;
            peak_shoulder = peak_shoulder.max(shoulder);
        }
        assert!(
            peak_shoulder > anchor["shoulder_pitch_joint"] + 0.4,
            "peek must commit to its reach: {peak_shoulder}"
        );
        assert!(
            character.surroundings.reaction.is_none(),
            "reaction {:?}, motion {:?}",
            character.surroundings.reaction,
            core.snapshot().motion
        );
        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
        for joint in &core.snapshot().joints {
            assert!((joint.position - anchor[&joint.name]).abs() < 0.01);
        }
        // New tracker identity at the same object cannot evade habituation.
        character.surroundings.settings.new_object_peek = true;
        character
            .observe_novelty(
                Some(&novel("book", 80., 77., "object:99", 0.06)),
                80.,
                &mut core,
            )
            .unwrap();
        assert!(character.surroundings.reaction.is_none());
    }
    #[test]
    fn pet_lift_tracks_one_episode_and_privacy_settles_its_owned_return() {
        let (mut core, mut character, boot) = novelty_fixture();
        let anchor = character.status.active_anchor.clone().unwrap();
        let mut run = None;
        for step in 251..=550 {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            if step % 25 == 0 {
                observe_object(
                    "cat",
                    now,
                    5.,
                    "object:2",
                    if now >= 10. { 0.3 } else { 0.06 },
                    &mut character,
                    &mut core,
                );
            }
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
            if step == 400 {
                assert_eq!(character.status.active_clip.as_deref(), Some("pet_notice"));
                run = character.surroundings.reaction.as_ref().unwrap().run;
            }
        }
        let reaction = character.surroundings.reaction.as_ref().unwrap();
        assert_eq!(reaction.kind, ReactionKind::Pet);
        assert_eq!(reaction.run, run);
        assert_eq!(
            reaction.object.as_ref().unwrap().2,
            0.3,
            "fresh pet bearing must retarget"
        );
        character.clear_surroundings(11., &mut core).unwrap();
        for step in 551..=950 {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
        }
        assert!(character.surroundings.reaction.is_none());
        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
        for joint in &core.snapshot().joints {
            assert!((joint.position - anchor[&joint.name]).abs() < 0.01);
        }
    }
    #[test]
    fn blocked_or_unreachable_novelty_is_consumed_and_does_not_move_later() {
        let (mut core, mut character, _) = novelty_fixture();
        character.surroundings.settings.new_object_peek = true;
        let object = novel("book", 8., 5., "object:1", 0.06);
        character.set_curiosity_available(false);
        character
            .observe_novelty(Some(&object), 8., &mut core)
            .unwrap();
        character.set_curiosity_available(true);
        character
            .observe_novelty(Some(&object), 9., &mut core)
            .unwrap();
        assert!(character.surroundings.reaction.is_none());
        let mut unreachable = novel("book", 10., 5., "object:2", 0.06);
        unreachable.point = Some([0., 0.28, 0.]); // Behind the base.
        let _ = character.observe_novelty(Some(&unreachable), 10., &mut core);
        assert!(character.surroundings.reaction.is_none());
        assert_eq!(
            character.surroundings.last_novelty.as_deref(),
            Some("object:2")
        );
        // A declined plan has not consumed a valid opportunity's budget.
        character
            .observe_novelty(
                Some(&novel("book", 11., 5., "object:3", 0.06)),
                11.,
                &mut core,
            )
            .unwrap();
        assert!(character.surroundings.reaction.is_some());
    }
}
