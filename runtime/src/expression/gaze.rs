//! Bounded bearing-to-keyframe planning; all execution stays in MotionSequence.
use crate::control::camera_rays::Bearing;
use crate::motion::kinematics::Kinematics;
use crate::{
    Error, JointLimit, JointPositions, KeyframeArrival, MotionDefinition, MotionKeyframe,
    MotionSequence, MotionSpace, MotionStyle, Result, RuntimeCore, RuntimeDriver,
};
use serde::{Deserialize, Serialize};

pub const GAZE_DEADBAND_RAD: f64 = 4.0 * std::f64::consts::PI / 180.0;
pub const GAZE_COMPILED_PEAK_RAD_S: f64 = 1.5;
pub const GAZE_NECK_STEP_SPEED_RAD_S: f64 = 2.5;
pub const GAZE_UPDATE_SECONDS: f64 = 0.1;
const BASE_CATCHUP_SPEED_RAD_S: f64 = 0.6;
const NECK_LEAD_SECONDS: f64 = 0.2;
const TARGET_MAX_AGE_NS: u64 = 1_000_000_000;
const AIM_SECONDS: f64 = 0.18;
const HOLD_SECONDS: f64 = 1.1;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GazeTarget {
    pub track_id: u64,
    pub capture_boottime_ns: u64,
    pub azimuth: f64,
    pub elevation: f64,
    pub confidence: f64,
    #[serde(default, rename = "class")]
    pub category: Option<String>,
    #[serde(default)]
    pub range: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GazeStatus {
    pub enabled: bool,
    pub peak_speed_rad_s: f64,
    pub base_step_speed_rad_s: f64,
    pub neck_step_speed_rad_s: f64,
    pub state: &'static str,
    pub stop_command: Option<&'static str>,
    pub track_id: Option<u64>,
    pub run_id: Option<u64>,
    pub clamped: bool,
    pub extensions: u64,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct GazeCoordinator {
    status: GazeStatus,
    availability: GazeAvailability,
    target: Option<GazeTarget>,
    last_capture: Option<u64>,
    started_at: f64,
    last_update: f64,
    explicit_clear: bool,
}

// Target loss and motion handover never change this startup policy/Stop latch.
#[derive(Debug, PartialEq, Eq)]
enum GazeAvailability {
    Off,
    Ready,
    Stopped(GazeStopCommand),
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GazeStopCommand {
    Stop,
    Disable,
}

impl Default for GazeCoordinator {
    fn default() -> Self {
        Self {
            status: GazeStatus {
                enabled: false,
                peak_speed_rad_s: GAZE_COMPILED_PEAK_RAD_S,
                base_step_speed_rad_s: BASE_CATCHUP_SPEED_RAD_S,
                neck_step_speed_rad_s: GAZE_NECK_STEP_SPEED_RAD_S,
                state: "off",
                stop_command: None,
                track_id: None,
                run_id: None,
                clamped: false,
                extensions: 0,
                error: None,
            },
            availability: GazeAvailability::Off,
            target: None,
            last_capture: None,
            started_at: 0.0,
            last_update: f64::NEG_INFINITY,
            explicit_clear: false,
        }
    }
}

impl GazeCoordinator {
    pub fn set_enabled(&mut self, enabled: bool) {
        self.availability = if enabled {
            GazeAvailability::Ready
        } else {
            GazeAvailability::Off
        };
        self.status.enabled = enabled;
        self.status.state = if enabled { "idle" } else { "off" };
        if !enabled {
            self.forget();
        }
    }
    pub(crate) fn set_unavailable(&mut self, error: String) {
        self.availability = GazeAvailability::Unavailable;
        self.status.enabled = false;
        self.status.error = Some(error);
        self.forget();
    }
    pub(crate) fn is_stopped(&self) -> bool {
        matches!(self.availability, GazeAvailability::Stopped(_))
    }
    pub(crate) fn wake_can_resume(&self) -> bool {
        self.availability == GazeAvailability::Stopped(GazeStopCommand::Stop)
    }
    pub(crate) fn suspend(&mut self, command: GazeStopCommand) {
        // Disable is stronger than Stop. A later Stop/clear cannot convert a
        // deliberate torque-off latch into one that voice can clear.
        if matches!(
            self.availability,
            GazeAvailability::Ready | GazeAvailability::Stopped(GazeStopCommand::Stop)
        ) {
            self.availability = GazeAvailability::Stopped(command);
        }
        self.status.enabled = false;
        self.forget();
    }
    pub(crate) fn resume(&mut self, powered: bool) -> Result<()> {
        if matches!(
            self.availability,
            GazeAvailability::Off | GazeAvailability::Unavailable
        ) {
            return Err(Error::InvalidState("Gaze is unavailable".into()));
        }
        if !powered {
            return Err(Error::InvalidState(
                "Gaze resume requires powered holding torque; use character start.".into(),
            ));
        }
        self.availability = GazeAvailability::Ready;
        self.status.enabled = true;
        self.forget();
        Ok(())
    }
    pub fn status(&self) -> GazeStatus {
        GazeStatus {
            stop_command: match self.availability {
                GazeAvailability::Stopped(GazeStopCommand::Stop) => Some("stop"),
                GazeAvailability::Stopped(GazeStopCommand::Disable) => Some("disable"),
                _ => None,
            },
            ..self.status.clone()
        }
    }
    pub fn active(&self) -> bool {
        self.target.is_some() || self.status.run_id.is_some()
    }
    pub fn queue<D: RuntimeDriver>(
        &mut self,
        target: GazeTarget,
        now: f64,
        core: &RuntimeCore<D>,
    ) -> Result<()> {
        self.validate_target(&target, core)?;
        if self
            .last_capture
            .is_some_and(|previous| target.capture_boottime_ns <= previous)
        {
            return Err(Error::InvalidArgument(
                "Gaze capture timestamps must increase".into(),
            ));
        }
        if !self.active() {
            self.started_at = now;
            self.last_update = f64::NEG_INFINITY;
        }
        self.last_capture = Some(target.capture_boottime_ns);
        self.status.track_id = Some(target.track_id);
        self.target = Some(target); // One latest-value slot; never queue a stream.
        self.explicit_clear = false;
        self.status.error = None;
        Ok(())
    }

    pub(crate) fn validate_target<D: RuntimeDriver>(
        &self,
        target: &GazeTarget,
        core: &RuntimeCore<D>,
    ) -> Result<()> {
        if !self.status.enabled {
            return Err(Error::InvalidState("Gaze is unavailable".into()));
        }
        let boot = core
            .camera_boottime_ns()
            .ok_or_else(|| Error::InvalidState("V2 gaze geometry/clock is unavailable".into()))?;
        if !target.azimuth.is_finite()
            || !target.elevation.is_finite()
            || target.azimuth.abs() > std::f64::consts::PI
            || target.elevation.abs() > std::f64::consts::FRAC_PI_2
            || !target.confidence.is_finite()
            || !(0.0..=1.0).contains(&target.confidence)
            || target
                .range
                .is_some_and(|range| !range.is_finite() || range <= 0.0)
            || target
                .category
                .as_ref()
                .is_some_and(|class| class.len() > 32)
            || target.capture_boottime_ns > boot.saturating_add(50_000_000)
            || boot.saturating_sub(target.capture_boottime_ns) > TARGET_MAX_AGE_NS
        {
            return Err(Error::InvalidArgument(
                "Invalid or stale gaze target bearing".into(),
            ));
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.target = None;
        self.status.track_id = None;
        self.explicit_clear = true;
    }

    /// One confirmed-wake turn. Its zero-offset settle is relative to an
    /// immutable facing anchor; CharacterCoordinator installs the measured
    /// anchor only after completion and owns the later conversational return.
    pub(crate) fn attention_motion<D: RuntimeDriver>(
        &self,
        bearing: Option<(f64, f64)>,
        previous: &JointPositions,
        core: &RuntimeCore<D>,
    ) -> Result<(MotionDefinition, JointPositions)> {
        if !self.status.enabled {
            return Err(Error::InvalidState("Gaze is unavailable".into()));
        }
        let limits = core.driver().joint_limits()?;
        let start = core.driver().clamp_positions_to_safe_range(
            &core
                .snapshot()
                .joints
                .iter()
                .map(|joint| (joint.name.clone(), joint.position))
                .collect(),
        )?;
        let velocity: JointPositions = core
            .snapshot()
            .joints
            .iter()
            .map(|joint| (joint.name.clone(), joint.velocity))
            .collect();
        let mut end = previous.clone();
        let mut stages = Vec::with_capacity(3);
        let name;
        if let Some((azimuth, elevation)) = bearing {
            if !azimuth.is_finite()
                || !elevation.is_finite()
                || azimuth.abs() > std::f64::consts::PI
                || elevation.abs() > std::f64::consts::FRAC_PI_2
            {
                return Err(Error::InvalidArgument("Invalid attention bearing".into()));
            }
            let model = core
                .camera_kinematics()
                .ok_or_else(|| Error::InvalidState("Gaze kinematics unavailable".into()))?;
            let (lower, upper) = bounds(&limits, "base_yaw_joint")?;
            let base = start["base_yaw_joint"];
            *end.get_mut("base_yaw_joint").unwrap() = (azimuth
                + model.joint_reference("base_yaw_joint")?)
            .clamp((base - 0.65).max(lower), (base + 0.65).min(upper));
            // Bound body travel, then use calibrated neck/pitch reach. Refuse
            // only a solved pose that cannot face the requested bearing.
            let (solved, clamped) = solve_aim(model, &end, azimuth, elevation, &limits)?;
            if clamped {
                return Err(Error::InvalidState(
                    "Attention bearing is outside calibrated neck reach after the bounded base turn.".into(),
                ));
            }
            end = solved;
            let mut neck_first = start.clone();
            for joint in ["shoulder_pitch_joint", "elbow_pitch_joint"] {
                *neck_first.get_mut(joint).unwrap() = previous[joint];
            }
            neck_first = solve_aim(model, &neck_first, azimuth, elevation, &limits)?.0;
            stages.push(neck_first);
            name = "gaze_attention";
        } else {
            let (lower, upper) = bounds(&limits, "head_roll_joint")?;
            for direction in [-1.0, 1.0] {
                let mut look = previous.clone();
                *look.get_mut("head_roll_joint").unwrap() =
                    (previous["head_roll_joint"] + direction * 0.18).clamp(lower, upper);
                stages.push(look);
            }
            name = "gaze_look_around";
        }
        stages.push(end.clone());
        let mut last = start.clone();
        let keyframes = stages
            .into_iter()
            .map(|pose| {
                let seconds = pose
                    .iter()
                    .map(|(joint, q)| 1.875 * (q - last[joint]).abs() / GAZE_COMPILED_PEAK_RAD_S)
                    .fold(0.3_f64, f64::max);
                last = pose.clone();
                MotionKeyframe {
                    pose_name: None,
                    target: pose
                        .iter()
                        .map(|(joint, q)| (joint.clone(), q - end[joint]))
                        .collect(),
                    duration_seconds: seconds,
                    arrival: KeyframeArrival::Settle,
                    hold_seconds: 0.05,
                    marker: None,
                }
            })
            .collect();
        let mut definition = MotionDefinition {
            name: name.into(),
            description: "Capture-grounded speaker attention with a measured facing settle".into(),
            space: MotionSpace::AnchorRelative,
            return_to_anchor: true,
            style: MotionStyle {
                name: "return_home",
                tempo: 1.0,
                tangent_tension: 0.0,
                joint_lag: 0.0,
                amplitude: 1.0,
                overshoot_scale: 0.0,
                settle_character: 1.0,
            },
            keyframes,
        };
        for _ in 0..4 {
            let scale = definition.uniform_amplitude_scale(&end, &limits)?;
            let compiled = MotionSequence::compile_scaled_calibrated(
                &definition,
                start.clone(),
                velocity.clone(),
                None,
                end.clone(),
                scale,
                &limits,
            )?;
            let peak = compiled.peak_velocity_rad_s();
            if peak <= GAZE_COMPILED_PEAK_RAD_S * (1.0 + 1e-9) {
                return Ok((definition, end));
            }
            for frame in &mut definition.keyframes {
                frame.duration_seconds *= peak / GAZE_COMPILED_PEAK_RAD_S * 1.02;
            }
        }
        Err(Error::InvalidState(
            "Attention cannot meet the gaze speed cap".into(),
        ))
    }

    pub(crate) fn forget(&mut self) {
        self.target = None;
        self.status.track_id = None;
        self.status.run_id = None;
        self.status.state = match self.availability {
            GazeAvailability::Off => "off",
            GazeAvailability::Ready => "idle",
            GazeAvailability::Stopped(_) => "stopped",
            GazeAvailability::Unavailable => "unavailable",
        };
        self.explicit_clear = false;
    }

    /// Higher-priority work cancels only the run we own. It never touches a
    /// scene or speech run that has already taken over, and never reports an error.
    pub fn yield_to_priority<D: RuntimeDriver>(&mut self, now: f64, core: &mut RuntimeCore<D>) {
        if self.status.run_id.is_some_and(|id| {
            core.snapshot()
                .motion
                .as_ref()
                .is_some_and(|run| run.run_id == id)
        }) {
            let _ = core.handle_command("stop", now);
        }
        self.forget();
    }

    /// Called outside RuntimeCore::tick. Planning has a fixed iteration budget
    /// and extends at most 10 times per second from the commanded spline state.
    pub fn tick<D: RuntimeDriver>(
        &mut self,
        now: f64,
        anchor: &JointPositions,
        core: &mut RuntimeCore<D>,
    ) -> bool {
        if !self.status.enabled {
            return false;
        }
        if let Some(id) = self.status.run_id {
            if !core
                .snapshot()
                .motion
                .as_ref()
                .is_some_and(|run| run.run_id == id)
            {
                let completed = core.snapshot().last_motion.as_ref().is_some_and(|run| {
                    run.run_id == id && run.state == crate::MovementPhase::Completed
                });
                if !completed {
                    self.forget();
                    return false;
                }
                self.status.run_id = None;
                if self.target.is_none() {
                    self.forget();
                    return false;
                }
            }
        }
        if let Some(target) = &self.target {
            if core.camera_boottime_ns().is_none_or(|boot| {
                boot.saturating_sub(target.capture_boottime_ns) > TARGET_MAX_AGE_NS
            }) {
                self.target = None;
                self.status.track_id = None;
                self.status.state = "settling";
            }
        }
        if !self.active() {
            return false;
        }
        if now - self.last_update + 1e-9 < GAZE_UPDATE_SECONDS {
            return true;
        }
        if self.target.is_none() && !self.explicit_clear {
            return self.status.run_id.is_some();
        }
        self.last_update = now;
        match self.plan(now, anchor, core) {
            Ok(Some(definition)) => {
                let run = match self.status.run_id {
                    Some(id) => {
                        core.extend_character_performance(id, definition, anchor.clone(), now)
                    }
                    None => core.play_generated_anchored_relative(definition, anchor.clone(), now),
                };
                match run {
                    Ok(id) => {
                        self.status.run_id = Some(id);
                        self.status.extensions += 1;
                        self.status.state = if self.target.is_some() {
                            "tracking"
                        } else {
                            "settling"
                        };
                        self.explicit_clear = false;
                    }
                    Err(error) => {
                        self.status.error = Some(error.to_string());
                        self.clear();
                    }
                }
            }
            Ok(None) => {} // Wait for safe handover or the last settle to finish.
            Err(error) => {
                self.status.error = Some(error.to_string());
                self.clear();
            }
        }
        self.active()
    }

    fn plan<D: RuntimeDriver>(
        &mut self,
        now: f64,
        anchor: &JointPositions,
        core: &RuntimeCore<D>,
    ) -> Result<Option<MotionDefinition>> {
        let commanded = self
            .status
            .run_id
            .and_then(|id| core.character_commanded_state(id, now));
        if self.status.run_id.is_some() && commanded.is_none() {
            return Ok(None);
        }
        let start = match &commanded {
            Some(state) => state.positions.clone(),
            None => core.driver().clamp_positions_to_safe_range(
                &core
                    .snapshot()
                    .joints
                    .iter()
                    .map(|joint| (joint.name.clone(), joint.position))
                    .collect(),
            )?,
        };
        let velocity = commanded
            .as_ref()
            .map(|state| state.velocities.clone())
            .unwrap_or_else(|| {
                core.snapshot()
                    .joints
                    .iter()
                    .map(|joint| (joint.name.clone(), joint.velocity))
                    .collect()
            });
        if velocity
            .values()
            .any(|v| v.abs() > GAZE_COMPILED_PEAK_RAD_S)
        {
            return Ok(None);
        }
        let acceleration = commanded.as_ref().map(|state| state.accelerations.clone());
        let limits = core.driver().joint_limits()?;
        let mut aim = start.clone();
        if let Some(target) = &self.target {
            for name in ["shoulder_pitch_joint", "elbow_pitch_joint"] {
                *aim.get_mut(name).unwrap() = anchor[name];
            }
            let model = core
                .camera_kinematics()
                .ok_or_else(|| Error::InvalidState("Gaze kinematics unavailable".into()))?;
            let current =
                Bearing::from_direction(model.fk(&aim)?.camera.rotate_vector([0.0, 0.0, 1.0]));
            let centred = (start["head_roll_joint"] - model.joint_reference("head_roll_joint")?)
                .abs()
                <= GAZE_DEADBAND_RAD;
            if angle_difference(target.azimuth, current.azimuth).abs() > GAZE_DEADBAND_RAD
                || (target.elevation - current.elevation).abs() > GAZE_DEADBAND_RAD
                || !centred
            {
                if now - self.started_at >= NECK_LEAD_SECONDS && !centred {
                    let (lower, upper) = bounds(&limits, "base_yaw_joint")?;
                    let desired = (target.azimuth + model.joint_reference("base_yaw_joint")?)
                        .clamp(lower, upper);
                    let step = BASE_CATCHUP_SPEED_RAD_S * GAZE_UPDATE_SECONDS / 1.875;
                    let base = start["base_yaw_joint"]
                        + (desired - start["base_yaw_joint"]).clamp(-step, step);
                    *aim.get_mut("base_yaw_joint").unwrap() = base;
                    // Counter-rotate as an initial guess; FK solves the tilted
                    // swivel geometry instead of treating joint angles as bearings.
                    *aim.get_mut("head_roll_joint").unwrap() -= base - start["base_yaw_joint"];
                }
                let (solved, _) =
                    solve_aim(model, &aim, target.azimuth, target.elevation, &limits)?;
                smooth_head(&start, &solved, &mut aim);
            }
            let achieved =
                Bearing::from_direction(model.fk(&aim)?.camera.rotate_vector([0., 0., 1.]));
            let (lower, upper) = bounds(&limits, "base_yaw_joint")?;
            let base_angle = target.azimuth + model.joint_reference("base_yaw_joint")?;
            self.status.clamped = base_angle != base_angle.clamp(lower, upper)
                || angle_difference(achieved.azimuth, target.azimuth).abs() > GAZE_DEADBAND_RAD
                || (achieved.elevation - target.elevation).abs() > GAZE_DEADBAND_RAD;
        }
        let return_seconds = settle_seconds(&start, anchor);
        let offsets = aim
            .iter()
            .map(|(name, value)| (name.clone(), value - anchor[name]))
            .collect();
        let definition = MotionDefinition {
            name: "gaze_performance".into(),
            description: "Capture-grounded gaze with an anchor return".into(),
            space: MotionSpace::AnchorRelative,
            return_to_anchor: true,
            style: MotionStyle {
                name: "return_home",
                tempo: 1.0,
                tangent_tension: 0.0,
                joint_lag: 0.0,
                amplitude: 1.0,
                overshoot_scale: 0.0,
                settle_character: 1.0,
            },
            keyframes: vec![
                MotionKeyframe {
                    pose_name: None,
                    target: offsets,
                    duration_seconds: AIM_SECONDS,
                    arrival: KeyframeArrival::Settle,
                    hold_seconds: if self.target.is_some() {
                        HOLD_SECONDS
                    } else {
                        0.2
                    },
                    marker: None,
                },
                MotionKeyframe {
                    pose_name: None,
                    target: anchor.keys().map(|name| (name.clone(), 0.0)).collect(),
                    duration_seconds: return_seconds,
                    arrival: KeyframeArrival::Settle,
                    hold_seconds: 0.0,
                    marker: None,
                },
            ],
        };
        cap_tracking_speed(definition, &start, &velocity, acceleration, anchor, &limits)
    }
}

/// Face and hand tracking share the same per-update head step and compiled cap.
pub(crate) fn smooth_head(
    start: &JointPositions,
    solved: &JointPositions,
    aim: &mut JointPositions,
) {
    let step = GAZE_NECK_STEP_SPEED_RAD_S * GAZE_UPDATE_SECONDS / 1.875;
    for name in ["head_roll_joint", "head_pitch_joint"] {
        *aim.get_mut(name).unwrap() = start[name] + (solved[name] - start[name]).clamp(-step, step);
    }
}

pub(crate) fn settle_seconds(start: &JointPositions, anchor: &JointPositions) -> f64 {
    start
        .iter()
        .map(|(name, q)| 1.875 * (q - anchor[name]).abs() / GAZE_COMPILED_PEAK_RAD_S)
        .fold(0.55_f64, f64::max)
}

pub(crate) fn cap_tracking_speed(
    mut definition: MotionDefinition,
    start: &JointPositions,
    velocity: &JointPositions,
    acceleration: Option<JointPositions>,
    anchor: &JointPositions,
    limits: &[JointLimit],
) -> Result<Option<MotionDefinition>> {
    for _ in 0..4 {
        let scale = definition.uniform_amplitude_scale(anchor, limits)?;
        let sequence = MotionSequence::compile_scaled_calibrated(
            &definition,
            start.clone(),
            velocity.clone(),
            acceleration.clone(),
            anchor.clone(),
            scale,
            limits,
        )?;
        let peak = sequence.peak_velocity_rad_s();
        if peak <= GAZE_COMPILED_PEAK_RAD_S * (1.0 + 1e-9) {
            return Ok(Some(definition));
        }
        for frame in &mut definition.keyframes {
            frame.duration_seconds *= peak / GAZE_COMPILED_PEAK_RAD_S * 1.02;
        }
    }
    Ok(None)
}

/// A hand step settles at its temporary target; ownership retains the held pose.
pub(crate) fn hand_step(
    start: &crate::TrajectorySample,
    target: &JointPositions,
    limits: &[JointLimit],
    returning: bool,
) -> Result<Option<MotionDefinition>> {
    let definition = MotionDefinition {
        name: "hands_track".into(),
        description: "Head tracking from the held pose".into(),
        space: MotionSpace::AnchorRelative,
        return_to_anchor: true,
        style: MotionStyle {
            name: "return_home",
            tempo: 1.0,
            tangent_tension: 0.0,
            joint_lag: 0.0,
            amplitude: 1.0,
            overshoot_scale: 0.0,
            settle_character: 1.0,
        },
        keyframes: vec![MotionKeyframe {
            pose_name: None,
            target: target.keys().map(|n| (n.clone(), 0.)).collect(),
            duration_seconds: if returning {
                settle_seconds(&start.positions, target)
            } else {
                AIM_SECONDS
            },
            arrival: KeyframeArrival::Settle,
            hold_seconds: 0.,
            marker: None,
        }],
    };
    cap_tracking_speed(
        definition,
        &start.positions,
        &start.velocities,
        Some(start.accelerations.clone()),
        target,
        limits,
    )
}

fn bounds(limits: &[JointLimit], name: &str) -> Result<(f64, f64)> {
    let limit = limits
        .iter()
        .find(|limit| limit.name == name)
        .ok_or_else(|| Error::InvalidState(format!("Missing gaze calibration for {name}")))?;
    Ok((limit.lower_rad, limit.upper_rad))
}
fn angle_difference(a: f64, b: f64) -> f64 {
    (a - b).sin().atan2((a - b).cos())
}

/// Two orientation degrees of freedom solved against URDF FK, at most four
/// seeds × eight Newton steps. Seeded limit branches choose the nearer reach.
pub(crate) fn solve_aim(
    model: &Kinematics,
    start: &JointPositions,
    azimuth: f64,
    elevation: f64,
    limits: &[JointLimit],
) -> Result<(JointPositions, bool)> {
    let names = ["head_roll_joint", "head_pitch_joint"];
    let ranges = [bounds(limits, names[0])?, bounds(limits, names[1])?];
    let direction = [
        -azimuth.sin() * elevation.cos(),
        -azimuth.cos() * elevation.cos(),
        elevation.sin(),
    ];
    let mut best = start.clone();
    let mut best_score = f64::NEG_INFINITY;
    for seed in [start[names[0]], 0.0, ranges[0].0, ranges[0].1] {
        let mut pose = start.clone();
        *pose.get_mut(names[0]).unwrap() = seed.clamp(ranges[0].0, ranges[0].1);
        for _ in 0..8 {
            let forward = model.fk(&pose)?.camera.rotate_vector([0.0, 0.0, 1.0]);
            let score: f64 = forward.into_iter().zip(direction).map(|(a, b)| a * b).sum();
            if score > best_score {
                best_score = score;
                best = pose.clone();
            }
            let bearing = Bearing::from_direction(forward);
            let error = [
                angle_difference(azimuth, bearing.azimuth),
                elevation - bearing.elevation,
            ];
            let mut jacobian = [[0.0; 2]; 2];
            for (index, name) in names.iter().enumerate() {
                let original = pose[*name];
                let delta = if original + 1e-4 <= ranges[index].1 {
                    1e-4
                } else {
                    -1e-4
                };
                *pose.get_mut(*name).unwrap() = original + delta;
                let changed =
                    Bearing::from_direction(model.fk(&pose)?.camera.rotate_vector([0.0, 0.0, 1.0]));
                jacobian[0][index] = angle_difference(changed.azimuth, bearing.azimuth) / delta;
                jacobian[1][index] = (changed.elevation - bearing.elevation) / delta;
                *pose.get_mut(*name).unwrap() = original;
            }
            let determinant = jacobian[0][0] * jacobian[1][1] - jacobian[0][1] * jacobian[1][0];
            if determinant.abs() < 1e-8 {
                break;
            }
            let delta = [
                (error[0] * jacobian[1][1] - error[1] * jacobian[0][1]) / determinant,
                (jacobian[0][0] * error[1] - jacobian[1][0] * error[0]) / determinant,
            ];
            for (index, name) in names.iter().enumerate() {
                *pose.get_mut(*name).unwrap() = (pose[*name] + delta[index].clamp(-0.5, 0.5))
                    .clamp(ranges[index].0, ranges[index].1);
            }
        }
    }
    Ok((best, best_score.clamp(-1.0, 1.0).acos() > GAZE_DEADBAND_RAD))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::control::camera_rays::BoottimeClock;
    use crate::{JointState, MotionLibrary, MujocoDriver, ORION_JOINT_NAMES, PoseLibrary};
    use std::path::{Path, PathBuf};
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    struct Clock(Arc<AtomicU64>);
    impl BoottimeClock for Clock {
        fn now_ns(&self) -> Option<u64> {
            Some(self.0.load(Ordering::Relaxed))
        }
    }
    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_owned()
    }
    fn libraries() -> (PoseLibrary, MotionLibrary) {
        let poses = PoseLibrary::load(
            root().join("motion/config/v2/poses.yaml"),
            &ORION_JOINT_NAMES,
        )
        .unwrap();
        let motions = MotionLibrary::load(root().join("motion/motions/v2"), &poses).unwrap();
        (poses, motions)
    }
    fn physical_limits() -> Vec<JointLimit> {
        crate::HardwareVersion::V2
            .load_calibration(root().join("hardware/profiles/v2-joints.json"))
            .unwrap()
            .iter()
            .map(|joint| {
                let (lower_rad, upper_rad) = joint.safe_range_radians();
                JointLimit {
                    name: joint.name.clone(),
                    lower_rad,
                    upper_rad,
                }
            })
            .collect()
    }

    pub(crate) struct FollowingDriver {
        pub(crate) positions: JointPositions,
        writes: Vec<JointPositions>,
        next_velocity: Arc<AtomicU64>,
        feedback_offsets: [AtomicU64; 2],
    }
    impl FollowingDriver {
        pub(crate) fn offset_feedback(&self, shoulder: f64, elbow: f64) {
            self.feedback_offsets[0].store(shoulder.to_bits(), Ordering::Relaxed);
            self.feedback_offsets[1].store(elbow.to_bits(), Ordering::Relaxed);
        }
    }
    impl RuntimeDriver for FollowingDriver {
        fn apply_servo_profile(&mut self) -> Result<()> {
            Ok(())
        }
        fn activate(&mut self) -> Result<Vec<JointState>> {
            self.read()
        }
        fn deactivate(&mut self) -> Result<()> {
            Ok(())
        }
        fn read(&mut self) -> Result<Vec<JointState>> {
            let velocity = f64::from_bits(self.next_velocity.swap(0, Ordering::Relaxed));
            Ok(self
                .positions
                .iter()
                .map(|(name, q)| JointState {
                    name: name.clone(),
                    position: *q
                        + match name.as_str() {
                            "shoulder_pitch_joint" => {
                                f64::from_bits(self.feedback_offsets[0].load(Ordering::Relaxed))
                            }
                            "elbow_pitch_joint" => {
                                f64::from_bits(self.feedback_offsets[1].load(Ordering::Relaxed))
                            }
                            _ => 0.,
                        },
                    velocity,
                    current_ma: 0.0,
                    voltage_v: 12.0,
                    temperature_c: 25.0,
                    status: 0,
                })
                .collect())
        }
        fn write(&mut self, positions: &JointPositions) -> Result<()> {
            self.validate_positions(positions)?;
            self.positions = positions.clone();
            self.writes.push(positions.clone());
            Ok(())
        }
        fn joint_limits(&self) -> Result<Vec<JointLimit>> {
            Ok(physical_limits())
        }
        fn validate_positions(&self, positions: &JointPositions) -> Result<()> {
            for limit in physical_limits() {
                if !(limit.lower_rad..=limit.upper_rad).contains(&positions[&limit.name]) {
                    return Err(Error::InvalidArgument(format!(
                        "Outside V2 calibration: {}",
                        limit.name
                    )));
                }
            }
            Ok(())
        }
        fn clamp_positions_to_safe_range(
            &self,
            positions: &JointPositions,
        ) -> Result<JointPositions> {
            self.validate_positions(positions)?;
            Ok(positions.clone())
        }
    }
    fn prepare<D: RuntimeDriver>(
        driver: D,
        poses: PoseLibrary,
        motions: MotionLibrary,
    ) -> (RuntimeCore<D>, Arc<AtomicU64>) {
        let mut core = RuntimeCore::new(driver, poses, motions).unwrap();
        let boot = Arc::new(AtomicU64::new(1_000_000_000));
        core.configure_camera_geometry(
            root().join("description/urdf/orion-v2.urdf"),
            Box::new(Clock(boot.clone())),
        )
        .unwrap();
        for command in ["configure", "enable"] {
            let result: serde_json::Value =
                serde_json::from_str(&core.handle_command(command, 0.0)).unwrap();
            assert_eq!(result["ok"], true);
        }
        (core, boot)
    }
    pub(crate) fn following() -> (RuntimeCore<FollowingDriver>, JointPositions, Arc<AtomicU64>) {
        let (poses, motions) = libraries();
        let anchor = poses.pose("home").unwrap().clone();
        let driver = FollowingDriver {
            positions: anchor.clone(),
            writes: Vec::new(),
            next_velocity: Arc::new(AtomicU64::new(0)),
            feedback_offsets: std::array::from_fn(|_| AtomicU64::new(0)),
        };
        let (core, boot) = prepare(driver, poses, motions);
        (core, anchor, boot)
    }
    fn target(boot: u64, azimuth: f64) -> GazeTarget {
        GazeTarget {
            track_id: 1,
            capture_boottime_ns: boot,
            azimuth,
            elevation: 0.0,
            confidence: 0.95,
            category: Some("face".into()),
            range: None,
        }
    }

    #[test]
    fn confirmed_wake_never_starts_a_disabled_or_cancelled_startup_character() {
        for commands in [&[][..], &["configure"], &["configure", "enable"]] {
            let (poses, motions) = libraries();
            let driver = FollowingDriver {
                positions: poses.pose("home").unwrap().clone(),
                writes: Vec::new(),
                next_velocity: Arc::new(AtomicU64::new(0)),
                feedback_offsets: std::array::from_fn(|_| AtomicU64::new(0)),
            };
            let mut core = RuntimeCore::new(driver, poses, motions).unwrap();
            for command in commands {
                let response: serde_json::Value =
                    serde_json::from_str(&core.handle_command(command, 0.)).unwrap();
                assert_eq!(response["ok"], true);
            }
            let mode = core.mode();
            let mut character = crate::CharacterCoordinator::new(1);
            character.set_gaze_enabled(true);
            character.forget_gaze_for_external_stop(GazeStopCommand::Stop);
            assert!(
                !character.resume_gaze_for_confirmed_wake(&core).unwrap(),
                "{commands:?}"
            );
            assert_eq!(core.mode(), mode);
            assert!(!character.status().enabled);
            assert!(core.snapshot().motion.is_none());
            assert!(core.driver().writes.is_empty());
            assert_eq!(character.gaze_status().state, "stopped");
        }
        let (mut core, _, _) = following();
        let mut character = crate::CharacterCoordinator::new(1);
        character.set_gaze_enabled(true);
        character.start(0., &mut core).unwrap();
        character.forget_gaze_for_external_stop(GazeStopCommand::Stop);
        let response: serde_json::Value =
            serde_json::from_str(&core.handle_command("stop", 0.1)).unwrap();
        assert_eq!(response["ok"], true);
        assert!(!character.resume_gaze_for_confirmed_wake(&core).unwrap());
        assert!(core.snapshot().motion.is_none());
        assert_eq!(character.gaze_status().state, "stopped");
    }

    #[test]
    fn disabled_visual_attention_reports_unavailable() {
        let (mut core, _, _) = following();
        let mut character = crate::CharacterCoordinator::new(1);
        let error = character
            .attend_bearing(Some((0.4, 0.)), 0., &mut core)
            .unwrap_err();
        assert!(error.to_string().contains("Gaze is unavailable"));
    }

    #[test]
    fn conversation_anchor_is_frozen_during_speech_and_retargets_afterward() {
        let (mut core, _, boot) = following();
        let mut character = crate::CharacterCoordinator::new(1);
        character.set_gaze_enabled(true);
        character.start(0.0, &mut core).unwrap();
        let tick = |step: u64,
                    speaking,
                    character: &mut crate::CharacterCoordinator,
                    core: &mut RuntimeCore<FollowingDriver>| {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            character
                .tick(now, core, false, None, speaking, None, None)
                .unwrap();
        };
        for step in 1..=250 {
            tick(step, false, &mut character, &mut core);
        }
        character
            .attend_tracked_bearing(Some((0.4, 0.0)), Some(1), 5.0, &mut core)
            .unwrap();
        for step in 251..=400 {
            tick(step, false, &mut character, &mut core);
        }
        let facing = character.status().active_anchor.clone().unwrap();
        assert!(
            (facing["base_yaw_joint"]
                - core
                    .camera_kinematics()
                    .unwrap()
                    .joint_reference("base_yaw_joint")
                    .unwrap()
                - 0.4)
                .abs()
                < 0.01
        );
        for step in 401..=450 {
            tick(step, true, &mut character, &mut core);
            assert!(
                !character
                    .queue_gaze_target(
                        target(boot.load(Ordering::Relaxed), 0.8),
                        step as f64 * 0.02,
                        &mut core
                    )
                    .unwrap()
            );
            assert_eq!(character.status().active_anchor.as_ref(), Some(&facing));
        }
        tick(451, false, &mut character, &mut core);
        assert!(
            character
                .queue_gaze_target(target(boot.load(Ordering::Relaxed), 0.8), 9.02, &mut core)
                .unwrap()
        );
        for step in 452..=600 {
            tick(step, false, &mut character, &mut core);
        }
        assert!(
            (character.status().active_anchor.as_ref().unwrap()["base_yaw_joint"]
                - core
                    .camera_kinematics()
                    .unwrap()
                    .joint_reference("base_yaw_joint")
                    .unwrap()
                - 0.8)
                .abs()
                < 0.01
        );
    }

    #[test]
    fn stopped_state_survives_clear_priority_yield_and_long_idle_until_resume() {
        let (mut core, anchor, boot) = following();
        let mut gaze = GazeCoordinator::default();
        gaze.set_enabled(true);
        gaze.suspend(GazeStopCommand::Stop);
        gaze.clear();
        gaze.yield_to_priority(0., &mut core);
        for seconds in 1..=120 {
            let now = seconds as f64;
            boot.store(seconds * 1_000_000_000, Ordering::Relaxed);
            core.tick(now).unwrap();
            assert!(
                gaze.queue(target(boot.load(Ordering::Relaxed), 0.4), now, &core)
                    .is_err()
            );
            assert!(!gaze.tick(now, &anchor, &mut core));
            assert!(core.snapshot().motion.is_none());
        }
        assert_eq!(gaze.status().state, "stopped");
        gaze.resume(core.mode().torque_is_enabled()).unwrap();
        assert_eq!(gaze.status().state, "idle");
        gaze.queue(target(boot.load(Ordering::Relaxed), 0.4), 120., &core)
            .unwrap();
        assert!(gaze.tick(120., &anchor, &mut core));
        assert!(core.snapshot().motion.is_some());
    }

    #[test]
    fn visual_speakers_at_sixty_and_one_hundred_degrees_use_bounded_base_and_neck() {
        for degrees in [60.0_f64, 100.0, -60.0, -100.0] {
            let (mut core, anchor, _) = following();
            let mut gaze = GazeCoordinator::default();
            gaze.set_enabled(true);
            let azimuth = degrees.to_radians();
            let (definition, facing) = gaze
                .attention_motion(Some((azimuth, 0.)), &anchor, &core)
                .unwrap();
            assert!((facing["base_yaw_joint"] - anchor["base_yaw_joint"]).abs() <= 0.65 + 1e-9);
            let aimed = Bearing::from_direction(
                core.camera_kinematics()
                    .unwrap()
                    .fk(&facing)
                    .unwrap()
                    .camera
                    .rotate_vector([0., 0., 1.]),
            );
            assert!(angle_difference(aimed.azimuth, azimuth).abs() < GAZE_DEADBAND_RAD);
            assert!(aimed.elevation.abs() < GAZE_DEADBAND_RAD);
            core.play_generated_anchored_relative(definition, facing.clone(), 0.)
                .unwrap();
            for tick in 1..=400 {
                core.tick(tick as f64 * 0.02).unwrap();
            }
            assert_eq!(
                core.snapshot().last_motion.as_ref().unwrap().state,
                crate::MovementPhase::Completed
            );
            for pose in &core.driver().writes {
                core.driver().validate_positions(pose).unwrap();
            }
            for (joint, value) in facing {
                assert!((core.driver().positions[&joint] - value).abs() < 1e-8);
            }
        }
    }

    #[test]
    fn visual_attention_preempts_gaze_before_using_refreshed_handover_velocity() {
        let (mut core, _, boot) = following();
        let mut character = crate::CharacterCoordinator::new(1);
        character.set_gaze_enabled(true);
        character.start(0., &mut core).unwrap();
        for tick in 1..=150 {
            let now = tick as f64 * 0.02;
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
        }
        character
            .queue_gaze_target(target(boot.load(Ordering::Relaxed), 0.4), 3., &mut core)
            .unwrap();
        character
            .tick(3., &mut core, false, None, false, None, None)
            .unwrap();
        let previous = character.gaze_status().run_id.unwrap();
        // One feedback read reports motion. Stop's refresh reads the stationary
        // handover; compiling before Stop incorrectly sees the old 2 rad/s.
        core.driver()
            .next_velocity
            .store(2.0_f64.to_bits(), Ordering::Relaxed);
        core.tick(3.02).unwrap();
        assert!(core.snapshot().joints[0].velocity > GAZE_COMPILED_PEAK_RAD_S);
        character
            .attend_bearing(Some((0.3, 0.1)), 3.02, &mut core)
            .unwrap();
        assert_eq!(
            core.snapshot().last_motion.as_ref().unwrap().run_id,
            previous
        );
        assert_eq!(
            core.snapshot().last_motion.as_ref().unwrap().state,
            crate::MovementPhase::Cancelled
        );
        assert_eq!(
            core.snapshot().motion.as_ref().unwrap().name,
            "gaze_attention"
        );
        assert!(
            core.snapshot()
                .joints
                .iter()
                .all(|joint| joint.velocity == 0.)
        );
    }

    #[test]
    fn visual_attention_compiles_neck_first_and_preserves_measured_anchor_lifecycle() {
        let (mut core, original, _) = following();
        let mut character = crate::CharacterCoordinator::new(1);
        character.set_gaze_enabled(true);
        character.start(0.0, &mut core).unwrap();
        for tick in 1..=150 {
            let now = tick as f64 * 0.02;
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
        }
        let previous = character.status().active_anchor.clone().unwrap();
        let gaze = {
            let mut gaze = GazeCoordinator::default();
            gaze.set_enabled(true);
            gaze
        };
        let (definition, facing) = gaze
            .attention_motion(Some((0.4, 0.1)), &previous, &core)
            .unwrap();
        assert!(
            definition
                .keyframes
                .last()
                .unwrap()
                .target
                .values()
                .all(|offset| *offset == 0.0)
        );
        let compiled = MotionSequence::compile_scaled_calibrated(
            &definition,
            original.clone(),
            original.keys().map(|name| (name.clone(), 0.0)).collect(),
            None,
            facing,
            1.0,
            &physical_limits(),
        )
        .unwrap();
        let early = compiled.sample(0.2).unwrap();
        let model = core.camera_kinematics().unwrap();
        assert!(
            (early["head_roll_joint"] - model.joint_reference("head_roll_joint").unwrap()).abs()
                > (early["base_yaw_joint"] - model.joint_reference("base_yaw_joint").unwrap())
                    .abs()
                    + 0.01
        );
        assert!(compiled.peak_velocity_rad_s() <= GAZE_COMPILED_PEAK_RAD_S * (1.0 + 1e-9));
        character
            .attend_bearing(Some((0.4, 0.1)), 3.0, &mut core)
            .unwrap();
        assert_eq!(character.status().active_anchor.as_ref(), Some(&previous));
        for tick in 151..=500 {
            let now = tick as f64 * 0.02;
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
        }
        let facing = character.status().active_anchor.clone().unwrap();
        let bearing = Bearing::from_direction(
            core.camera_kinematics()
                .unwrap()
                .fk(&facing)
                .unwrap()
                .camera
                .rotate_vector([0., 0., 1.]),
        );
        assert!((bearing.azimuth - 0.4).abs() < 0.01);
        assert!((bearing.elevation - 0.1).abs() < 0.01);
        character.set_reaction("neutral", 10.0, &mut core).unwrap();
        for tick in 501..=1500 {
            let now = tick as f64 * 0.02;
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
        }
        assert_eq!(character.status().active_anchor.as_ref(), Some(&previous));
        assert!(
            gaze.attention_motion(Some((1.0, 0.0)), &previous, &core)
                .is_ok()
        );
        assert!(
            gaze.attention_motion(Some((std::f64::consts::PI, 1.4)), &previous, &core)
                .is_err()
        );
    }

    #[test]
    fn no_face_scan_is_one_bounded_calibrated_run_returning_to_its_anchor() {
        let (mut core, anchor, _) = following();
        let mut gaze = GazeCoordinator::default();
        assert!(gaze.attention_motion(None, &anchor, &core).is_err());
        gaze.set_enabled(true);
        let (definition, end) = gaze.attention_motion(None, &anchor, &core).unwrap();
        assert_eq!(end, anchor);
        assert_eq!(definition.keyframes.len(), 3);
        assert_eq!(definition.name, "gaze_look_around");
        core.play_generated_anchored_relative(definition, end, 0.0)
            .unwrap();
        for tick in 1..=200 {
            core.tick(tick as f64 * 0.02).unwrap();
        }
        assert_eq!(
            core.snapshot().last_motion.as_ref().unwrap().state,
            crate::MovementPhase::Completed
        );
        for joint in &core.snapshot().joints {
            assert!((joint.position - anchor[&joint.name]).abs() < 1e-8);
        }
        assert!(
            core.driver()
                .writes
                .iter()
                .any(|q| q["head_roll_joint"] < -0.1)
        );
        assert!(
            core.driver()
                .writes
                .iter()
                .any(|q| q["head_roll_joint"] > 0.1)
        );
    }
    fn advance<D: RuntimeDriver>(
        gaze: &mut GazeCoordinator,
        core: &mut RuntimeCore<D>,
        anchor: &JointPositions,
        boot: &AtomicU64,
        tick: u64,
        bearing: Option<f64>,
    ) {
        let now = tick as f64 * 0.02;
        let time = 1_000_000_000 + tick * 20_000_000;
        boot.store(time, Ordering::Relaxed);
        core.tick(now).unwrap();
        if let Some(bearing) = bearing {
            gaze.queue(target(time, bearing), now, core).unwrap();
        }
        gaze.tick(now, anchor, core);
        assert!(gaze.status.error.is_none(), "{:?}", gaze.status);
    }

    #[test]
    fn bounded_aim_reports_the_achieved_azimuth_and_full_loop_respects_near_limits() {
        for azimuth in [2.9, -2.9, 1.6, -1.6, 3.1] {
            let (mut core, anchor, boot) = following();
            let mut gaze = GazeCoordinator::default();
            gaze.set_enabled(true);
            gaze.queue(target(1_000_000_000, azimuth), 0.0, &core)
                .unwrap();
            let plan = gaze.plan(0.0, &anchor, &core).unwrap().unwrap();
            let aim: JointPositions = plan.keyframes[0]
                .target
                .iter()
                .map(|(name, offset)| (name.clone(), anchor[name] + offset))
                .collect();
            let achieved = Bearing::from_direction(
                core.camera_kinematics()
                    .unwrap()
                    .fk(&aim)
                    .unwrap()
                    .camera
                    .rotate_vector([0., 0., 1.]),
            );
            assert!(angle_difference(achieved.azimuth, azimuth).abs() > GAZE_DEADBAND_RAD);
            assert!(
                gaze.status.clamped,
                "bounded aim misses {azimuth}: {achieved:?}"
            );
            for tick in 1..=1200 {
                advance(
                    &mut gaze,
                    &mut core,
                    &anchor,
                    &boot,
                    tick,
                    (tick % 5 == 0).then_some(azimuth),
                );
            }
            assert!(core.driver().writes.len() > 1100);
            for pose in &core.driver().writes {
                core.driver().validate_positions(pose).unwrap();
            }
            assert!(
                gaze.status.clamped,
                "base-limited target {azimuth} must remain reported as clamped"
            );
        }
    }

    #[test]
    fn neck_leads_base_then_centres_and_every_compiled_sample_is_calibrated() {
        let (mut core, anchor, boot) = following();
        let mut gaze = GazeCoordinator::default();
        gaze.set_enabled(true);
        for tick in 0..=400 {
            advance(
                &mut gaze,
                &mut core,
                &anchor,
                &boot,
                tick,
                (tick % 5 == 0).then_some(0.55),
            );
            if tick == 10 {
                let q = &core.driver().positions;
                assert!(
                    (q["head_roll_joint"] - anchor["head_roll_joint"]).abs()
                        > (q["base_yaw_joint"] - anchor["base_yaw_joint"]).abs()
                );
            }
        }
        let q = &core.driver().positions;
        assert!(
            (q["head_roll_joint"]
                - core
                    .camera_kinematics()
                    .unwrap()
                    .joint_reference("head_roll_joint")
                    .unwrap())
            .abs()
                <= GAZE_DEADBAND_RAD + 0.005,
            "{q:?}"
        );
        let bearing = Bearing::from_direction(
            core.camera_kinematics()
                .unwrap()
                .fk(q)
                .unwrap()
                .camera
                .rotate_vector([0.0, 0.0, 1.0]),
        );
        assert!(
            (bearing.azimuth - 0.55).abs() <= GAZE_DEADBAND_RAD + 0.005,
            "{bearing:?}"
        );
        assert!(gaze.status.extensions <= 81);
        for pair in core.driver().writes.windows(2) {
            assert!(
                (pair[1]["base_yaw_joint"] - pair[0]["base_yaw_joint"]).abs() / 0.02
                    <= GAZE_COMPILED_PEAK_RAD_S + 0.001
            );
            assert!(
                (pair[1]["head_roll_joint"] - pair[0]["head_roll_joint"]).abs() / 0.02
                    <= GAZE_COMPILED_PEAK_RAD_S + 0.001
            );
        }
    }

    #[test]
    fn stale_or_cleared_targets_hold_briefly_and_settle_to_the_immutable_anchor() {
        for explicit in [false, true] {
            let (mut core, anchor, boot) = following();
            let mut gaze = GazeCoordinator::default();
            gaze.set_enabled(true);
            for tick in 0..=150 {
                advance(
                    &mut gaze,
                    &mut core,
                    &anchor,
                    &boot,
                    tick,
                    (tick % 5 == 0).then_some(-0.4),
                );
            }
            assert_ne!(core.driver().positions, anchor);
            if explicit {
                gaze.clear();
            }
            for tick in 151..=450 {
                advance(&mut gaze, &mut core, &anchor, &boot, tick, None);
            }
            assert!(!gaze.active());
            for name in ORION_JOINT_NAMES {
                assert!((core.driver().positions[name] - anchor[name]).abs() < 1e-10);
            }
            assert_eq!(gaze.status.state, "idle");
        }
    }

    #[test]
    fn stop_and_foreground_ownership_preempt_gaze_without_restart_or_error() {
        for command in ["stop", "goto home 1"] {
            let (mut core, anchor, boot) = following();
            let mut gaze = GazeCoordinator::default();
            gaze.set_enabled(true);
            advance(&mut gaze, &mut core, &anchor, &boot, 0, Some(0.5));
            advance(&mut gaze, &mut core, &anchor, &boot, 5, Some(0.5));
            if command != "stop" {
                gaze.yield_to_priority(0.1, &mut core);
            }
            let result: serde_json::Value =
                serde_json::from_str(&core.handle_command(command, 0.1)).unwrap();
            assert_eq!(result["ok"], true);
            let before = core.snapshot().motion.clone();
            assert!(!gaze.tick(0.12, &anchor, &mut core));
            assert_eq!(
                core.snapshot().motion.as_ref().map(|m| m.run_id),
                before.as_ref().map(|m| m.run_id)
            );
            assert!(gaze.status.error.is_none());
        }
    }

    #[test]
    fn unreachable_bearings_choose_a_nearer_limit_and_inputs_require_enabled_gaze_and_freshness() {
        let (core, anchor, boot) = following();
        let mut gaze = GazeCoordinator::default();
        assert!(
            gaze.queue(target(boot.load(Ordering::Relaxed), 0.5), 0.0, &core)
                .is_err()
        );
        gaze.set_enabled(true);
        boot.store(1_000_000_001, Ordering::Relaxed);
        assert!(gaze.queue(target(0, 0.5), 0.0, &core).is_err());
        let model = core.camera_kinematics().unwrap();
        for (azimuth, elevation) in [(0.6, 0.2), (-0.6, -0.2), (std::f64::consts::PI, 1.4)] {
            let (pose, clamped) =
                solve_aim(model, &anchor, azimuth, elevation, &physical_limits()).unwrap();
            core.driver().validate_positions(&pose).unwrap();
            if elevation == 1.4 {
                assert!(clamped);
            } else {
                let aimed = Bearing::from_direction(
                    model
                        .fk(&pose)
                        .unwrap()
                        .camera
                        .rotate_vector([0.0, 0.0, 1.0]),
                );
                assert!(angle_difference(aimed.azimuth, azimuth).abs() < 0.001);
                assert!((aimed.elevation - elevation).abs() < 0.001);
            }
        }
    }

    #[test]
    fn native_mujoco_tracks_a_moving_bearing_through_the_character_compiler() {
        let (poses, motions) = libraries();
        let anchor = poses.pose("home").unwrap().clone();
        let driver = MujocoDriver::launch_for_hardware(
            root().join(".venv/bin/python"),
            root().join("runtime/mujoco_bridge.py"),
            root().join("simulation/mujoco/v2/scene.xml"),
            &anchor,
            crate::HardwareVersion::V2,
        )
        .unwrap();
        let (mut core, boot) = prepare(driver, poses, motions);
        let mut gaze = GazeCoordinator::default();
        gaze.set_enabled(true);
        for tick in 0..=500 {
            let desired = 0.3 * (tick as f64 * 0.02 * 0.5).sin();
            advance(
                &mut gaze,
                &mut core,
                &anchor,
                &boot,
                tick,
                (tick % 5 == 0).then_some(desired),
            );
            if tick > 100 {
                let positions = core
                    .snapshot()
                    .joints
                    .iter()
                    .map(|q| (q.name.clone(), q.position))
                    .collect();
                let measured = Bearing::from_direction(
                    core.camera_kinematics()
                        .unwrap()
                        .fk(&positions)
                        .unwrap()
                        .camera
                        .rotate_vector([0.0, 0.0, 1.0]),
                );
                assert!(
                    (measured.azimuth - desired).abs() < 6.0_f64.to_radians(),
                    "{measured:?} target {desired}"
                );
            }
        }
    }
}
