//! Command-time planning and installation; no IK runs in the control tick.
use super::*;
use crate::motion::ik::{ComfortBox, ComfortBoxConfig, LeanSolution, solve_lean};
use crate::{MotionKeyframe, MotionStyle};

pub(crate) struct PreparedAim {
    sequence: MotionSequence,
    target: JointPositions,
    name: String,
}

impl<D: RuntimeDriver> RuntimeCore<D> {
    pub(crate) fn validate_curiosity_motion(
        &self,
        definition: &MotionDefinition,
        anchor: &JointPositions,
        replacing: Option<(u64, f64)>,
    ) -> Result<()> {
        let limits = self.driver.joint_limits()?;
        let scale = definition.uniform_amplitude_scale(anchor, &limits)?;
        let state = replacing
            .filter(|(id, _)| {
                self.active_movement
                    .as_ref()
                    .is_some_and(|motion| motion.status.run_id == *id)
            })
            .and_then(|(_, now)| {
                self.motion_sequence.as_ref().map(|sequence| {
                    sequence.sample_state((now - self.movement_started_at).max(0.0))
                })
            })
            .transpose()?;
        let (positions, velocities, accelerations) = state.map_or_else(
            || (self.measured_positions(), self.measured_velocities(), None),
            |state| (state.positions, state.velocities, Some(state.accelerations)),
        );
        let sequence = MotionSequence::compile_scaled_calibrated(
            definition,
            positions,
            velocities,
            accelerations,
            anchor.clone(),
            scale,
            &limits,
        )?;
        let comfort = &self
            .comfort
            .as_ref()
            .ok_or_else(|| Error::InvalidState("Comfort box unavailable".into()))?
            .1;
        comfort.validate_measured_sequence(&sequence, COMPLETION_POSITION_TOLERANCE_RAD)
    }
    pub(crate) fn prepare_arrival_motion(
        &self,
        anchor: &JointPositions,
        azimuth: f64,
        elevation: f64,
    ) -> Result<MotionDefinition> {
        let model = self
            .camera_kinematics()
            .ok_or_else(|| Error::InvalidState("V2 geometry unavailable".into()))?;
        let comfort = &self
            .comfort
            .as_ref()
            .ok_or_else(|| Error::InvalidState("Comfort box unavailable".into()))?
            .1;
        let limits = self.driver.joint_limits()?;
        let centre = comfort.tracking_start(
            &self.measured_positions(),
            COMPLETION_POSITION_TOLERANCE_RAD,
        )?;
        let mut target = self.poses.pose("attentive")?.clone();
        let delta = azimuth - centre["base_yaw_joint"];
        target.insert(
            "base_yaw_joint".into(),
            centre["base_yaw_joint"] + delta.sin().atan2(delta.cos()).clamp(-0.35, 0.35),
        );
        let (target, clamped) =
            crate::expression::gaze::solve_aim(model, &target, azimuth, elevation, &limits)?;
        if clamped {
            return Err(Error::InvalidState(
                "Arrival is outside calibrated gaze reach".into(),
            ));
        }
        let target = comfort.project(&centre, &target)?.0;
        let head =
            crate::expression::gaze::solve_aim(model, &centre, azimuth, elevation, &limits)?.0;
        let head = comfort.project(&centre, &head)?.0;
        let mut definition = self.motions.motion("arrival_notice")?.clone();
        for frame in &mut definition.keyframes {
            let pose = match frame.marker.as_deref() {
                Some("arrival_head") => Some(&head),
                Some("arrival_attentive") => Some(&target),
                _ => None,
            };
            if let Some(pose) = pose {
                frame.target = pose
                    .iter()
                    .map(|(joint, position)| (joint.clone(), position - anchor[joint]))
                    .collect();
            }
        }
        self.validate_curiosity_motion(&definition, anchor, None)?;
        Ok(definition)
    }

