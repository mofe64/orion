//! trajectory.rs is responsible for turning orion joint positions into a timed movement plan.
//! it calculates where every joint should be and how its planned position is changing at an point during that movement.
//! A pose says: "Put Orion's head at this angle."
//! A motion says: "Move through this pose, reach that pose, then pause."
//! A trajectory answers: "We are 0.42 seconds into that motion. What angle should each joint be commanded to now?"
//! Targets are resolved before reaching trajectory.rs, so we don't have to deal with poses heres
//! The units matter throughout:
//! Quantity	    Unit	            Physical meaning
//! Position	    radians	            Joint angle
//! Velocity	    radians/second	    How quickly that angle changes, including direction
//! Acceleration	radians/second²	    How quickly velocity changes
//! Time	        seconds	            Elapsed time or duration
//!
//!
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::devices::driver::JointLimit;
use crate::error::{OrionRuntimeError as Error, Result};
use crate::motion::pose::JointPositions;
use crate::motion::style::MotionStyle;

/// Published no-load capability of the 7.4 V Feetech STS3215 (52 RPM).
pub const STS3215_MAX_SPEED_RAD_S: f64 = 5.445_427_266_222_309;
/// Allows up to 12 iterations of stretching movement durations to fit within limits before giving up.
const RETIME_ITERATIONS: usize = 12;
/// Divided each movement segment into 80 samples for curve checks in other to estimate overshoot.
const OVERSHOOT_SAMPLES: usize = 80;
/// Allows up to 18 candidate blends when accommodating starting velocity within calibration
const CALIBRATION_BLEND_ITERATIONS: usize = 18;
/// Checks calibrated positions on a 50 Hz time grid: every 0.02 seconds
const COMMAND_RATE_HZ: f64 = 50.0;
/// Uses 95% of the ceiling when bounding an above-ceiling starting velocity
const INTERRUPT_SPEED_HEADROOM: f64 = 0.95;

/// WaypointArrival describes what should happen when orion reaches a destination
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaypointArrival {
    /// Through: treat this destination as part of an ongoing movement
    Through,
    /// Settle: Reach this destination with velocity and acceleration both at 0, so orion stops at the exact position
    Settle,
}

/// TrajectorWaypoint describes the complete set of instructions needed to reach a destination
#[derive(Clone, Debug)]
pub struct TrajectoryWaypoint {
    /// A human-readable label for this waypoint
    pub label: String,
    /// Destination angles for every joint
    pub positions: JointPositions,
    /// Requested travel from from preceeding point to this destination (relative not absolute)
    pub duration_seconds: f64,
    /// Whether to flow through or come to rest at this destination
    pub arrival: WaypointArrival,
    /// How long to hold at this destination after reaching it
    pub hold_seconds: f64,
    /// An optional named event attached to the arrival of this waypoint
    pub marker: Option<String>,
}
/// TrajectorSample is the planned state at one instant
/// It describes what Orion's joints are suposed to be doing at one particular moment in a movement
/// the values are calculated from the trajectory not readings from the servos
/// Each field answers a different question
/// - `positions` answers what angle should each joint be at this point
/// - `velocities` answers how quickly should each joint's angle be changing
/// - `accelerations` answers how quickly should each joint's velocity be changing
#[derive(Clone, Debug)]
pub struct TrajectorySample {
    pub positions: JointPositions,
    pub velocities: JointPositions,
    pub accelerations: JointPositions,
}

/// Polynomial represents one joint's movement during one travel segment
/// a travel segement is movement from one keyframe to the next
/// Its six coefficients define: p(t) = c0 + c1t + c2t² + c3t³ + c4t⁴ + c5t⁵
/// - p(t) is the joint angle.
/// - t is the time measured from the beginning of this segment.
/// - c0 through c5 determine the curve's shape.
/// One polynomial describes one joint, and we scale this by number of joints and number of travel segments.
/// If Orion has five joints and a motion has three travel segments, the compiled motion contains fifteen polynomials.
/// All five joint polynomials in a segment share the same duration.
/// That gives the joints a common timeline even though their individual angles and speeds differ.
#[derive(Clone, Copy, Debug)]
struct Polynomial {
    coefficients: [f64; 6],
}

