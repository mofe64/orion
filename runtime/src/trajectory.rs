use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::driver::JointLimit;
use crate::pose::JointPositions;
use crate::style::MotionStyle;
use crate::{Error, Result};

/// Published no-load capability of the 7.4 V Feetech STS3215 (52 RPM).
pub const STS3215_MAX_SPEED_RAD_S: f64 = 5.445_427_266_222_309;
const RETIME_ITERATIONS: usize = 12;
const OVERSHOOT_SAMPLES: usize = 80;
const CALIBRATION_BLEND_ITERATIONS: usize = 18;
const COMMAND_RATE_HZ: f64 = 50.0;
const INTERRUPT_SPEED_HEADROOM: f64 = 0.95;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaypointArrival {
    Through,
    Settle,
}

#[derive(Clone, Debug)]
pub struct TrajectoryWaypoint {
    pub label: String,
    pub positions: JointPositions,
    pub duration_seconds: f64,
    pub arrival: WaypointArrival,
    pub hold_seconds: f64,
    pub marker: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TrajectorySample {
    pub positions: JointPositions,
    pub velocities: JointPositions,
    pub accelerations: JointPositions,
}

#[derive(Clone, Copy, Debug)]
struct Polynomial {
    coefficients: [f64; 6],
}

impl Polynomial {
    fn quintic(p0: f64, v0: f64, a0: f64, p1: f64, v1: f64, a1: f64, t: f64) -> Self {
        let c0 = p0;
        let c1 = v0;
        let c2 = a0 / 2.0;
        let displacement = p1 - (c0 + c1 * t + c2 * t * t);
        let velocity = v1 - (c1 + 2.0 * c2 * t);
        let acceleration = a1 - 2.0 * c2;
        let c3 =
            (10.0 * displacement - 4.0 * velocity * t + 0.5 * acceleration * t * t) / t.powi(3);
        let c4 = (-15.0 * displacement + 7.0 * velocity * t - acceleration * t * t) / t.powi(4);
        let c5 = (6.0 * displacement - 3.0 * velocity * t + 0.5 * acceleration * t * t) / t.powi(5);
        Self {
            coefficients: [c0, c1, c2, c3, c4, c5],
        }
    }

    fn sample(self, t: f64) -> (f64, f64, f64) {
        let [c0, c1, c2, c3, c4, c5] = self.coefficients;
        let position = c0 + t * (c1 + t * (c2 + t * (c3 + t * (c4 + t * c5))));
        let velocity = c1 + t * (2.0 * c2 + t * (3.0 * c3 + t * (4.0 * c4 + t * 5.0 * c5)));
        let acceleration = 2.0 * c2 + t * (6.0 * c3 + t * (12.0 * c4 + t * 20.0 * c5));
        (position, velocity, acceleration)
    }
}

#[derive(Clone, Debug)]
struct CompiledSegment {
    label: String,
    starts_at: f64,
    arrives_at: f64,
    holds_until: f64,
    polynomials: BTreeMap<String, Polynomial>,
    target: JointPositions,
    marker: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CompiledTrajectory {
    name: String,
    segments: Vec<CompiledSegment>,
    duration_seconds: f64,
    peak_velocity_rad_s: f64,
}

impl CompiledTrajectory {
    pub fn compile(
        name: impl Into<String>,
        start: JointPositions,
        start_velocity: JointPositions,
        waypoints: Vec<TrajectoryWaypoint>,
        style: MotionStyle,
        maximum_velocity_rad_s: f64,
    ) -> Result<Self> {
        let name = name.into();
        validate_inputs(
            &name,
            &start,
            &start_velocity,
            &waypoints,
            maximum_velocity_rad_s,
        )?;
        let mut durations: Vec<f64> = waypoints
            .iter()
            .map(|waypoint| {
                let settle_weight = if waypoint.arrival == WaypointArrival::Settle {
                    0.85 + 0.30 * style.settle_character
                } else {
                    1.0
                };
                waypoint.duration_seconds / style.tempo * settle_weight
            })
            .collect();
        for _ in 0..RETIME_ITERATIONS {
            let candidate = compile_with_durations(
                &name,
                &start,
                &start_velocity,
                &waypoints,
                &durations,
                style,
            )?;
            let mut changed = false;
            for (index, peak) in candidate.segment_peak_velocities().into_iter().enumerate() {
                if peak > maximum_velocity_rad_s * (1.0 + 1e-9) {
                    durations[index] *= (peak / maximum_velocity_rad_s) * 1.015;
                    changed = true;
                }
            }
            if !changed {
                return Ok(candidate);
            }
        }
        let candidate = compile_with_durations(
            &name,
            &start,
            &start_velocity,
            &waypoints,
            &durations,
            style,
        )?;
        if candidate.peak_velocity_rad_s > maximum_velocity_rad_s * 1.001 {
            return Err(Error::Runtime(format!(
                "Trajectory '{name}' could not be retimed below {maximum_velocity_rad_s:.3} rad/s."
            )));
        }
        Ok(candidate)
    }