    pub(crate) fn prepare_peek_motion(
        &self,
        anchor: &JointPositions,
        point: [f64; 3],
    ) -> Result<MotionDefinition> {
        let comfort = &self
            .comfort
            .as_ref()
            .ok_or_else(|| Error::InvalidState("Comfort box unavailable".into()))?
            .1;
        let model = self
            .camera_kinematics()
            .ok_or_else(|| Error::InvalidState("V2 geometry unavailable".into()))?;
        let solution = solve_lean(model, comfort, self.poses.pose("home")?, anchor, point, 0.8)?;
        let target = solution
            .positions
            .ok_or_else(|| Error::InvalidState("Object outside desk reach".into()))?;
        let start = comfort.tracking_start(
            &self.measured_positions(),
            COMPLETION_POSITION_TOLERANCE_RAD,
        )?;
        let mut pullback = start.clone();
        let mut lead = start.clone();
        for joint in ["shoulder_pitch_joint", "elbow_pitch_joint"] {
            pullback.insert(
                joint.into(),
                start[joint] - 0.08 * (target[joint] - start[joint]),
            );
            lead.insert(
                joint.into(),
                start[joint] + 0.35 * (target[joint] - start[joint]),
            );
        }
        let pullback = comfort.project(&start, &pullback)?.0;
        let mut curious = target.clone();
        *curious.get_mut("head_roll_joint").unwrap() += 0.06;
        let curious = comfort.project(&target, &curious)?.0;
        let mut definition = self.motions.motion("peek_new_object")?.clone();
        for frame in &mut definition.keyframes {
            let pose = match frame.marker.as_deref() {
                Some("pullback") => &pullback,
                Some("arm_lead") => &lead,
                Some("reach") => &target,
                Some("curious") => &curious,
                _ => anchor,
            };
            frame.target = pose
                .iter()
                .map(|(joint, position)| (joint.clone(), position - anchor[joint]))
                .collect();
        }
        // Explicit drawings stage the arm and head. Synchronous settle curves
        // keep interpolation inside the convex shoulder/elbow envelope.
        definition.style.joint_lag = 0.0;
        definition.style.tempo = 1.0;
        definition.style.amplitude = 1.0;
        self.validate_curiosity_motion(&definition, anchor, None)?;
        Ok(definition)
    }

    pub(crate) fn prepare_pet_motion(
        &self,
        anchor: &JointPositions,
        centre: Option<&JointPositions>,
        azimuth: f64,
        elevation: f64,
        replacing: Option<(u64, f64)>,
        tracking_hold: f64,
    ) -> Result<(MotionDefinition, JointPositions)> {
        let comfort = &self
            .comfort
            .as_ref()
            .ok_or_else(|| Error::InvalidState("Comfort box unavailable".into()))?
            .1;
        let model = self
            .camera_kinematics()
            .ok_or_else(|| Error::InvalidState("V2 geometry unavailable".into()))?;
        let start = comfort.tracking_start(
            &self.measured_positions(),
            COMPLETION_POSITION_TOLERANCE_RAD,
        )?;
        let lift = match centre {
            Some(centre) => centre.clone(),
            None => {
                comfort
                    .project(
                        &start,
                        &self.motions.motion("delight_lift")?.final_target(anchor)?,
                    )?
                    .0
            }
        };
        let mut facing = lift.clone();
        let delta = azimuth - lift["base_yaw_joint"];
        *facing.get_mut("base_yaw_joint").unwrap() +=
            delta.sin().atan2(delta.cos()).clamp(-0.10, 0.10);
        let solved = crate::expression::gaze::solve_aim(
            model,
            &facing,
            azimuth,
            elevation,
            &self.driver.joint_limits()?,
        )?
        .0;
        for joint in ["head_roll_joint", "head_pitch_joint"] {
            facing.insert(
                joint.into(),
                lift[joint] + (solved[joint] - lift[joint]).clamp(-0.12, 0.12),
            );
        }
        let facing = comfort.project(&lift, &facing)?.0;
        let mut head = facing.clone();
        head.insert("base_yaw_joint".into(), lift["base_yaw_joint"]);
        let mut compression = start.clone();
        for joint in ["shoulder_pitch_joint", "elbow_pitch_joint"] {
            compression.insert(
                joint.into(),
                start[joint] - 0.06 * (lift[joint] - start[joint]),
            );
        }
        let compression = comfort.project(&start, &compression)?.0;
        let mut definition = self.motions.motion("pet_notice")?.clone();
        if centre.is_some() {
            definition.keyframes.drain(..2);
        }
        for frame in &mut definition.keyframes {
            let pose = match frame.marker.as_deref() {
                Some("compression") => &compression,
                Some("delight") => &lift,
                Some("pet_head") => &head,
                Some("pet_track") => {
                    frame.hold_seconds = tracking_hold;
                    &facing
                }
                _ => anchor,
            };
            frame.target = pose
                .iter()
                .map(|(joint, position)| (joint.clone(), position - anchor[joint]))
                .collect();
        }
        definition.style.joint_lag = 0.0;
        definition.style.tempo = 1.0;
        definition.style.amplitude = 1.0;
        self.validate_curiosity_motion(&definition, anchor, replacing)?;
        Ok((definition, lift))
    }