impl Polynomial {
    /// Constructs a curve from 6 endpoint requirements
    /// -p0: start position
    /// -v0: start velocity
    /// -a0: start acceleration
    /// -p1: end position
    /// -v1: end velocity
    /// -a1: end acceleration
    /// -t: duration of the segment
    /// Our function calculates a curve that satisfies all six conditions over the chosen duration.
    /// Note: For a through waypoint, the ending acceleration and velocity can be nonzero, the next segment would start with those same values
    /// allowing the movement to flow smoothly through the waypoint.
    /// The function will do 3 things:
    /// - use the constant-acceleration equation (position = p0 + v0 x time + 1/2 x acceleration x time^2 ) alonside our starting position, velocity and acceleration
    ///   to calculate the polynomial coefficients c0, c1, c2,
    /// - compare those predictions with our desired ending, and calc the differences in position, velocity, and acceleration
    /// - calculate c3, c4, c5 to correct those difference, in order to reshape the movement so we arrive with the correct ending while preserving the starting position, velocity, and acceleration.
    fn quintic(p0: f64, v0: f64, a0: f64, p1: f64, v1: f64, a1: f64, t: f64) -> Self {
        // the flow for this can be summarised as
        // c0, c1, c2: "Begin from this position, velocity, and acceleration."
        // remaining differences: "Work out how that simple continuation misses our desired ending."
        // c3, c4, c5: "Shape the movement so we arrive with the correct ending."

        // The first three coefficients establish the starting position, velocity, and acceleration.
        // they form part of this formula position =  starting position + movement caused by starting velocity  +  movement caused by starting acceleration (position = p0 + v0 x time + 1/2 x acceleration x time^2 )
        let c0 = p0;
        let c1 = v0;
        // we divide our acceleration by 2 because it changes our velocity gradually over time
        // if that acceleration stayed constant, the angle travelled due to it would be ½ × acceleration × time²
        // this is added to the movement we already get from our starting velocity
        let c2 = a0 / 2.0;
        // Next we predict the ending position, velocity, and acceleration if the
        // starting acceleration stayed constant throughout the segment.
        // Subtract each prediction from its desired ending value.
        // These gives us the differences that the remaining polynomial terms must correct.
        let displacement = p1 - (c0 + c1 * t + c2 * t * t);
        let velocity = v1 - (c1 + 2.0 * c2 * t);
        let acceleration = a1 - 2.0 * c2;

        // finally we calculate the corrections we need to apply to our starting motion formula to correct for the error
        // c3,c4 and c5 bend the movement curve so the joint
        // -Begins with the specified position, velocity, and acceleration.
        // -Changes its motion during the segment.
        // -Finishes at the specified position, velocity, and acceleration.Ends with the specified position, velocity, and acceleration.
        let c3 =
            (10.0 * displacement - 4.0 * velocity * t + 0.5 * acceleration * t * t) / t.powi(3);
        let c4 = (-15.0 * displacement + 7.0 * velocity * t - acceleration * t * t) / t.powi(4);
        let c5 = (6.0 * displacement - 3.0 * velocity * t + 0.5 * acceleration * t * t) / t.powi(5);
        Self {
            coefficients: [c0, c1, c2, c3, c4, c5],
        }
    }

    // Sample the trajectory at a given time, returning the position, velocity, and acceleration.
    // it used the polynomial coefficients we have already calculated to sample the trajectory and return the position, velocity, and acceleration at that time.
    fn sample(self, t: f64) -> (f64, f64, f64) {
        let [c0, c1, c2, c3, c4, c5] = self.coefficients;
        let position = c0 + t * (c1 + t * (c2 + t * (c3 + t * (c4 + t * c5))));
        let velocity = c1 + t * (2.0 * c2 + t * (3.0 * c3 + t * (4.0 * c4 + t * 5.0 * c5)));
        let acceleration = 2.0 * c2 + t * (6.0 * c3 + t * (12.0 * c4 + t * 20.0 * c5));
        (position, velocity, acceleration)
    }
}