    /// Compile an interruption blend whose 50 Hz command samples remain inside
    /// calibration. Measured start velocity is preserved joint-by-joint when
    /// it is physically representable and safe. A present-speed telemetry
    /// spike above the motor profile ceiling is first bounded with small
    /// braking headroom; only joints whose blend would then leave their
    /// calibrated range are progressively attenuated. Authored targets and
    /// all other joints are unchanged.
    pub fn compile_calibrated(
        name: impl Into<String>,
        start: JointPositions,
        start_velocity: JointPositions,
        waypoints: Vec<TrajectoryWaypoint>,
        style: MotionStyle,
        maximum_velocity_rad_s: f64,
        limits: &[JointLimit],
    ) -> Result<Self> {
        let name = name.into();
        validate_limits(&start, limits)?;
        let mut blended_velocity = start_velocity;
        let bounded_start_speed = maximum_velocity_rad_s * INTERRUPT_SPEED_HEADROOM;
        for velocity in blended_velocity.values_mut() {
            if velocity.abs() > maximum_velocity_rad_s {
                *velocity = velocity.signum() * bounded_start_speed;
            }
        }
        for _ in 0..CALIBRATION_BLEND_ITERATIONS {
            let candidate = Self::compile(
                name.clone(),
                start.clone(),
                blended_velocity.clone(),
                waypoints.clone(),
                style,
                maximum_velocity_rad_s,
            )?;
            let violations = candidate.calibration_violations(limits)?;
            if violations.is_empty() {
                return Ok(candidate);
            }
            for joint in violations {
                let velocity = blended_velocity.get_mut(&joint).ok_or_else(|| {
                    Error::InvalidArgument(format!(
                        "Calibration limit references unknown trajectory joint '{joint}'."
                    ))
                })?;
                *velocity *= 0.5;
                if velocity.abs() < 1e-6 {
                    *velocity = 0.0;
                }
            }
        }
        Err(Error::Runtime(format!(
            "Trajectory '{name}' could not preserve a calibration-safe interruption blend."
        )))
    }

    pub fn sample_state(&self, elapsed_seconds: f64) -> Result<TrajectorySample> {
        let index = self.segment_index(elapsed_seconds)?;
        let segment = &self.segments[index];
        if elapsed_seconds >= segment.arrives_at {
            let zeros: JointPositions = segment
                .target
                .keys()
                .map(|name| (name.clone(), 0.0))
                .collect();
            return Ok(TrajectorySample {
                positions: segment.target.clone(),
                velocities: zeros.clone(),
                accelerations: zeros,
            });
        }
        let local = (elapsed_seconds.max(0.0) - segment.starts_at)
            .clamp(0.0, segment.arrives_at - segment.starts_at);
        let mut positions = JointPositions::new();
        let mut velocities = JointPositions::new();
        let mut accelerations = JointPositions::new();
        for (joint, polynomial) in &segment.polynomials {
            let (position, velocity, acceleration) = polynomial.sample(local);
            positions.insert(joint.clone(), position);
            velocities.insert(joint.clone(), velocity);
            accelerations.insert(joint.clone(), acceleration);
        }
        Ok(TrajectorySample {
            positions,
            velocities,
            accelerations,
        })
    }