    pub(crate) fn set_character_comfort(&mut self, enabled: bool) {
        self.character_comfort = enabled;
    }
    /// Shrink every offset by one factor so each keyframe lies inside the
    /// comfort box. The clip keeps its shape, arrivals, overlap and overshoot;
    /// the caller still validates the compiled curve and falls back to
    /// `fit_character_comfort` when the curve bows outside.
    pub(crate) fn scale_character_comfort(
        &self,
        definition: &mut MotionDefinition,
        anchor: &JointPositions,
    ) -> Result<()> {
        let comfort = &self
            .comfort
            .as_ref()
            .ok_or_else(|| Error::InvalidState("Comfort box unavailable".into()))?
            .1;
        let mut scale = 1.0_f64;
        for target in definition.resolved_targets(anchor)? {
            scale = scale.min(comfort.project(anchor, &target)?.1);
        }
        if scale < 1.0 {
            for frame in &mut definition.keyframes {
                for offset in frame.target.values_mut() {
                    *offset *= scale;
                }
            }
        }
        Ok(())
    }
    /// Last resort: project each keyframe separately and stop at every one, so
    /// each segment is a straight joint-space line inside the convex box.
    pub(crate) fn fit_character_comfort(
        &self,
        definition: &mut MotionDefinition,
        anchor: &JointPositions,
    ) -> Result<()> {
        let comfort = &self
            .comfort
            .as_ref()
            .ok_or_else(|| Error::InvalidState("Comfort box unavailable".into()))?
            .1;
        let targets = definition.resolved_targets(anchor)?;
        let amplitude = definition.style.amplitude;
        for (frame, target) in definition.keyframes.iter_mut().zip(targets) {
            let target = comfort.project(anchor, &target)?.0;
            frame.target = target
                .into_iter()
                .map(|(joint, value)| {
                    let offset = if amplitude > 1e-12 {
                        (value - anchor[&joint]) / amplitude
                    } else {
                        0.0
                    };
                    (joint, offset)
                })
                .collect();
            frame.arrival = KeyframeArrival::Settle;
        }
        definition.style.joint_lag = 0.0;
        definition.style.overshoot_scale = 0.0;
        Ok(())
    }
    pub(crate) fn hands_light_target(
        &self,
        anchor: &JointPositions,
        point: [f64; 3],
    ) -> Result<JointPositions> {
        let comfort = &self
            .comfort
            .as_ref()
            .ok_or_else(|| Error::InvalidState("Comfort box unavailable".into()))?
            .1;
        let model = self
            .camera_kinematics()
            .ok_or_else(|| Error::InvalidState("Hands needs V2 geometry".into()))?;
        let solution = solve_lean(
            model,
            comfort,
            self.poses.pose("home")?,
            anchor,
            point,
            0.35,
        )?;
        solution
            .positions
            .ok_or_else(|| Error::InvalidState("Work point outside desk reach".into()))
    }
    pub(crate) fn prepare_hands_motion(
        &self,
        name: &str,
        target: &JointPositions,
        replacing: Option<(u64, f64)>,
    ) -> Result<MotionDefinition> {
        let comfort = &self
            .comfort
            .as_ref()
            .ok_or_else(|| Error::InvalidState("Comfort box unavailable".into()))?
            .1;
        let start = comfort.tracking_start(
            &self.measured_positions(),
            COMPLETION_POSITION_TOLERANCE_RAD,
        )?;
        let mut pullback = start.clone();
        let mut lead = start.clone();
        for joint in ["shoulder_pitch_joint", "elbow_pitch_joint"] {
            pullback.insert(
                joint.into(),
                start[joint] - 0.04 * (target[joint] - start[joint]),
            );
            lead.insert(
                joint.into(),
                start[joint] + 0.25 * (target[joint] - start[joint]),
            );
        }
        let pullback = comfort.project(&start, &pullback)?.0;
        let mut definition = self.motions.motion(name)?.clone();
        definition.space = MotionSpace::AnchorRelative;
        definition.return_to_anchor = true;
        for frame in &mut definition.keyframes {
            frame.pose_name = None;
            let pose = match frame.marker.as_deref() {
                Some("anticipation") => &pullback,
                Some("arm_lead") => &lead,
                _ => target,
            };
            frame.target = pose
                .iter()
                .map(|(joint, value)| (joint.clone(), value - target[joint]))
                .collect();
        }
        definition.style.joint_lag = 0.0;
        definition.style.amplitude = 1.0;
        self.validate_curiosity_motion(&definition, target, replacing)?;
        Ok(definition)
    }