/// CompiledSegemnt packgages every joint's curve with the segment's timing
/// a segment corresponseds to "Travel to this waypoint, then perform its optional hold."
/// the three times are `starts_at`, `arrives_at`, and `holds_until`, and they are measured from the start of the whole trajectory.
#[derive(Clone, Debug)]
struct CompiledSegment {
    /// identifies the destination stage
    label: String,
    /// when travel beginds
    starts_at: f64,
    /// when travel reaches the destination
    arrives_at: f64,
    /// when destination holds finishes
    holds_until: f64,
    /// one movement formula per joint
    polynomials: BTreeMap<String, Polynomial>,
    /// exact destination joint angles
    /// we store target angles here to avoid recomputing them
    target: JointPositions,
    /// Optional event associated with arrival
    marker: Option<String>,
}

/// CompiledTrajectory represents the entire prepared movement
/// it contains the name of the movment, its ordered segemnts, its total compiled travel and hold time and the sampled estimat of its highest abs joint velocity
#[derive(Clone, Debug)]
pub struct CompiledTrajectory {
    name: String,
    segments: Vec<CompiledSegment>,
    duration_seconds: f64,
    peak_velocity_rad_s: f64,
}

impl CompiledTrajectory {
    /// prepares a movement and stretches travel times when necessary.
    /// start - says where the execution begins
    /// start_velocity - says how the joints are already moving
    /// waypoints = saus where this execution should go
    /// the compilation process is as follows:
    ///```text
    ///             validate inputs
    ///                      ↓
    ///        calculate initial durations from style
    ///                      ↓
    ///              build candidate curves
    ///                      ↓
    ///         estimate each segment's peak speed
    ///                      ↓
    ///         stretch segments that are too fast
    ///                      ↓
    ///        rebuild until acceptable, or reject
    /// ```
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