    pub fn sample(&self, elapsed_seconds: f64) -> Result<JointPositions> {
        Ok(self.sample_state(elapsed_seconds)?.positions)
    }
    pub fn progress(&self, elapsed_seconds: f64) -> Result<f64> {
        if !elapsed_seconds.is_finite() {
            return Err(Error::InvalidArgument(
                "Trajectory elapsed time must be finite.".into(),
            ));
        }
        Ok((elapsed_seconds / self.duration_seconds).clamp(0.0, 1.0))
    }
    pub fn complete(&self, elapsed_seconds: f64) -> Result<bool> {
        Ok(self.progress(elapsed_seconds)? >= 1.0)
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }
    pub fn peak_velocity_rad_s(&self) -> f64 {
        self.peak_velocity_rad_s
    }
    pub fn keyframe_name(&self, elapsed_seconds: f64) -> Result<&str> {
        Ok(&self.segments[self.segment_index(elapsed_seconds)?].label)
    }
    pub fn keyframe_index(&self, elapsed_seconds: f64) -> Result<usize> {
        self.segment_index(elapsed_seconds)
    }
    pub fn keyframe_count(&self) -> usize {
        self.segments.len()
    }
    pub fn keyframe_arrival_time(&self, index: usize) -> Option<f64> {
        self.segments.get(index).map(|segment| segment.arrives_at)
    }
    pub fn marker_time(&self, marker: &str) -> Option<f64> {
        self.segments
            .iter()
            .find(|segment| segment.marker.as_deref() == Some(marker))
            .map(|segment| segment.arrives_at)
    }
    pub fn reached_markers(&self, elapsed_seconds: f64) -> Vec<String> {
        self.segments
            .iter()
            .filter(|segment| elapsed_seconds >= segment.arrives_at)
            .filter_map(|segment| segment.marker.clone())
            .collect()
    }

    fn calibration_violations(&self, limits: &[JointLimit]) -> Result<BTreeSet<String>> {
        let steps = (self.duration_seconds * COMMAND_RATE_HZ).ceil() as usize;
        let mut violations = BTreeSet::new();
        for step in 0..=steps {
            let elapsed = (step as f64 / COMMAND_RATE_HZ).min(self.duration_seconds);
            let positions = self.sample(elapsed)?;
            for limit in limits {
                let value = positions.get(&limit.name).ok_or_else(|| {
                    Error::InvalidArgument(format!(
                        "Calibration limit references unknown trajectory joint '{}'.",
                        limit.name
                    ))
                })?;
                if *value < limit.lower_rad || *value > limit.upper_rad {
                    violations.insert(limit.name.clone());
                }
            }
        }
        Ok(violations)
    }

    fn segment_peak_velocities(&self) -> Vec<f64> {
        self.segments
            .iter()
            .map(|segment| {
                let duration = segment.arrives_at - segment.starts_at;
                (0..=OVERSHOOT_SAMPLES)
                    .flat_map(|sample| {
                        let t = duration * sample as f64 / OVERSHOOT_SAMPLES as f64;
                        segment
                            .polynomials
                            .values()
                            .map(move |polynomial| polynomial.sample(t).1.abs())
                    })
                    .fold(0.0, f64::max)
            })
            .collect()
    }