    pub fn configure_comfort_box(&mut self, config: ComfortBoxConfig) -> Result<()> {
        let comfort = ComfortBox::from_config(&config, &self.poses, &self.driver.joint_limits()?)?;
        self.comfort = Some((config, comfort));
        Ok(())
    }

    pub(crate) fn prepare_light_target(
        &self,
        anchor: &JointPositions,
        point: [f64; 3],
        lean: f64,
    ) -> Result<(LeanSolution, Option<PreparedAim>)> {
        let model = self
            .camera_kinematics()
            .ok_or_else(|| Error::InvalidState("V2 geometry is unavailable".into()))?;
        let comfort = &self
            .comfort
            .as_ref()
            .ok_or_else(|| Error::InvalidState("Comfort box is unavailable".into()))?
            .1;
        let solution = solve_lean(
            model,
            comfort,
            self.poses.pose("home")?,
            anchor,
            point,
            lean,
        )?;
        let Some(target) = solution.positions.as_ref() else {
            return Ok((solution, None));
        };
        let start = comfort.tracking_start(
            &self.measured_positions(),
            COMPLETION_POSITION_TOLERANCE_RAD,
        )?;
        // Arm leads the approach. The head joins only after the reach starts.
        // The bounded anticipation is projected toward the existing anchor.
        let mut pullback = start.clone();
        for joint in ["shoulder_pitch_joint", "elbow_pitch_joint"] {
            pullback.insert(
                joint.into(),
                start[joint] - 0.08 * (target[joint] - start[joint]),
            );
        }
        let (pullback, _) = comfort.project(&start, &pullback)?;
        let mut lead = start.clone();
        for joint in ["shoulder_pitch_joint", "elbow_pitch_joint"] {
            lead.insert(
                joint.into(),
                start[joint] + 0.35 * (target[joint] - start[joint]),
            );
        }
        let prepared = self.prepare_work_motion(
            "aim_light",
            vec![(pullback, 0.35), (lead, 0.65), (target.clone(), 2.5)],
            None,
        )?;
        Ok((solution, Some(prepared)))
    }

