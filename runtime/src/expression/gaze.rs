//! Bounded bearing-to-keyframe planning; all execution stays in MotionSequence.
use crate::control::camera_rays::Bearing;
use crate::motion::kinematics::Kinematics;
use crate::{
    Error, JointLimit, JointPositions, KeyframeArrival, MotionDefinition, MotionKeyframe,
    MotionSequence, MotionSpace, MotionStyle, Result, RuntimeCore, RuntimeDriver,
};
use serde::{Deserialize, Serialize};

pub const GAZE_DEADBAND_RAD: f64 = 4.0 * std::f64::consts::PI / 180.0;
pub const GAZE_BASE_SPEED_RAD_S: f64 = 1.5;
pub const GAZE_NECK_SPEED_RAD_S: f64 = 2.5;
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
    pub state: &'static str,
    pub track_id: Option<u64>,
    pub run_id: Option<u64>,
    pub clamped: bool,
    pub extensions: u64,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct GazeCoordinator {
    status: GazeStatus,
    target: Option<GazeTarget>,
    last_capture: Option<u64>,
    started_at: f64,
    last_update: f64,
    explicit_clear: bool,
}

impl Default for GazeCoordinator {
    fn default() -> Self {
        Self {
            status: GazeStatus {
                enabled: false,
                state: "off",
                track_id: None,
                run_id: None,
                clamped: false,
                extensions: 0,
                error: None,
            },
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
        self.status.enabled = enabled;
        self.status.state = if enabled { "idle" } else { "off" };
        if !enabled {
            self.forget();
        }
    }
    pub fn status(&self) -> GazeStatus {
        self.status.clone()
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
        if !self.status.enabled {
            return Err(Error::InvalidState("Gaze requires --gaze on".into()));
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

    pub fn clear(&mut self) {
        self.target = None;
        self.status.track_id = None;
        self.explicit_clear = true;
    }

    pub(crate) fn forget(&mut self) {
        self.target = None;
        self.status.track_id = None;
        self.status.run_id = None;
        self.status.state = if self.status.enabled { "idle" } else { "off" };
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
        if velocity.values().any(|v| v.abs() > GAZE_BASE_SPEED_RAD_S) {
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
            let centred = start["head_roll_joint"].abs() <= GAZE_DEADBAND_RAD;
            if angle_difference(target.azimuth, current.azimuth).abs() > GAZE_DEADBAND_RAD
                || (target.elevation - current.elevation).abs() > GAZE_DEADBAND_RAD
                || !centred
            {
                if now - self.started_at >= NECK_LEAD_SECONDS && !centred {
                    let (lower, upper) = bounds(&limits, "base_yaw_joint")?;
                    let desired = target.azimuth.clamp(lower, upper);
                    let step = BASE_CATCHUP_SPEED_RAD_S * GAZE_UPDATE_SECONDS / 1.875;
                    let base = start["base_yaw_joint"]
                        + (desired - start["base_yaw_joint"]).clamp(-step, step);
                    *aim.get_mut("base_yaw_joint").unwrap() = base;
                    // Counter-rotate as an initial guess; FK solves the tilted
                    // swivel geometry instead of treating joint angles as bearings.
                    *aim.get_mut("head_roll_joint").unwrap() -= base - start["base_yaw_joint"];
                }
                let (solved, clamped) =
                    solve_aim(model, &aim, target.azimuth, target.elevation, &limits)?;
                self.status.clamped = clamped;
                for name in ["head_roll_joint", "head_pitch_joint"] {
                    let step = GAZE_NECK_SPEED_RAD_S * GAZE_UPDATE_SECONDS / 1.875;
                    *aim.get_mut(name).unwrap() =
                        start[name] + (solved[name] - start[name]).clamp(-step, step);
                }
            }
        }
        let return_seconds = start
            .iter()
            .map(|(name, value)| 1.875 * (value - anchor[name]).abs() / GAZE_BASE_SPEED_RAD_S)
            .fold(0.55_f64, f64::max);
        let offsets = aim
            .iter()
            .map(|(name, value)| (name.clone(), value - anchor[name]))
            .collect();
        let mut definition = MotionDefinition {
            name: "gaze_performance".into(),
            description: "Capture-grounded gaze; simulation_preview".into(),
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
        // Preview with the same compiler, state and calibration used at install.
        // Its shared peak ceiling is conservatively 1.5 rad/s, below both caps.
        for _ in 0..4 {
            let scale = definition.uniform_amplitude_scale(anchor, &limits)?;
            let sequence = MotionSequence::compile_scaled_calibrated(
                &definition,
                start.clone(),
                velocity.clone(),
                acceleration.clone(),
                anchor.clone(),
                scale,
                &limits,
            )?;
            let peak = sequence.peak_velocity_rad_s();
            if peak <= GAZE_BASE_SPEED_RAD_S * (1.0 + 1e-9) {
                return Ok(Some(definition));
            }
            for frame in &mut definition.keyframes {
                frame.duration_seconds *= peak / GAZE_BASE_SPEED_RAD_S * 1.02;
            }
        }
        Ok(None)
    }
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
fn solve_aim(
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
mod tests {
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
        let c: serde_json::Value = serde_json::from_str(include_str!(
            "../../../simulation/mujoco/config/servo_calibration-v2.json"
        ))
        .unwrap();
        ORION_JOINT_NAMES
            .iter()
            .map(|name| {
                let joint = &c["joints"][name];
                let per_count = std::f64::consts::TAU / c["encoder_resolution"].as_f64().unwrap();
                let a = joint["safe_min_delta_raw"].as_f64().unwrap() * per_count
                    / joint["encoder_direction"].as_f64().unwrap();
                let b = joint["safe_max_delta_raw"].as_f64().unwrap() * per_count
                    / joint["encoder_direction"].as_f64().unwrap();
                JointLimit {
                    name: (*name).into(),
                    lower_rad: a.min(b),
                    upper_rad: a.max(b),
                }
            })
            .collect()
    }
    struct FollowingDriver {
        positions: JointPositions,
        writes: Vec<JointPositions>,
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
            Ok(self
                .positions
                .iter()
                .map(|(name, q)| JointState {
                    name: name.clone(),
                    position: *q,
                    velocity: 0.0,
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
    fn following() -> (RuntimeCore<FollowingDriver>, JointPositions, Arc<AtomicU64>) {
        let (poses, motions) = libraries();
        let anchor = poses.pose("home").unwrap().clone();
        let driver = FollowingDriver {
            positions: anchor.clone(),
            writes: Vec::new(),
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
            q["head_roll_joint"].abs() <= GAZE_DEADBAND_RAD + 0.005,
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
                    <= GAZE_BASE_SPEED_RAD_S + 0.001
            );
            assert!(
                (pair[1]["head_roll_joint"] - pair[0]["head_roll_joint"]).abs() / 0.02
                    <= GAZE_NECK_SPEED_RAD_S + 0.001
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
    fn unreachable_bearings_choose_a_nearer_limit_and_inputs_require_opt_in_and_freshness() {
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