    fn segment_index(&self, elapsed_seconds: f64) -> Result<usize> {
        if !elapsed_seconds.is_finite() {
            return Err(Error::InvalidArgument(
                "Trajectory elapsed time must be finite.".into(),
            ));
        }
        let elapsed = elapsed_seconds.max(0.0);
        self.segments
            .iter()
            .enumerate()
            .find_map(|(index, segment)| {
                (elapsed < segment.holds_until || index + 1 == self.segments.len()).then_some(index)
            })
            .ok_or_else(|| Error::InvalidState("Trajectory contains no segment.".into()))
    }
}

fn compile_with_durations(
    name: &str,
    start: &JointPositions,
    start_velocity: &JointPositions,
    waypoints: &[TrajectoryWaypoint],
    durations: &[f64],
    style: MotionStyle,
) -> Result<CompiledTrajectory> {
    let mut points = vec![start.clone()];
    points.extend(waypoints.iter().map(|waypoint| waypoint.positions.clone()));
    let mut velocities = derivative_maps(start, start_velocity, waypoints, durations, style, true);
    let mut accelerations =
        derivative_maps(start, start_velocity, waypoints, durations, style, false);
    clamp_unrequested_overshoot(&points, durations, &mut velocities, &mut accelerations);
    let mut starts_at = 0.0;
    let mut segments = Vec::with_capacity(waypoints.len());
    for (index, waypoint) in waypoints.iter().enumerate() {
        let duration = durations[index];
        let arrives_at = starts_at + duration;
        let holds_until = arrives_at + waypoint.hold_seconds;
        let polynomials = start
            .keys()
            .map(|joint| {
                (
                    joint.clone(),
                    Polynomial::quintic(
                        points[index][joint],
                        velocities[index][joint],
                        accelerations[index][joint],
                        points[index + 1][joint],
                        velocities[index + 1][joint],
                        accelerations[index + 1][joint],
                        duration,
                    ),
                )
            })
            .collect();
        segments.push(CompiledSegment {
            label: waypoint.label.clone(),
            starts_at,
            arrives_at,
            holds_until,
            polynomials,
            target: waypoint.positions.clone(),
            marker: waypoint.marker.clone(),
        });
        starts_at = holds_until;
    }
    let mut trajectory = CompiledTrajectory {
        name: name.to_owned(),
        segments,
        duration_seconds: starts_at,
        peak_velocity_rad_s: 0.0,
    };
    trajectory.peak_velocity_rad_s = trajectory
        .segment_peak_velocities()
        .into_iter()
        .fold(0.0, f64::max);
    Ok(trajectory)
}

fn derivative_maps(
    start: &JointPositions,
    start_velocity: &JointPositions,
    waypoints: &[TrajectoryWaypoint],
    durations: &[f64],
    style: MotionStyle,
    velocity: bool,
) -> Vec<JointPositions> {
    let mut points = vec![start];
    points.extend(waypoints.iter().map(|waypoint| &waypoint.positions));
    (0..points.len())
        .map(|index| {
            start
                .keys()
                .map(|joint| {
                    let value = if index == 0 {
                        if velocity { start_velocity[joint] } else { 0.0 }
                    } else if index + 1 == points.len()
                        || waypoints[index - 1].arrival == WaypointArrival::Settle
                    {
                        0.0
                    } else {
                        let before = (points[index][joint] - points[index - 1][joint])
                            / durations[index - 1];
                        let after =
                            (points[index + 1][joint] - points[index][joint]) / durations[index];
                        let lag_character = joint_lag_character(joint, style.joint_lag);
                        if velocity {
                            if before * after <= 0.0 {
                                0.0
                            } else {
                                let weighted = (before * durations[index]
                                    + after * durations[index - 1])
                                    / (durations[index - 1] + durations[index]);
                                weighted.signum()
                                    * weighted.abs().min(3.0 * before.abs().min(after.abs()))
                                    * style.tangent_tension
                                    * lag_character
                            }
                        } else {
                            2.0 * (after - before) / (durations[index - 1] + durations[index])
                                * style.tangent_tension
                                * lag_character
                                * (0.5 + 0.5 * style.overshoot_scale)
                        }
                    };
                    (joint.clone(), value)
                })
                .collect()
        })
        .collect()
}

fn joint_lag_character(joint: &str, lag: f64) -> f64 {
    let order = match joint {
        "base_yaw_joint" => 0.0,
        "shoulder_pitch_joint" => 1.0,
        "elbow_pitch_joint" => 2.0,
        "head_roll_joint" => 3.0,
        "head_pitch_joint" => 4.0,
        _ => 2.0,
    };
    1.0 - lag.clamp(0.0, 1.0) * order / 4.0
}

fn clamp_unrequested_overshoot(
    points: &[JointPositions],
    durations: &[f64],
    velocities: &mut [JointPositions],
    accelerations: &mut [JointPositions],
) {
    for _ in 0..8 {
        let mut offending = BTreeSet::<(usize, String)>::new();
        for index in 0..durations.len() {
            let duration = durations[index];
            for joint in points[index].keys() {
                let polynomial = Polynomial::quintic(
                    points[index][joint],
                    velocities[index][joint],
                    accelerations[index][joint],
                    points[index + 1][joint],
                    velocities[index + 1][joint],
                    accelerations[index + 1][joint],
                    duration,
                );
                let lower = points[index][joint].min(points[index + 1][joint]) - 1e-9;
                let upper = points[index][joint].max(points[index + 1][joint]) + 1e-9;
                if (1..OVERSHOOT_SAMPLES).any(|sample| {
                    let value = polynomial
                        .sample(duration * sample as f64 / OVERSHOOT_SAMPLES as f64)
                        .0;
                    value < lower || value > upper
                }) {
                    offending.insert((index, joint.clone()));
                }
            }
        }
        if offending.is_empty() {
            break;
        }
        // Clamp only the two derivatives bordering an offending segment. A
        // global per-joint clamp makes one difficult turn flatten that joint
        // across an entire long performance, creating visible stop-start
        // motion far away from the actual overshoot risk.
        for (segment, joint) in offending {
            for point in [segment, segment + 1] {
                if point > 0 && point + 1 < velocities.len() {
                    *velocities[point]
                        .get_mut(&joint)
                        .expect("joint derivative exists") *= 0.5;
                    *accelerations[point]
                        .get_mut(&joint)
                        .expect("joint derivative exists") *= 0.5;
                }
            }
        }
    }
}

fn validate_inputs(
    name: &str,
    start: &JointPositions,
    start_velocity: &JointPositions,
    waypoints: &[TrajectoryWaypoint],
    maximum_velocity_rad_s: f64,
) -> Result<()> {
    if name.is_empty() || start.is_empty() || waypoints.is_empty() {
        return Err(Error::InvalidArgument(
            "Trajectory requires a name, start, and waypoints.".into(),
        ));
    }
    if start.keys().ne(start_velocity.keys())
        || !maximum_velocity_rad_s.is_finite()
        || maximum_velocity_rad_s <= 0.0
    {
        return Err(Error::InvalidArgument(
            "Trajectory start velocity and finite positive motor speed are required.".into(),
        ));
    }
    for waypoint in waypoints {
        if start.keys().ne(waypoint.positions.keys())
            || !waypoint.duration_seconds.is_finite()
            || waypoint.duration_seconds <= 0.0
            || !waypoint.hold_seconds.is_finite()
            || waypoint.hold_seconds < 0.0
            || waypoint.hold_seconds > 0.0 && waypoint.arrival != WaypointArrival::Settle
            || waypoint.positions.values().any(|value| !value.is_finite())
        {
            return Err(Error::InvalidArgument(
                "Trajectory waypoints must share finite joints; holds require settle arrivals."
                    .into(),
            ));
        }
    }
    Ok(())
}

fn validate_limits(start: &JointPositions, limits: &[JointLimit]) -> Result<()> {
    if limits.len() != start.len() {
        return Err(Error::InvalidArgument(
            "Calibrated trajectory requires exactly one limit for every joint.".into(),
        ));
    }
    let mut names = BTreeSet::new();
    for limit in limits {
        if !names.insert(limit.name.clone())
            || !start.contains_key(&limit.name)
            || !limit.lower_rad.is_finite()
            || !limit.upper_rad.is_finite()
            || limit.lower_rad >= limit.upper_rad
        {
            return Err(Error::InvalidArgument(
                "Calibrated trajectory limits must be unique, finite, ordered, and match every joint."
                    .into(),
            ));
        }
        let value = start[&limit.name];
        if value < limit.lower_rad || value > limit.upper_rad {
            return Err(Error::InvalidArgument(format!(
                "Trajectory start for '{}' is outside its calibrated range.",
                limit.name
            )));
        }
    }
    Ok(())
}

/// One-target `goto` commands use the same compiler as authored motion.
#[derive(Clone, Debug)]
pub struct JointTrajectory(CompiledTrajectory);

impl JointTrajectory {
    pub fn new(
        name: impl Into<String>,
        start: JointPositions,
        target: JointPositions,
        duration_seconds: f64,
    ) -> Result<Self> {
        let zero_velocity = start.keys().map(|name| (name.clone(), 0.0)).collect();
        Self::with_start_velocity(name, start, zero_velocity, target, duration_seconds)
    }