        // first we apply style to our requested travel times
        // for through waypoints duration = requested_duration / tempo
        // for settle waypoints duration = requested_duration / tempo  × (0.85 + 0.30 × settle_character)
        // as a result
        // Higher tempo shortens travel.
        // Lower tempo lengthens travel.
        // Higher settle_character gives settled arrivals more travel time.
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
        // we start our retiming loop which stretches a whole segment when one of its joints is too fast
        for _ in 0..RETIME_ITERATIONS {
            // compile a candidate trajectory alongside its durations
            let candidate = compile_with_durations(
                &name,
                &start,
                &start_velocity,
                &waypoints,
                &durations,
                style,
            )?;
            // assume we don't need to make any changes to the compiled trajectory
            let mut changed = false;
            // iterate over each segment of the compiled trajectory to check for peak velocity violations
            for (index, peak) in candidate.segment_peak_velocities().into_iter().enumerate() {
                // if the peak velocity is too high, scale the duration to keep it below the maximum,
                // then rebuild and check again
                // for example
                // estimated peak = 8 rad/s
                // ceiling        = 5 rad/s
                // duration multiplier = 8/5 × 1.015 = 1.624
                // the extra 1.015 adds a 1.5% margin
                if peak > maximum_velocity_rad_s * (1.0 + 1e-9) {
                    durations[index] *= (peak / maximum_velocity_rad_s) * 1.015;
                    // set the changed flag to true so we know to recompile with the updated durations
                    // this will allow us to retry with the corrected durations
                    changed = true;
                }
            }
            // if no changes were made, return the candidate as-is
            if !changed {
                return Ok(candidate);
            }
        }
        // build final candidate after our bounded retime loop
        let candidate = compile_with_durations(
            &name,
            &start,
            &start_velocity,
            &waypoints,
            &durations,
            style,
        )?;
        // check our final candidate's peak velocity
        // we treject the final candidate if its estimated peak speed remains
        // more than 0.1% above the maximum, and return an error
        if candidate.peak_velocity_rad_s > maximum_velocity_rad_s * 1.001 {
            return Err(Error::Runtime(format!(
                "Trajectory '{name}' could not be retimed below {maximum_velocity_rad_s:.3} rad/s."
            )));
        }
        // return our candidate as the result
        Ok(candidate)
    }

    /// Compile a trajectory similar to [`Self::compile`], but with additional joint angle checks
    /// Bounds starting speeds above the motor-speed ceiling, then reduces
    /// individual starting velocities when needed to keep the trajectory's
    /// 50 Hz position samples inside the calibrated joint-angle ranges.
    /// Returns an error if compilation cannot find an acceptable trajectory.
    /// this method is used to address cases of interruptions near a boundary.
    /// when we send a new position command near a boundary, for example
    /// permitted range:  -1.0 to +1.0 rad
    /// current position: +0.99 rad
    /// current velocity: +1.0 rad/s
    /// new target:        0.0 rad
    /// The target is inside the range. However, a curve preserving the positive starting velocity(1.0 rad/s) may initially move farther upward before turning toward zero.
    /// thereby exceeding our calibrated joint limits
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
        // first check the calibration data
        // we require
        // - Exactly one limit per starting joint.
        // - Unique names matching the starting map.
        // - Finite lower and upper bounds.
        // - lower_rad < upper_rad.
        // - A starting position inside each range.
        validate_limits(&start, limits)?;
        let mut blended_velocity = start_velocity;
        let bounded_start_speed = maximum_velocity_rad_s * INTERRUPT_SPEED_HEADROOM;
        // bound above ceiling starting speeds
        // we replace above-ceiling starting speeds with 95% of the ceiling,
        // preserving their direction using signum().
        // we leave other starting speeds unchanged.
        for velocity in blended_velocity.values_mut() {
            if velocity.abs() > maximum_velocity_rad_s {
                *velocity = velocity.signum() * bounded_start_speed;
            }
        }
        // we use a blend loop (18 max iterations) to iteratively adjust the velocity
        // until the trajectory passes the calibration limits
        for _ in 0..CALIBRATION_BLEND_ITERATIONS {
            // compile a candidate trajectory with the current blended velocity
            let candidate = Self::compile(
                name.clone(),
                start.clone(),
                blended_velocity.clone(),
                waypoints.clone(),
                style,
                maximum_velocity_rad_s,
            )?;
            // sample the candiate trajectory positions at 50hz every 0.02 seconds to see if it passes the calibration limits
            // we are essentually checkign positions at 0.00, 0.02, 0.04, ... final duration to see if the positions at those times are within the calibration limits
            let violations = candidate.calibration_violations(limits)?;
            // if not violations, return the candidate
            if violations.is_empty() {
                return Ok(candidate);
            }
            // if there are violations we halve the planned starting velocity for only those joints that are out of bounds
            // non offending joints are left at their current velocity
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
        // if there are still violations, return an error
        Err(Error::Runtime(format!(
            "Trajectory '{name}' could not preserve a calibration-safe interruption blend."
        )))
    }

    /// Samples the trajectory state at the given elapsed time, returning the positions, velocities, and accelerations.
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

    /// Samples the trajectory at the given elapsed time, returning the positions.
    pub fn sample(&self, elapsed_seconds: f64) -> Result<JointPositions> {
        Ok(self.sample_state(elapsed_seconds)?.positions)
    }

    /// Returns the progress of the trajectory as a value between 0.0 and 1.0.
    /// uses t / compiled duration clamped to [0.0, 1.0]
    pub fn progress(&self, elapsed_seconds: f64) -> Result<f64> {
        if !elapsed_seconds.is_finite() {
            return Err(Error::InvalidArgument(
                "Trajectory elapsed time must be finite.".into(),
            ));
        }
        Ok((elapsed_seconds / self.duration_seconds).clamp(0.0, 1.0))
    }

    /// Returns whether the planned timeline has finished, not whether the physical joints have settled.
    pub fn complete(&self, elapsed_seconds: f64) -> Result<bool> {
        Ok(self.progress(elapsed_seconds)? >= 1.0)
    }

    /// Returns the name of the trajectory.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the total compiled duration of the trajectory in seconds.
    /// including style timing, retiming and holds
    pub fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }

    /// Returns the sampled estimated peak velocity of the trajectory in radians per second.
    pub fn peak_velocity_rad_s(&self) -> f64 {
        self.peak_velocity_rad_s
    }

    /// Returns the name of the keyframe/segment at the given elapsed time.
    pub fn keyframe_name(&self, elapsed_seconds: f64) -> Result<&str> {
        Ok(&self.segments[self.segment_index(elapsed_seconds)?].label)
    }

    /// Returns the index of the keyframe/segment at the given elapsed time.
    pub fn keyframe_index(&self, elapsed_seconds: f64) -> Result<usize> {
        self.segment_index(elapsed_seconds)
    }

    /// Returns the number of keyframes/segments in the trajectory.
    pub fn keyframe_count(&self) -> usize {
        self.segments.len()
    }

    /// Returns the arrival time of the keyframe/segment at the given index.
    /// when the segment is reached, and before the hold period begins.
    pub fn keyframe_arrival_time(&self, index: usize) -> Option<f64> {
        self.segments.get(index).map(|segment| segment.arrives_at)
    }

    /// Returns the time at which the given marker is reached.
    pub fn marker_time(&self, marker: &str) -> Option<f64> {
        self.segments
            .iter()
            .find(|segment| segment.marker.as_deref() == Some(marker))
            .map(|segment| segment.arrives_at)
    }

    /// Returns all markers whose planned arrival time has been reached by the given elapsed time.
    pub fn reached_markers(&self, elapsed_seconds: f64) -> Vec<String> {
        self.segments
            .iter()
            .filter(|segment| elapsed_seconds >= segment.arrives_at)
            .filter_map(|segment| segment.marker.clone())
            .collect()
    }

    /// Returns the calibration violations for the given joint limits.
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

    /// estimates how fast each segment asks any joints to move
    fn segment_peak_velocities(&self) -> Vec<f64> {
        self.segments
            .iter()
            .map(|segment| {
                // for every segment, estimate the peak velocity by sampling the curve
                let duration = segment.arrives_at - segment.starts_at;
                // we sample 81 times 79 + 2 endpoints (start and end)
                (0..=OVERSHOOT_SAMPLES)
                    .flat_map(|sample| {
                        // at each sample point, estimate t(time) within the segment's duration
                        let t = duration * sample as f64 / OVERSHOOT_SAMPLES as f64;
                        segment
                            .polynomials
                            .values()
                            // call the sample method (returns position, velocity, acceleration)
                            // use .1 to get the velocity from the returned tuple
                            // use .abs() to get the absolute value of the velocity
                            .map(move |polynomial| polynomial.sample(t).1.abs())
                    })
                    // use .fold() to get the maximum velocity that is the highest individual velocity for that segment
                    .fold(0.0, f64::max)
            })
            // use .collect() to collect the results into a vector
            // so we have one peak estimate velocity for each segment
            // [segment 0 peak, segment 1 peak, segment 2 peak, ...]
            .collect()
    }

    /// Returns the index of the segment that is currently active at the given elapsed time.
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