    pub(crate) fn prepare_work_return(&self, anchor: &JointPositions) -> Result<PreparedAim> {
        self.prepare_work_motion("aim_light_return", vec![(anchor.clone(), 3.5)], None)
    }

    pub(crate) fn prepare_work_scan(
        &self,
        anchor: &JointPositions,
        index: usize,
    ) -> Result<PreparedAim> {
        if index > 2 {
            return Err(Error::InvalidArgument(
                "Search stop must be 0, 1 or 2".into(),
            ));
        }
        let mut motion = self.motions.motion("look_around")?.clone();
        let stops: Vec<_> = motion
            .keyframes
            .iter()
            .enumerate()
            .filter(|(_, frame)| {
                frame
                    .marker
                    .as_deref()
                    .is_some_and(|m| m.starts_with("search_"))
            })
            .map(|(i, _)| i)
            .collect();
        if stops.len() != 3 {
            return Err(Error::InvalidArgument(
                "look_around needs three search stops".into(),
            ));
        }
        let start = if index == 0 { 0 } else { stops[index - 1] + 1 };
        motion.keyframes = motion.keyframes[start..=stops[index]].to_vec();
        let comfort = &self
            .comfort
            .as_ref()
            .ok_or_else(|| Error::InvalidState("Comfort box is unavailable".into()))?
            .1;
        let frames = motion
            .resolved_targets(anchor)?
            .into_iter()
            .zip(&motion.keyframes)
            .map(|(pose, frame)| Ok((comfort.project(anchor, &pose)?.0, frame.duration_seconds)))
            .collect::<Result<Vec<_>>>()?;
        let authored = motion
            .keyframes
            .iter()
            .map(|frame| (frame.arrival, frame.hold_seconds))
            .collect();
        self.prepare_work_motion("look_around", frames, Some(authored))
    }

    /// `authored` carries each frame's arrival and hold; `None` flows through
    /// every frame except the last. If that curve bows outside the comfort
    /// box, every frame settles instead, so each segment is a straight
    /// joint-space line inside the convex box.
    fn prepare_work_motion(
        &self,
        name: &str,
        targets: Vec<(JointPositions, f64)>,
        authored: Option<Vec<(KeyframeArrival, f64)>>,
    ) -> Result<PreparedAim> {
        if !self.mode.torque_is_enabled() {
            return Err(Error::InvalidState(
                "Aiming requires powered holding torque".into(),
            ));
        }
        let comfort = &self
            .comfort
            .as_ref()
            .ok_or_else(|| Error::InvalidState("Comfort box is unavailable".into()))?
            .1;
        let start = self.measured_positions();
        let target = targets
            .last()
            .ok_or_else(|| Error::InvalidArgument("No work target".into()))?
            .0
            .clone();
        let mut style = MotionStyle::named("speaking_calm")?;
        // Explicit staging carries the idea. Independent joint lag could bow
        // the shoulder/elbow spline outside the coupled comfort polygon.
        style.joint_lag = 0.;
        style.overshoot_scale = 0.;
        let last = targets.len() - 1;
        let limits = self.driver.joint_limits()?;
        let compile = |all_settle: bool| -> Result<MotionSequence> {
            let definition = MotionDefinition {
                name: name.into(),
                description: "Owned desk-light movement".into(),
                space: MotionSpace::Absolute,
                style,
                return_to_anchor: false,
                keyframes: targets
                    .iter()
                    .enumerate()
                    .map(|(i, (target, duration_seconds))| {
                        let (arrival, hold_seconds) = match &authored {
                            Some(frames) => frames[i],
                            None => (KeyframeArrival::Through, 0.),
                        };
                        MotionKeyframe {
                            pose_name: None,
                            target: target.clone(),
                            duration_seconds: *duration_seconds,
                            arrival: if all_settle || i == last {
                                KeyframeArrival::Settle
                            } else {
                                arrival
                            },
                            hold_seconds,
                            marker: None,
                        }
                    })
                    .collect(),
            };
            let sequence = MotionSequence::compile_scaled_calibrated(
                &definition,
                start.clone(),
                self.measured_velocities(),
                None,
                start.clone(),
                1.,
                &limits,
            )?;
            comfort.validate_measured_sequence(&sequence, COMPLETION_POSITION_TOLERANCE_RAD)?;
            Ok(sequence)
        };
        let sequence = match compile(false) {
            Ok(sequence) => sequence,
            Err(_) => compile(true)?,
        };
        Ok(PreparedAim {
            sequence,
            target,
            name: name.into(),
        })
    }