    pub fn with_start_velocity(
        name: impl Into<String>,
        start: JointPositions,
        start_velocity: JointPositions,
        target: JointPositions,
        duration_seconds: f64,
    ) -> Result<Self> {
        Self::compile(name, start, start_velocity, target, duration_seconds, None)
    }

    pub fn with_start_velocity_calibrated(
        name: impl Into<String>,
        start: JointPositions,
        start_velocity: JointPositions,
        target: JointPositions,
        duration_seconds: f64,
        limits: &[JointLimit],
    ) -> Result<Self> {
        Self::compile(
            name,
            start,
            start_velocity,
            target,
            duration_seconds,
            Some(limits),
        )
    }

    fn compile(
        name: impl Into<String>,
        start: JointPositions,
        start_velocity: JointPositions,
        target: JointPositions,
        duration_seconds: f64,
        limits: Option<&[JointLimit]>,
    ) -> Result<Self> {
        let style = MotionStyle {
            name: "goto",
            tempo: 1.0,
            tangent_tension: 0.0,
            joint_lag: 0.0,
            amplitude: 1.0,
            overshoot_scale: 0.0,
            // A neutral settle weight preserves the duration explicitly requested
            // by a direct goto command; authored motions can still stylize settles.
            settle_character: 0.5,
        };
        let waypoints = vec![TrajectoryWaypoint {
            label: "target".into(),
            positions: target,
            duration_seconds,
            arrival: WaypointArrival::Settle,
            hold_seconds: 0.0,
            marker: None,
        }];
        let trajectory = if let Some(limits) = limits {
            CompiledTrajectory::compile_calibrated(
                name,
                start,
                start_velocity,
                waypoints,
                style,
                STS3215_MAX_SPEED_RAD_S,
                limits,
            )?
        } else {
            CompiledTrajectory::compile(
                name,
                start,
                start_velocity,
                waypoints,
                style,
                STS3215_MAX_SPEED_RAD_S,
            )?
        };
        Ok(Self(trajectory))
    }
    pub fn sample(&self, elapsed_seconds: f64) -> Result<JointPositions> {
        self.0.sample(elapsed_seconds)
    }
    pub fn progress(&self, elapsed_seconds: f64) -> Result<f64> {
        self.0.progress(elapsed_seconds)
    }
    pub fn complete(&self, elapsed_seconds: f64) -> Result<bool> {
        self.0.complete(elapsed_seconds)
    }
    pub fn name(&self) -> &str {
        self.0.name()
    }
    pub fn duration_seconds(&self) -> f64 {
        self.0.duration_seconds()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn positions(values: &[(&str, f64)]) -> JointPositions {
        values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value))
            .collect()
    }
    fn style() -> MotionStyle {
        MotionStyle::named("expressive_turn").unwrap()
    }