/// compile_with_durations buulds one candidate trajectory using the supplied travel durations.
/// its job is to create the trajectory for one particular timing proposal
fn compile_with_durations(
    name: &str,
    start: &JointPositions,
    start_velocity: &JointPositions,
    waypoints: &[TrajectoryWaypoint],
    durations: &[f64],
    style: MotionStyle,
) -> Result<CompiledTrajectory> {
    // first create all the positions along our movement path
    // starting from the initial position, and adding each waypoint's position
    let mut points = vec![start.clone()];
    points.extend(waypoints.iter().map(|waypoint| waypoint.positions.clone()));
    // next calculate velocity and acceleration at each point along the path
    let mut velocities = derivative_maps(start, start_velocity, waypoints, durations, style, true);
    let mut accelerations =
        derivative_maps(start, start_velocity, waypoints, durations, style, false);
    // reduce problematic internal derivatives
    clamp_unrequested_overshoot(&points, durations, &mut velocities, &mut accelerations);
    let mut starts_at = 0.0;
    let mut segments = Vec::with_capacity(waypoints.len());
    // for each waypoint
    // 1 calculate its arrival and hold end times
    // 2 build one quintic per joint
    // 3 packages them into a compiled segment
    // 4 starts the next segment when this hold ends
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
    // create the compiled trajectory with calcuated segments and 0 peak velocity
    let mut trajectory = CompiledTrajectory {
        name: name.to_owned(),
        segments,
        duration_seconds: starts_at,
        peak_velocity_rad_s: 0.0,
    };
    // calculate the peak velocity from the segments
    trajectory.peak_velocity_rad_s = trajectory
        .segment_peak_velocities()
        .into_iter()
        .fold(0.0, f64::max);
    // return the compiled trajectory as a result
    Ok(trajectory)
}