    pub(crate) fn install_work_motion(&mut self, prepared: PreparedAim, now: f64) -> Result<u64> {
        if self.mode != RuntimeMode::Holding || self.active_movement.is_some() {
            return Err(Error::InvalidState(
                "Work request no longer owns holding torque".into(),
            ));
        }
        self.motion_sequence = Some(prepared.sequence);
        self.trajectory = None;
        self.movement_started_at = now;
        self.mode = RuntimeMode::Moving;
        self.begin_movement(&prepared.name, prepared.target)
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use crate::expression::{
        character::CharacterCoordinator,
        gaze::tests::{FollowingDriver, following},
    };

    fn fixture() -> (RuntimeCore<FollowingDriver>, JointPositions, JointPositions) {
        let (mut core, _, _) = following();
        core.configure_comfort_box(
            crate::HardwareVersion::V2
                .profile()
                .comfort_box
                .clone()
                .unwrap(),
        )
        .unwrap();
        let desk = core.poses.pose("desk_work").unwrap().clone();
        let anchor = core.hands_light_target(&desk, [0.14, -0.32, 0.]).unwrap();
        let mut measured = anchor.clone();
        *measured.get_mut("shoulder_pitch_joint").unwrap() += 0.02;
        assert!(!core.comfort.as_ref().unwrap().1.plans(&measured));
        core.driver.positions = measured.clone();
        core.tick(0.1).unwrap();
        (core, anchor, measured)
    }

    #[test]
    fn settled_tracking_error_allows_idle_follow_return_aim_peek_and_pet() {
        for action in ["idle", "follow", "return", "aim", "peek", "pet"] {
            let (mut core, anchor, measured) = fixture();
            core.set_character_comfort(true);
            match action {
                "idle" => {
                    let definition = core.motions.motion("idle_breathe").unwrap().clone();
                    core.play_generated_anchored_relative(definition, anchor, 0.1)
                        .unwrap();
                }
                "follow" | "return" => {
                    let target = if action == "follow" {
                        core.hands_light_target(&anchor, [-0.12, -0.35, 0.])
                            .unwrap()
                    } else {
                        core.poses.pose("home").unwrap().clone()
                    };
                    let definition = core
                        .prepare_hands_motion("hands_follow", &target, None)
                        .unwrap();
                    core.play_generated_anchored_relative(definition, target, 0.1)
                        .unwrap();
                }
                "aim" => {
                    let (_, prepared) = core
                        .prepare_light_target(&anchor, [0., -0.28, 0.], 0.8)
                        .unwrap();
                    core.install_work_motion(prepared.unwrap(), 0.1).unwrap();
                }
                "peek" => {
                    let definition = core.prepare_peek_motion(&anchor, [0., -0.28, 0.]).unwrap();
                    core.play_generated_anchored_relative(definition, anchor, 0.1)
                        .unwrap();
                }
                "pet" => {
                    let (definition, _) = core
                        .prepare_pet_motion(&anchor, None, 0., -0.5, None, 3.)
                        .unwrap();
                    core.play_generated_anchored_relative(definition, anchor, 0.1)
                        .unwrap();
                }
                _ => unreachable!(),
            }
            let sequence = core.motion_sequence.as_ref().unwrap();
            for (n, q) in sequence.sample(0.).unwrap() {
                assert!(
                    (q - measured[&n]).abs() < 1e-9,
                    "{action}: discontinuous start"
                );
            }
            let comfort = &core.comfort.as_ref().unwrap().1;
            let first = sequence.keyframe_arrival_time(0).unwrap();
            for i in 0..=100 {
                let time = first + (sequence.duration_seconds() - first) * i as f64 / 100.;
                assert!(
                    comfort.contains(&sequence.sample(time).unwrap()),
                    "{action} at {time}"
                );
            }
        }
    }

    fn moving_through(sequence: &MotionSequence, keyframe: usize) -> bool {
        let time = sequence.keyframe_arrival_time(keyframe).unwrap();
        sequence
            .sample_state(time)
            .unwrap()
            .velocities
            .values()
            .any(|v| v.abs() > 1e-3)
    }

    #[test]
    fn desk_reach_and_search_flow_through_their_internal_keyframes() {
        let (mut core, _, _) = following();
        core.configure_comfort_box(
            crate::HardwareVersion::V2
                .profile()
                .comfort_box
                .clone()
                .unwrap(),
        )
        .unwrap();
        let home = core.poses.pose("home").unwrap().clone();
        let (_, prepared) = core
            .prepare_light_target(&home, [0., -0.28, 0.], 0.8)
            .unwrap();
        let reach = prepared.unwrap().sequence;
        // The anticipation is a reversal (zero velocity by nature); the arm
        // lead then hands over to the reach without a stop.
        assert!(moving_through(&reach, 1));
        // The first search stop keeps look_around's authored 0.4 s hold.
        let scan = core.prepare_work_scan(&home, 0).unwrap().sequence;
        assert!(scan.duration_seconds() >= 0.35 + 0.65 + 1.6 + 0.4 - 1e-9);
    }

    #[test]
    fn desk_anchor_idle_keeps_its_authored_flow() {
        let (mut core, anchor, _) = fixture();
        core.set_character_comfort(true);
        let definition = core.motions.motion("idle_breathe").unwrap().clone();
        let internal = definition.keyframes.len() - 1;
        core.play_generated_anchored_relative(definition, anchor, 0.1)
            .unwrap();
        let sequence = core.motion_sequence.as_ref().unwrap();
        assert!((0..internal).any(|k| moving_through(sequence, k)));
    }

    #[test]
    fn rejected_generated_idle_does_not_escape_character_tick() {
        let (mut core, _, _) = following();
        core.configure_comfort_box(
            crate::HardwareVersion::V2
                .profile()
                .comfort_box
                .clone()
                .unwrap(),
        )
        .unwrap();
        let mut character = CharacterCoordinator::new(1);
        character.start(0., &mut core).unwrap();
        for i in 1..=250 {
            let t = i as f64 * 0.02;
            core.tick(t).unwrap();
            character
                .tick(t, &mut core, false, None, false, None, None)
                .unwrap();
        }
        core.set_character_comfort(true);
        core.driver
            .positions
            .insert("elbow_pitch_joint".into(), -0.75);
        core.tick(100.).unwrap();
        character
            .tick(100., &mut core, false, None, false, None, None)
            .unwrap();
        assert!(
            character
                .status()
                .last_rejection
                .as_ref()
                .unwrap()
                .contains("comfort recovery tolerance")
        );
        assert!(core.snapshot().motion.is_none());
        for i in 1..=100 {
            let t = 100. + i as f64 * 0.02;
            core.tick(t).unwrap();
            character
                .tick(t, &mut core, false, None, false, None, None)
                .unwrap();
        }
        assert!(core.snapshot().torque_enabled);
    }
}