    #[test]
    fn keeps_position_velocity_and_acceleration_continuous_through_keyframes() {
        let trajectory = CompiledTrajectory::compile(
            "fluid",
            positions(&[("joint", 0.0)]),
            positions(&[("joint", 0.0)]),
            vec![
                TrajectoryWaypoint {
                    label: "through".into(),
                    positions: positions(&[("joint", 1.0)]),
                    duration_seconds: 0.5,
                    arrival: WaypointArrival::Through,
                    hold_seconds: 0.0,
                    marker: Some("notice".into()),
                },
                TrajectoryWaypoint {
                    label: "settle".into(),
                    positions: positions(&[("joint", 2.0)]),
                    duration_seconds: 0.5,
                    arrival: WaypointArrival::Settle,
                    hold_seconds: 0.0,
                    marker: None,
                },
            ],
            style(),
            STS3215_MAX_SPEED_RAD_S,
        )
        .unwrap();
        let marker = trajectory.marker_time("notice").unwrap();
        let left = trajectory.sample_state(marker - 1e-6).unwrap();
        let right = trajectory.sample_state(marker + 1e-6).unwrap();
        assert!((left.positions["joint"] - right.positions["joint"]).abs() < 1e-4);
        assert!((left.velocities["joint"] - right.velocities["joint"]).abs() < 1e-3);
        assert!((left.accelerations["joint"] - right.accelerations["joint"]).abs() < 1e-2);
        assert!(left.velocities["joint"].abs() > 0.01);
        let end = trajectory
            .sample_state(trajectory.duration_seconds())
            .unwrap();
        assert_eq!(end.velocities["joint"], 0.0);
        assert_eq!(end.accelerations["joint"], 0.0);
    }