/// derivative_maps calculates how joints enter and leave each waypoint
/// can be used to calculate derivatives for velocity or acceleration by using the velocity: bool flag
/// a derivative describes how a quantity changes
/// velocity is the first derivative of position
/// acceleration is the second derivative of position
/// it returns a vector of joint positions(<string, f64>) with each entry in the list corresponding to one path point along the trajectory,
/// and each map contains that points derivative value for every joint
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
                    // at the start of the trajectory, we use the supplied velocity when calculating for velocity and zero acceleration when calculating for acceleration
                    let value = if index == 0 {
                        if velocity { start_velocity[joint] } else { 0.0 }
                    }
                    // at the end of the trajectory or at any point that settles, we assume zero velocity and acceleration
                    else if index + 1 == points.len()
                        || waypoints[index - 1].arrival == WaypointArrival::Settle
                    {
                        0.0
                    }
                    // at any internal through point, we calculate the derivative
                    else {
                        // first examine the movement at both sides of this point to get the average neighboring velocities
                        // we will use the average to determine a sensible derivative at this point
                        let before = (points[index][joint] - points[index - 1][joint])
                            / durations[index - 1];
                        let after =
                            (points[index + 1][joint] - points[index][joint]) / durations[index];
                        // get the lag character for this joint we will use this to  soften the movement for different joints
                        let lag_character = joint_lag_character(joint, style.joint_lag);
                        // if we are calculating velocity
                        // we need to answer the question how fast should each joint be moving  as it passed through this point
                        if velocity {
                            // we first check to see if the joint needs to reverse direction using our average neighboring velocities
                            // if the joint needs to reverse, we set the derivative to 0 since we have to stop moving forward before reversing
                            // so its velocity will be 0 at this turning point
                            if before * after <= 0.0 {
                                0.0
                            } else {
                                // if the joint needs to keep moving forward, we blend the neighboring velocities to get a smooth transition
                                // if before and after are the same, the the velocity will also be the same
                                // if they are different, we choose a weighted average to blend them smoothly
                                // the shorter section gets more influence
                                let weighted = (before * durations[index]
                                    + after * durations[index - 1])
                                    / (durations[index - 1] + durations[index]);
                                // then we apply the following rules
                                // 1 - cap at 3 times the smaller of the two neighboring velocities to avoid choosing an excessively strong velocity
                                // 2 - multiply by tangent tension to control how stronly the movement flows through in accordance with the motion style
                                // 3 - multiply by the joint's lag factor to soften the movement for different joints
                                weighted.signum()
                                    * weighted.abs().min(3.0 * before.abs().min(after.abs()))
                                    * style.tangent_tension
                                    * lag_character
                            }
                        } else {
                            // if we are calculating acceleration, we need to answer a different question
                            // How should the speed be changing at this point?
                            // The code compares after with before:
                            // - Equal values suggest no change in velocity.
                            // - Different values suggest the joint should be changing its velocity.
                            // - The durations determine how quickly that change should happen.
                            // The style settings then adjust how strong that change is.
                            //  Here, overshoot_scale helps scale acceleration; it does not tell the joint to overshoot by a particular angle.
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

/// Varies derivative strength based on the joint's order and lag factor.
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

/// reduces curves that swing beyond their segment endpoints.
/// suppose a segment goes from 0.2 rad → 0.6 rad, its endpoint interval is approx [0.2, 0.6]
/// a quintic can satisfy the endpoint interval exactly, but may swing beyond it by briefly reaching 0.65 in between. This is called a bulge.
/// we use this function to try and reduce the extra excess
/// for each segment and joint, it
/// - build the candidate points
/// - finds the lower and upper bounds of the endpoint interval
/// - samples 79 intermediate points along the curve
/// - records the segment and joint if any sample leaves that interval
/// we have a 1e-9 tolerance around the endpoints to accommodate numerical noise
/// offenders are stored as a BTreeSet of (segment index, joint name) pairs
/// For every offending segment, we halve the velocity and accelration at its bordering internal points
/// reducing the derivative makes the curve less likey to swing outside the endpoint interval
/// Note: because we are sampling the curve, we don't prove the entire curve stays within the endpoint interval, just at the sample points
/// also we only do 8 passes to reduce bulges.
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
                // the following if statement means leave the exec start and final point alone
                // we do this because
                // - The initial velocity represents the motion the compiler was asked to blend from.
                // - The final derivatives are already zero.
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

/// validate_inputs validates the inputs for a trajectory.
/// It rejects:
/// - An empty name.
/// - An empty starting map.
/// - An empty waypoint list.
/// - Different joint names in starting positions and starting velocities.
/// - A non-finite or non-positive speed ceiling.
/// - Waypoints containing different joint names.
/// - Non-finite or non-positive travel durations.
/// - Non-finite or negative holds.
/// - A positive hold attached to a Through waypoint.
/// - Non-finite waypoint positions.
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
    // start and start_velocity are oth BTreeMaps with sorted keys, so we can compare them directly
    // to ensure they have the same keys
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

/// Validate the provided joint limits.
/// It requires:
/// - Exactly one limit per starting joint.
/// - Unique names matching the starting map.
/// - Finite lower and upper bounds.
/// - lower_rad < upper_rad.
/// - A starting position inside each range.
/// The bounds are inclusive for the start-position check.
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

/// JointTrajectory is a convenience wrapper for moving to one target.
/// This is a tuple struct with one private field. Self.0 which refers to the wrapped `CompiledTrajectory`.
/// It represents a single destination, not necessarily a single joint.
/// It gives direct goto operations a simpler interface while using the same compiler as authored multi-keyframe motions.
#[derive(Clone, Debug)]
pub struct JointTrajectory(CompiledTrajectory);

impl JointTrajectory {
    /// Creates a new JointTrajectory, and supplies zero starting velocity.
    pub fn new(
        name: impl Into<String>,
        start: JointPositions,
        target: JointPositions,
        duration_seconds: f64,
    ) -> Result<Self> {
        let zero_velocity = start.keys().map(|name| (name.clone(), 0.0)).collect();
        Self::with_start_velocity(name, start, zero_velocity, target, duration_seconds)
    }

    /// Creates a new JointTrajectory with the given starting velocity.
    pub fn with_start_velocity(
        name: impl Into<String>,
        start: JointPositions,
        start_velocity: JointPositions,
        target: JointPositions,
        duration_seconds: f64,
    ) -> Result<Self> {
        Self::compile(name, start, start_velocity, target, duration_seconds, None)
    }

    /// Creates a new JointTrajectory with the given starting velocity, calibrated to the given joint limits.
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

    /// Compile creates a single waypoint trajectory with a dedicated goto motion style
    /// allows us to do single goto movements without needing to define a multi-waypoint trajectory.
    fn compile(
        name: impl Into<String>,
        start: JointPositions,
        start_velocity: JointPositions,
        target: JointPositions,
        duration_seconds: f64,
        limits: Option<&[JointLimit]>,
    ) -> Result<Self> {
        // default motion style for goto commands
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
        // create one-waypoint trajectory
        // we try to preserve the duration requested by the caller,
        // but retiming operations might change it if needed
        let waypoints = vec![TrajectoryWaypoint {
            label: "target".into(),
            positions: target,
            duration_seconds,
            arrival: WaypointArrival::Settle,
            hold_seconds: 0.0,
            marker: None,
        }];

        // if joint limits are supplied
        // we calibrate the trajectory to respect them
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
            // otherwise we use the default compile method
            CompiledTrajectory::compile(
                name,
                start,
                start_velocity,
                waypoints,
                style,
                STS3215_MAX_SPEED_RAD_S,
            )?
        };
        // wrap the compiled trajectory in a JointTrajectory struct and return it
        Ok(Self(trajectory))
    }
    // the following methods delegate to the underlying CompiledTrajectory to invoke
    // the corresponding methods on the compiled trajectory
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
