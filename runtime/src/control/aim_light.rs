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