    #[test]
    fn retimes_fast_segments_to_the_sts3215_ceiling_without_extra_overshoot() {
        let trajectory = CompiledTrajectory::compile(
            "fast",
            positions(&[("joint", 0.0)]),
            positions(&[("joint", 0.0)]),
            vec![TrajectoryWaypoint {
                label: "end".into(),
                positions: positions(&[("joint", 3.0)]),
                duration_seconds: 0.01,
                arrival: WaypointArrival::Settle,
                hold_seconds: 0.0,
                marker: None,
            }],
            style(),
            STS3215_MAX_SPEED_RAD_S,
        )
        .unwrap();
        assert!(trajectory.peak_velocity_rad_s() <= STS3215_MAX_SPEED_RAD_S * 1.001);
        for sample in 0..=100 {
            let value = trajectory
                .sample(trajectory.duration_seconds() * sample as f64 / 100.0)
                .unwrap()["joint"];
            assert!((0.0..=3.0).contains(&value));
        }
    }

    #[test]
    fn goto_uses_quintic_endpoints_and_midpoint() {
        let trajectory = JointTrajectory::new(
            "test",
            positions(&[("a", 0.0), ("b", 1.0)]),
            positions(&[("a", 1.0), ("b", -1.0)]),
            2.0,
        )
        .unwrap();
        assert_eq!(trajectory.sample(0.0).unwrap()["a"], 0.0);
        assert!((trajectory.sample(1.0).unwrap()["a"] - 0.5).abs() < 1e-12);
        assert_eq!(trajectory.sample(2.0).unwrap()["a"], 1.0);
    }

    #[test]
    fn interruption_preserves_measured_start_position_and_velocity() {
        let start = positions(&[("joint", 0.4)]);
        let velocity = positions(&[("joint", -0.3)]);
        let trajectory = CompiledTrajectory::compile(
            "interrupted",
            start.clone(),
            velocity.clone(),
            vec![TrajectoryWaypoint {
                label: "new_target".into(),
                positions: positions(&[("joint", 1.0)]),
                duration_seconds: 0.8,
                arrival: WaypointArrival::Settle,
                hold_seconds: 0.0,
                marker: None,
            }],
            style(),
            STS3215_MAX_SPEED_RAD_S,
        )
        .unwrap();
        let first = trajectory.sample_state(0.0).unwrap();
        assert_eq!(first.positions, start);
        assert_eq!(first.velocities, velocity);
        let next = trajectory.sample_state(1e-6).unwrap();
        assert!((next.positions["joint"] - (0.4 - 0.3e-6)).abs() < 1e-9);
    }

