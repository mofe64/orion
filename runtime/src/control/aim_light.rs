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
        comfort.validate_sequence(&sequence)
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
        let centre = self.measured_positions();
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
        let start = self.measured_positions();
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
        let start = self.measured_positions();
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
        let start = self.measured_positions();
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
        )?;
        Ok((solution, Some(prepared)))
    }

    pub(crate) fn prepare_work_return(&self, anchor: &JointPositions) -> Result<PreparedAim> {
        self.prepare_work_motion("aim_light_return", vec![(anchor.clone(), 3.5)])
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
        self.prepare_work_motion("look_around", frames)
    }

    fn prepare_work_motion(
        &self,
        name: &str,
        targets: Vec<(JointPositions, f64)>,
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
        let last = targets.len() - 1;
        let mut style = MotionStyle::named("speaking_calm")?;
        // Explicit staging carries the idea. Independent joint lag could bow
        // the shoulder/elbow spline outside the coupled comfort polygon.
        style.joint_lag = 0.;
        style.overshoot_scale = 0.;
        let definition = MotionDefinition {
            name: name.into(),
            description: "Owned desk-light movement".into(),
            space: MotionSpace::Absolute,
            style,
            return_to_anchor: false,
            keyframes: targets
                .into_iter()
                .enumerate()
                .map(|(i, (target, duration_seconds))| MotionKeyframe {
                    pose_name: None,
                    target,
                    duration_seconds,
                    arrival: if i == last {
                        KeyframeArrival::Settle
                    } else {
                        KeyframeArrival::Through
                    },
                    hold_seconds: 0.,
                    marker: None,
                })
                .collect(),
        };
        let sequence = MotionSequence::compile_scaled_calibrated(
            &definition,
            start.clone(),
            self.measured_velocities(),
            None,
            start,
            1.,
            &self.driver.joint_limits()?,
        )?;
        comfort.validate_sequence(&sequence)?;
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