    #[test]
    fn calibrated_interruption_attenuates_only_velocity_that_would_leave_range() {
        let start = positions(&[("safe", 0.0), ("edge", 0.99)]);
        let measured_velocity = positions(&[("safe", 0.3), ("edge", 1.0)]);
        let trajectory = CompiledTrajectory::compile_calibrated(
            "calibrated-interruption",
            start.clone(),
            measured_velocity.clone(),
            vec![TrajectoryWaypoint {
                label: "new_target".into(),
                positions: positions(&[("safe", 0.6), ("edge", 0.0)]),
                duration_seconds: 0.8,
                arrival: WaypointArrival::Settle,
                hold_seconds: 0.0,
                marker: None,
            }],
            style(),
            STS3215_MAX_SPEED_RAD_S,
            &[
                JointLimit {
                    name: "safe".into(),
                    lower_rad: -1.0,
                    upper_rad: 1.0,
                },
                JointLimit {
                    name: "edge".into(),
                    lower_rad: -1.0,
                    upper_rad: 1.0,
                },
            ],
        )
        .unwrap();

        let first = trajectory.sample_state(0.0).unwrap();
        assert_eq!(first.positions, start);
        assert_eq!(first.velocities["safe"], measured_velocity["safe"]);
        assert!(first.velocities["edge"] < measured_velocity["edge"]);
        let steps = (trajectory.duration_seconds() * COMMAND_RATE_HZ).ceil() as usize;
        for step in 0..=steps {
            let positions = trajectory
                .sample((step as f64 / COMMAND_RATE_HZ).min(trajectory.duration_seconds()))
                .unwrap();
            assert!((-1.0..=1.0).contains(&positions["safe"]));
            assert!((-1.0..=1.0).contains(&positions["edge"]));
        }
    }

    #[test]
    fn calibrated_interruption_bounds_telemetry_above_motor_ceiling() {
        let start = positions(&[("joint", -0.22)]);
        // STS3215 present-speed telemetry is quantized in 0.732 RPM units.
        // A transient raw value of 100 reports 7.665 rad/s, above the motor
        // profile ceiling and therefore impossible to preserve in a bounded
        // compiled command stream.
        let measured_velocity = positions(&[("joint", 7.665_486_074_759_095)]);
        let trajectory = CompiledTrajectory::compile_calibrated(
            "reversing-interruption",
            start.clone(),
            measured_velocity.clone(),
            vec![TrajectoryWaypoint {
                label: "new_target".into(),
                positions: positions(&[("joint", -0.36)]),
                duration_seconds: 0.95,
                arrival: WaypointArrival::Settle,
                hold_seconds: 0.0,
                marker: None,
            }],
            MotionStyle::named("thinking").unwrap(),
            STS3215_MAX_SPEED_RAD_S,
            &[JointLimit {
                name: "joint".into(),
                lower_rad: -1.0,
                upper_rad: 1.0,
            }],
        )
        .unwrap();

        let first = trajectory.sample_state(0.0).unwrap();
        assert_eq!(first.positions, start);
        assert!(first.velocities["joint"].abs() < measured_velocity["joint"].abs());
        assert!(trajectory.peak_velocity_rad_s() <= STS3215_MAX_SPEED_RAD_S * 1.001);
        let steps = (trajectory.duration_seconds() * COMMAND_RATE_HZ).ceil() as usize;
        for step in 0..=steps {
            let position = trajectory
                .sample((step as f64 / COMMAND_RATE_HZ).min(trajectory.duration_seconds()))
                .unwrap()["joint"];
            assert!((-1.0..=1.0).contains(&position));
        }
    }
}
