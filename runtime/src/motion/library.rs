//! Our motion.rs defines enums and structs that describe and represent orions movements.
//! it represents where the joints should go, how long it should take to get there, and
//! whether it flows through a position or settles at the target.
use crate::motion::shared::{collect_yaml_files, is_semantic_name};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::devices::driver::JointLimit;
use crate::error::{OrionRuntimeError, Result};
use crate::motion::pose::{JointPositions, PoseLibrary};
use crate::motion::style::MotionStyle;
use crate::motion::trajectory::{
    CompiledTrajectory, STS3215_MAX_SPEED_RAD_S, TrajectorySample, TrajectoryWaypoint,
    WaypointArrival,
};

pub const MOTION_FORMAT_VERSION: u32 = 2;

/// This enum represents how every keyframe's target is interpreted.
/// - Absolute: the keyframe's target values are actual target joint angles in radians
/// - AnchorRelative: the keyframe's target values are angular offsets from a fixed reference pose - the anchor pose
/// Each relative keyframe starts from the same anchor, in order to prevent accumulating angular offsets over time,
/// and drifting away from the original pose.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MotionSpace {
    Absolute,
    AnchorRelative,
}

/// This enum reprsents what action orion should take when it reached a keyframe
/// - Through: continue through this keyframe as part of a flowing action
/// - Settle: Arrive with zero velocity and acceleration, producing a smooth stop
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum KeyframeArrival {
    Through,
    Settle,
}

/// Defines conversion logic from our motion level intent [`KeyframeArrival`] to
/// the trajectory layer equivalent [`WaypointArrival`].
impl From<KeyframeArrival> for WaypointArrival {
    fn from(value: KeyframeArrival) -> Self {
        match value {
            KeyframeArrival::Through => Self::Through,
            KeyframeArrival::Settle => Self::Settle,
        }
    }
}

/// A MotionKeyframe defines describes a destination (joint positions) for orion that will be reached
/// during a motion sequence and the instructions for reaching it.
/// pose_name and target serve different purposes: the name preserves which authored pose was selected,
/// while the target contains the numerical data needed for movement.
/// It is one stage or step in a motion sequence.
/// - pose_name: Optional name of the pose supplying our target/destination, absolute keyframes use some(pose_name), relative use none
/// - target: a map of joint names to numbers values which represent absolute angles or relative offsets,
///     essentially where we want to joints to be at this point of the motion sequence
///     Our Joint positions here s an alias for BTreeMap<String, f64>. It stores values such as:
///     "head_pitch_joint" => 0.09
///     The map itself does not distinguish an angle from an offset; the enclosing motion’s space supplies that meaning.
/// - duration_seconds: Authored travel time from the previous stage or the starting state to this target.
/// - arrival: Whether to flow through or settle at this target.
/// - hold_seconds: Time to remain at the target after arriving.
/// - marker: Optional event label associated with this stage,
///     allowing a scene to coordinate something such as lighting or sound with the movement.
#[derive(Clone, Debug)]
pub struct MotionKeyframe {
    pub pose_name: Option<String>,
    pub target: JointPositions,
    pub duration_seconds: f64,
    pub arrival: KeyframeArrival,
    pub hold_seconds: f64,
    pub marker: Option<String>,
}

/// MotionDefintion represents a complete motion sequence
/// - name: A unique identifier for the motion.
/// - description: Human-readable description of the motion.
/// - space: Whether its targets are absolute angles or anchor-relative offsets..
/// - style: Expression parameters such as tempo, amplitude, and settling character.
/// - return_to_anchor: Declares that the motion should finish at its reference posture.
/// - keyframes: The sequence of keyframes defining the motion.x§
/// `return_to_anchor` does not append a return movement. For relative motions, the loader
/// requires this flag and validates that the authored final keyframe has zero offsets,
/// so it resolves to the fixed anchor (which may differ from the starting position).
#[derive(Clone, Debug)]
pub struct MotionDefinition {
    pub name: String,
    pub description: String,
    pub space: MotionSpace,
    pub style: MotionStyle,
    pub return_to_anchor: bool,
    pub keyframes: Vec<MotionKeyframe>,
}

impl MotionDefinition {
    /// Collects the marker names for all keyframes in this motion.
    /// Markers are named moments in a motion that let Orion coordinate movement with sound and lighting
    /// we attach them to specific keyframes so that when we reach that keyframe, the scene cordinator can
    /// use that to trigger lighting or sound effects.
    pub fn markers(&self) -> Vec<String> {
        self.keyframes
            .iter()
            .filter_map(|kf| kf.marker.clone())
            .collect()
    }

    /// Converts keyframes into actual joint targets using a scale of 1.0.
    /// When the motion space is relative, the anchor is used to resolve relative offsets.
    /// When the motion space is absolute, the anchor is ignored.
    /// On success, returns an ordered vector of joint positions.
    pub fn resolved_targets(&self, anchor: &JointPositions) -> Result<Vec<JointPositions>> {
        self.resolved_targets_with_scale(anchor, 1.0)
    }

    /// Returns a uniform amplitude scale for the motion, based on the motion space and joint limits.
    /// The goal here is to determine how much to scale the motion to fit within the joint limits.
    /// When the motion space is absolute, the scale is always 1.0.
    /// When the motion space is relative, the scale is calculated to fit the joint limits.
    pub fn uniform_amplitude_scale(
        &self,
        anchor: &JointPositions,
        limits: &[JointLimit],
    ) -> Result<f64> {
        // absolute motions do not need scaling to fit within limits, we can return 1
        if self.space == MotionSpace::Absolute {
            return Ok(1.0);
        }
        // build a lookup table of joint limits by name
        let by_name: BTreeMap<&str, &JointLimit> = limits
            .iter()
            .map(|limit| (limit.name.as_str(), limit))
            .collect();

        // set the default scale to 1
        // this assumes that the motion will fit within the limits, so we start with a scale of 1
        let mut scale: f64 = 1.0;

        for keyframe in &self.keyframes {
            // for each joint and its offset value in our relative keyframe
            for (joint, offset) in &keyframe.target {
                // check to see if the anchor position contains a value for this joint
                // retrieve it if so, otherwise return an error
                let anchor_value = anchor.get(joint).ok_or_else(|| {
                    OrionRuntimeError::InvalidArgument(format!(
                        "Idle anchor omits Orion joint '{joint}'."
                    ))
                })?;
                // check to see if our limits contain a value for this joint
                // retrieve it if so, otherwise return an error
                let limit = by_name.get(joint.as_str()).ok_or_else(|| {
                    OrionRuntimeError::InvalidArgument(format!(
                        "Calibration omits Orion joint '{joint}'."
                    ))
                })?;

                // calculate the styled offset for this joint
                let styled_offset = offset * self.style.amplitude;
                // a positive offset moves the joint towards the upper limit,
                //  a negative offset moves it towards the lower limit
                //
                // Note: we pick the minmum scale value for each iteration
                // when we get a new scale value, we replace the current one if it's smaller
                // so that means we choose the "Strictest" scale value (the smallest allowed multiplier)
                if styled_offset > 0.0 {
                    // calculate how much of our postive offset fits within the joint's range
                    // allowed scale = available upward movement / requested upward movement
                    // for example
                    // Suppose the head pitch upper limit is 0.25, with anchor 0.20:
                    // available movement = 0.25 - 0.20 = 0.05
                    // requested movement = 0.08
                    // allowed scale      = 0.05 / 0.08 = 0.625
                    // therefor scale = scale = min(1.0, 0.625) = 0.625
                    // Note: we pick the minmum scale value for each iteration
                    // when we get a new scale value, we replace the current one if it's smaller
                    scale = scale.min((limit.upper_rad - anchor_value) / styled_offset);
                }
                // negative offset moves towards lower limit
                // For example:
                // anchor        =  0.20
                // lower limit   =  0.17
                // styled offset = -0.04
                // allowed scale = (0.17 - 0.20) / -0.04
                //               = -0.03 / -0.04
                //               = 0.75
                else if styled_offset < 0.0 {
                    // Note: we pick the minmum scale value for each iteration
                    // when we get a new scale value, we replace the current one if it's smaller
                    scale = scale.min((limit.lower_rad - anchor_value) / styled_offset);
                }
            }
        }
        // Restricts the answer to the range 0.0..=1.0 and returns it
        Ok(scale.clamp(0.0, 1.0))
    }

    /// returns a list of joint positions for each keyframe in the motion definition
    /// For each keyframe it calculates actual target = anchor + (offset × amplitude × scale)
    /// and returns a list of these resolved targets.
    /// Note: amplitude controls the intended size of the expression.
    /// scale applies an additional adjustment—for example, the reduction calculated to fit joint limits.
    pub(crate) fn resolved_targets_with_scale(
        &self,
        anchor: &JointPositions,
        scale: f64,
    ) -> Result<Vec<JointPositions>> {
        // our logic depends on the motion space (absolute or anchor-relative)
        match self.space {
            // if absolute, copy and return the targets for each keyframe as a list
            MotionSpace::Absolute => Ok(self
                .keyframes
                .iter()
                .map(|keyframe| keyframe.target.clone())
                .collect()),
            // if anchor is relative, we need to calculate the target for each keyframe
            MotionSpace::AnchorRelative => self
                .keyframes
                .iter()
                .map(|keyframe| {
                    // clone the anchor joint positions for our relative movements
                    // this will be our starting point for each relative movement
                    let mut target = anchor.clone();
                    // get the joint and its offset for each keyframe
                    // note: when space is relative, our target values are offsets from the anchor
                    for (joint, offset) in &keyframe.target {
                        // check our anchor to see if it contains the current joint
                        // return an error if it does not
                        let value = target.get_mut(joint).ok_or_else(|| {
                            OrionRuntimeError::InvalidArgument(format!(
                                "Relative motion '{}' contains unknown joint '{joint}'.",
                                self.name
                            ))
                        })?;

                        // since all our relative movements are offsets,
                        // we will update the cloned anchor starting point to our desired relative target
                        // we do this by adding the offset * amplitude * scale  to the initial anchor position
                        // and this gives us our final target
                        *value += offset * self.style.amplitude * scale
                    }
                    // return our updated target as a result
                    Ok(target)
                })
                .collect(), // collect the results into a vector and return it
        }
    }

    /// Returns the final target of the motion, given an anchor point
    /// resolves all keyframes using our resolved_targets method
    /// if any keyframe fails to resolve, returns an error
    /// Removes and returns the last map from the newly created vector:
    /// - Some(last_target) if the vector contains keyframes.
    /// - None if it is empty.
    /// This modifies only the temporary vector of resolved targets. It does not remove a keyframe from self.keyframes.
    pub fn final_target(&self, anchor: &JointPositions) -> Result<JointPositions> {
        self.resolved_targets(anchor)?.pop().ok_or_else(|| {
            OrionRuntimeError::InvalidArgument("Motion has no final keyframe.".into())
        })
    }
}

/// Motion document represents the top-level headers of our motion yaml files.
/// One MotionDocument corresponds to one yaml file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MotionDocument {
    #[serde(default)]
    format_version: u32,
    motion: Option<MotionEntry>,
}

/// MotionEntry represents a single motion sequence defined in a yaml file.
/// It is the actual motion defined in the file including its name, description, and keyframes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MotionEntry {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    space: MotionSpace,
    #[serde(default)]
    style: String,
    #[serde(default)]
    return_to_anchor: bool,
    #[serde(default)]
    keyframes: Vec<KeyframeEntry>,
}

/// KeyframeEntry rerpeesnets the defined keyframes within the motion yaml file.
/// It contains the pose, offsets, duration, arrival, and hold values for a single keyframe.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyframeEntry {
    #[serde(default)]
    pose: Option<String>,
    #[serde(default)]
    offsets: Option<JointPositions>,
    duration: f64,
    arrival: KeyframeArrival,
    #[serde(default)]
    hold: f64,
    #[serde(default)]
    marker: Option<String>,
}

/// The MotionLibrary is a collection of MotionDefinitions loaded from the yaml files.
/// it stores a private map of motion names to MotionDefinitions.
/// MotionLibrary does two jobs:
/// - loading motion definitions into memory
/// - letting the rest of the orion runtime access the motion definitions
#[derive(Clone, Debug)]
pub struct MotionLibrary {
    motions: BTreeMap<String, MotionDefinition>,
}

impl MotionLibrary {
    /// Loads motion definitions from yaml files in the given directory into memory.
    pub fn load(directory: impl AsRef<Path>, poses: &PoseLibrary) -> Result<Self> {
        let directory = directory.as_ref();
        if !directory.is_dir() {
            return Err(OrionRuntimeError::Runtime(format!(
                "Motion library is not a directory: {}",
                directory.display()
            )));
        }
        let mut files = Vec::new();
        collect_yaml_files(directory, &mut files, "motion library")?;
        files.sort();
        if files.is_empty() {
            return Err(OrionRuntimeError::Runtime(format!(
                "Motion library contains no YAML files: {}",
                directory.display()
            )));
        }
        let mut motions = BTreeMap::new();
        for path in files {
            let motion = load_motion_file(&path, poses)?;
            if motions.insert(motion.name.clone(), motion).is_some() {
                return Err(OrionRuntimeError::Runtime(format!(
                    "Duplicate Orion motion name in {}",
                    path.display()
                )));
            }
        }
        Ok(Self { motions })
    }

    /// Returns the motion definition for the given name, or an error if the motion is not found.
    pub fn motion(&self, name: &str) -> Result<&MotionDefinition> {
        self.motions.get(name).ok_or_else(|| {
            OrionRuntimeError::InvalidArgument(format!("Unknown Orion motion: {name}"))
        })
    }

    /// Returns a list of all motion names in the library.
    pub fn names(&self) -> Vec<String> {
        self.motions.keys().cloned().collect()
    }

    /// Returns an iterator over all motion names and definitions in the library.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &MotionDefinition)> {
        self.motions.iter()
    }
}

/// converts one yaml motion file into a validated MotionDefinition.
/// it checks whether the authored motion maes sense, resolves named poses in the motion
/// into joint positions and preserves the timing and marker information needed for playback.
/// - path identifies one motion file
/// - poses supplies the already loaded named poses, so we can resolve poses mentioned in a motion back
///   into joint angles
fn load_motion_file(path: &Path, poses: &PoseLibrary) -> Result<MotionDefinition> {
    // read the motion file contents
    let contents: String = fs::read_to_string(path).map_err(|error| {
        OrionRuntimeError::Runtime(format!(
            "Could not read motion file '{}': {error}",
            path.display()
        ))
    })?;

    // we first check format_version before parsing the rest of the file
    // so we can error out early if the format is not supported with a clear
    // format version mismatch error message
    // Value here represents raw yaml mappings without requiring an exact struct
    let header: serde_yaml::Value = serde_yaml::from_str(&contents).map_err(|error| {
        OrionRuntimeError::Runtime(format!(
            "Could not parse motion file '{}': {error}",
            path.display()
        ))
    })?;

    // validate format_version before parsing the rest of the file
    if header
        .get("format_version")
        .and_then(serde_yaml::Value::as_u64)
        != Some(MOTION_FORMAT_VERSION as u64)
    {
        return Err(OrionRuntimeError::Runtime(
            "Motion file must use format_version 2 (v2 required).".into(),
        ));
    }

    // convert the yaml string to a MotionDocument struct
    let document: MotionDocument = serde_yaml::from_str(&contents).map_err(|error| {
        OrionRuntimeError::Runtime(format!(
            "Could not parse motion file '{}': {error}",
            path.display()
        ))
    })?;

    // validate the format_version field
    if document.format_version != MOTION_FORMAT_VERSION {
        return Err(OrionRuntimeError::Runtime(
            "Motion file must use format_version 2 (v2 required).".into(),
        ));
    }

    // check to see that the motion document contains the motion details
    // the motion: section of the yaml file
    let entry = document.motion.ok_or_else(|| {
        OrionRuntimeError::Runtime("Motion file must contain a motion mapping.".into())
    })?;

    // validate the supplied motion details
    // name must be valid and keyframes not empty
    if !is_semantic_name(&entry.name) || entry.keyframes.is_empty() {
        return Err(OrionRuntimeError::Runtime(
            "Motion requires a semantic name and at least one keyframe.".into(),
        ));
    }

    // validate that supplied motion style is part of our
    // supported motion style parameters, including tempo and amplitude.
    let style = MotionStyle::named(&entry.style).map_err(|error| {
        OrionRuntimeError::Runtime(format!(
            "Motion '{}' has invalid style: {error}",
            entry.name
        ))
    })?;

    // we need to validate that movement space and the return to anchor flag agree
    // absolute motion needs return_to_anchor: false
    // anchor-relative motion needs return_to_anchor: true
    // For Orion, relative clips allow gestures such as breathing to work around different postures.
    // Requiring them to return to their anchor prevents an authored clip from leaving a persistent displacement.
    if entry.space == MotionSpace::AnchorRelative && !entry.return_to_anchor {
        return Err(OrionRuntimeError::Runtime(format!(
            "Anchor-relative motion '{}' must set return_to_anchor: true.",
            entry.name
        )));
    }
    if entry.space == MotionSpace::Absolute && entry.return_to_anchor {
        return Err(OrionRuntimeError::Runtime(format!(
            "Absolute motion '{}' cannot set return_to_anchor.",
            entry.name
        )));
    }

    // the next step here is validate and covert each keyframe into our internal representation

    let mut markers = BTreeSet::new(); // keeps track of all marker encountered
    let mut keyframes = Vec::with_capacity(entry.keyframes.len()); // holds the converted runtime keyframes

    // for each input keyframe, timing must satisfy
    // - duration finitte and greater than 0
    // - hold finite and at least zero
    for keyframe in entry.keyframes {
        if !keyframe.duration.is_finite()
            || keyframe.duration <= 0.0
            || !keyframe.hold.is_finite()
            || keyframe.hold < 0.0
        {
            return Err(OrionRuntimeError::Runtime(format!(
                "Motion '{}' keyframes require positive duration and non-negative hold.",
                entry.name
            )));
        }

        // reject contradictory keyframe settings, through means keep moving through the target
        // a hold means remain there, so ay keyframe that holds must settle
        if keyframe.arrival == KeyframeArrival::Through && keyframe.hold > 0.0 {
            return Err(OrionRuntimeError::Runtime(format!(
                "Motion '{}' cannot hold a through keyframe.",
                entry.name
            )));
        }

        // any marker must be a valid name and unique within the motion
        if keyframe
            .marker
            .as_deref()
            .is_some_and(|marker| !is_semantic_name(marker) || !markers.insert(marker.to_owned()))
        {
            return Err(OrionRuntimeError::Runtime(format!(
                "Motion '{}' markers must be unique semantic names.",
                entry.name
            )));
        }
        // convert the keyframe according to it coordinate meaning
        // both branches produce the same pair of fields, but target has a different meaning in each space
        let (pose_name, target) = match entry.space {
            // for absolute motion, the loader
            // - rejects an offsets field
            // - requires a nonempty pose name
            // - looks up the pose in the pose library
            // - clones the poses joint positions into the keyframe
            MotionSpace::Absolute => {
                if keyframe.offsets.is_some() {
                    return Err(OrionRuntimeError::Runtime(format!(
                        "Absolute motion '{}' keyframes use pose, not offsets.",
                        entry.name
                    )));
                }
                let pose = keyframe
                    .pose
                    .filter(|pose| !pose.is_empty())
                    .ok_or_else(|| {
                        OrionRuntimeError::Runtime(format!(
                            "Absolute motion '{}' keyframe requires a pose.",
                            entry.name
                        ))
                    })?;
                let target = poses
                    .pose(&pose)
                    .map_err(|error| {
                        OrionRuntimeError::Runtime(format!(
                            "Invalid pose reference in '{}': {error}",
                            path.display()
                        ))
                    })?
                    .clone();
                (Some(pose), target)
            }
            // for anchor-relative motion, the loader
            // - rejects a pose field
            // - checks that every supplied joint name belongs to Orion's joint set
            // - checks that every supplied joint offset is finite
            // - an empty offsets field becomes an emoty map through unwrap_or_default(). This means no displacement from the anchor
            MotionSpace::AnchorRelative => {
                if keyframe.pose.is_some() {
                    return Err(OrionRuntimeError::Runtime(format!(
                        "Anchor-relative motion '{}' keyframes use offsets, not pose.",
                        entry.name
                    )));
                }
                let offsets = keyframe.offsets.unwrap_or_default();
                if offsets
                    .keys()
                    .any(|joint| !crate::ORION_JOINT_NAMES.contains(&joint.as_str()))
                    || offsets.values().any(|value| !value.is_finite())
                {
                    return Err(OrionRuntimeError::Runtime(format!(
                        "Anchor-relative motion '{}' contains an invalid joint offset.",
                        entry.name
                    )));
                }
                (None, offsets)
            }
        };
        //build the runtime keyframe
        keyframes.push(MotionKeyframe {
            pose_name,
            target,
            duration_seconds: keyframe.duration,
            arrival: keyframe.arrival,
            hold_seconds: keyframe.hold,
            marker: keyframe.marker,
        });
    }
    // check how the whole motion ends
    // the last keyframe in our motion must settle
    // internal keyframes may flow through targetes, but the complete motion must have a stopping point
    if keyframes
        .last()
        .is_some_and(|keyframe| keyframe.arrival != KeyframeArrival::Settle)
    {
        return Err(OrionRuntimeError::Runtime(format!(
            "Motion '{}' final keyframe must settle.",
            entry.name
        )));
    }
    // for relative motions, the final offsets must be effectively zero
    // meaning we return back to our anchor position
    // we use offset.abs() > 1e-12 so that we permit a tiny numerical tolerance
    // around zero
    if entry.space == MotionSpace::AnchorRelative
        && keyframes
            .last()
            .is_some_and(|keyframe| keyframe.target.values().any(|offset| offset.abs() > 1e-12))
    {
        return Err(OrionRuntimeError::Runtime(format!(
            "Anchor-relative motion '{}' must finish at zero offsets.",
            entry.name
        )));
    }

    // return the compiled motion definition
    Ok(MotionDefinition {
        name: entry.name,
        description: entry.description,
        space: entry.space,
        style,
        return_to_anchor: entry.return_to_anchor,
        keyframes,
    })
}

/// MotionSequence represents a sequence of compiled trajectories for a motion.
/// it is a motion prepared for playback from a particular starting state.
/// MotionDefinition describes an authored movement, and its keyframe and markers,
/// while MotionSequence turns that descrition into that a timed path.
/// The conversion depends on where orion starts, how fast its joints are already moving and its calibrated limits
/// So in summary MotionDefinition describes the movement we want Orion to perform—where its joints should go and how long each step should take.
/// MotionSequence prepares that movement using Orion's starting position and speed. It calculates where each joint should be throughout the movement.
/// During playback, it allows us to answer the question "We're 0.42 seconds into the movement—where should each joint be now?"
/// If we have a movement from A to B lasting 5 seconds, the motion sequnce calculates the movement path from A to B, over the course of 5 seconds,
/// and stores the calculated moevment plan in the trajectory field.
#[derive(Clone, Debug)]
pub struct MotionSequence {
    // our trajectory stores the calculated movement plan
    // it contains the
    // - curves used to calculate each joint's position
    // - when each movement segment starts and ends
    // - keyframe lables and markers
    // - total movement duration
    // This acts as a reproducible formula for calculating joint positions throughout the movemenent
    // the formula is essentially a smooth curve for each joint
    // we can use the formula to determine where joints should be at any time eg at 0.42 seconds into the movement
    // During compliation, each keyframe produces one segment
    // A keyframe describes a destination. Its segment describes the movement to that destination, plus any hold afterward.
    // Each segment contains the joint curves, arrival time, hold end time, and optional marker associated with its destination keyframe.
    trajectory: CompiledTrajectory,
}

/// The following args recur across our methods
/// motion: Which authored movement to prepare.
/// start: Joint angles where this execution begins.
/// start_velocity: How fast each joint is moving at that beginning, in radians per second.
/// anchor: The fixed reference posture for relative keyframes.
/// amplitude_scale: An additional multiplier from 0.0 to 1.0 for relative offsets.
/// limits: Calibrated joint-angle ranges used by the calibrated compiler.
/// Note:
/// start and anchor can differ.
/// Orion might be midway through a breathing gesture when a new gesture begins. Its head is currently at 0.24 radians, but the posture around which it gestures is 0.20 radians
/// in that case, start would be 0.24 but anchor would be 0.20
impl MotionSequence {
    /// new creates a new motion sequence with the simplest starting assumptions used to create the trajectory
    /// we assume that the starting posture is the same as the anchor posture, and that the joints are stationary
    pub fn new(motion: &MotionDefinition, start: JointPositions) -> Result<Self> {
        // create a zero velocity start
        // each joint is stationary (velocity is 0.0)
        let velocity = start.keys().map(|joint| (joint.clone(), 0.0)).collect();
        // compile the trajectory using the zero velocity start
        Self::compile(motion, start.clone(), velocity, start)
    }

    /// Creates a motion sequence using the provided starting positions,
    /// starting velocities, and anchor.
    /// Uses an additional amplitude scale of 1.0, so relative offsets
    /// retain their full styled size (no scaling to fit within calibrated limits).
    /// Does not check calibrated joint-angle limits.
    pub fn compile(
        motion: &MotionDefinition,
        start: JointPositions,
        start_velocity: JointPositions,
        anchor: JointPositions,
    ) -> Result<Self> {
        Self::compile_scaled(motion, start, start_velocity, anchor, 1.0)
    }

    /// compile_scaled creates a new motion sequence using the provided start and anchor positions,
    /// start velocity, and a provided amplitude scale to scale the motion to fit within any limits,
    /// The caller chooses the scale; this method does not calculate it
    /// or check calibrated joint-angle limits.
    pub fn compile_scaled(
        motion: &MotionDefinition,
        start: JointPositions,
        start_velocity: JointPositions,
        anchor: JointPositions,
        amplitude_scale: f64,
    ) -> Result<Self> {
        Self::compile_scaled_inner(motion, start, start_velocity, anchor, amplitude_scale, None)
    }

    /// Creates a motion sequence using the provided starting positions,
    /// starting velocities, anchor, and amplitude scale.
    /// Applies the scale to relative motion offsets and checks the resulting
    /// trajectory's sampled positions against calibrated joint-angle limits.
    /// May reduce the starting velocities used in the plan to keep it within
    /// those limits, or returns an error if compilation cannot succeed.
    /// This is the path uses by our runtime for motion playback on the robot
    /// The calibration limits check is neccessary here because valid destinations alone do not
    /// guarantee that the movement between them stays within range. An initial velocity toward a nearby limit
    /// can cause the calculated transition to leave the permitted range before turning back.
    pub fn compile_scaled_calibrated(
        motion: &MotionDefinition,
        start: JointPositions,
        start_velocity: JointPositions,
        anchor: JointPositions,
        amplitude_scale: f64,
        limits: &[JointLimit],
    ) -> Result<Self> {
        Self::compile_scaled_inner(
            motion,
            start,
            start_velocity,
            anchor,
            amplitude_scale,
            Some(limits),
        )
    }

    /// Internal helper for compiling a motion sequence with scaled offsets and optional joint-angle limits.
    fn compile_scaled_inner(
        motion: &MotionDefinition,
        start: JointPositions,
        start_velocity: JointPositions,
        anchor: JointPositions,
        amplitude_scale: f64,
        limits: Option<&[JointLimit]>,
    ) -> Result<Self> {
        // first we validate the amplitude scale is within the valid range
        // amplitude_scale controls how much of a relative gesture to use for relative keyframes:
        // - 1.0: use its full size.
        // - 0.5: use half its size.
        // - 0.0: remove its offsets from the anchor.
        // Note: a value of 0.0  every relative keyframe's target becomes the anchor, so if
        // - Already at the anchor and stationary: it stays there.
        // - Starting away from the anchor: it moves back to the anchor.
        // - Already moving: the trajectory must account for that starting velocity; zero amplitude doesn't mean an instant stop.
        if !amplitude_scale.is_finite() || !(0.0..=1.0).contains(&amplitude_scale) {
            return Err(OrionRuntimeError::InvalidArgument(
                "Motion amplitude scale must be between zero and one.".into(),
            ));
        }

        // work out the target joint angles (destination) for each keyframe, if the motion is relative, this calculates how far to move from the anchor
        // if the motion is absolute, the keyframe already contains the target joint angles, so we copy it directly
        let targets = motion.resolved_targets_with_scale(&anchor, amplitude_scale)?;

        // combine each target joint angle (destination) with its movement instructions into a TrajectoryWaypoint
        // The TrajectoryWaypoint can be though off as one complete instruction on how we reach each destination so essentially
        // "Go to these joint angles, take this long to get there and either keep moving through them or stop there"
        let waypoints = motion
            .keyframes
            .iter()
            .zip(targets)
            .map(|(keyframe, positions)| TrajectoryWaypoint {
                label: keyframe
                    .pose_name
                    .clone()
                    .unwrap_or_else(|| format!("{}-relative", motion.name)),
                positions,
                duration_seconds: keyframe.duration_seconds,
                arrival: keyframe.arrival.into(),
                hold_seconds: keyframe.hold_seconds,
                marker: keyframe.marker.clone(),
            })
            .collect();

        // now we use our trajectory compiler to calcuate the movement between our target joint angles (destinations)
        // The trajectory compiler calculates smooth movement along that path. It uses:
        // - start: where the joints begin.
        // - start_velocity: how fast they are already moving.
        // - waypoints: where to go and the requested timing.
        // - motion.style: how the movement should feel.
        // - STS3215_MAX_SPEED_RAD_S: the speed ceiling used by the compiler
        // if the joint limits were supplied then we use the calibrated compile method to account for them
        // else we use the standard compile method
        let trajectory = if let Some(limits) = limits {
            CompiledTrajectory::compile_calibrated(
                motion.name.clone(),
                start,
                start_velocity,
                waypoints,
                motion.style,
                STS3215_MAX_SPEED_RAD_S,
                limits,
            )?
        } else {
            CompiledTrajectory::compile(
                motion.name.clone(),
                start,
                start_velocity,
                waypoints,
                motion.style,
                STS3215_MAX_SPEED_RAD_S,
            )?
        };
        // wrap the compiled trajectory in a MotionSequence and return it
        Ok(Self { trajectory })
    }

    /// samples the trajectory at the given elapsed time, returning the joint positions expected at that time
    /// Answers the question: "What joint angles should Orion be commanded to at this elapsed time?"
    /// The trajectory finds the relevant segment and evaluates its joint curves.
    /// During a hold, it returns the held target. After the sequence ends, it returns the final target.
    /// used seconds elapsed since the sequence began, rather than an absolute clock time.
    pub fn sample(&self, elapsed_seconds: f64) -> Result<JointPositions> {
        self.trajectory.sample(elapsed_seconds)
    }
    /// samples the trajectory at the given elapsed time, returning the joint positions, velocities, and accelerations
    /// use seconds elapsed since the sequence began, rather than an absolute clock time.
    pub fn sample_state(&self, elapsed_seconds: f64) -> Result<TrajectorySample> {
        self.trajectory.sample_state(elapsed_seconds)
    }

    /// returns the progress of the trajectory at the given elapsed time, as a value between 0 and 1
    /// it returns elapsed time / compiled duration clamped to 0.0..=1.0.
    pub fn progress(&self, elapsed_seconds: f64) -> Result<f64> {
        self.trajectory.progress(elapsed_seconds)
    }
    /// Returns whether progress has reached 1.0.
    /// This means the planned timeline has finished. It does not establish that the physical servos have reached the target.
    /// The runtime daemon enters a separate settling phase afterward and checks physical state.
    pub fn complete(&self, elapsed_seconds: f64) -> Result<bool> {
        self.trajectory.complete(elapsed_seconds)
    }

    /// Returns the motion’s names
    pub fn name(&self) -> &str {
        self.trajectory.name()
    }

    /// Returns the destination label of the segment (keyframe) active at time t, including its hold. Useful for status reporting.
    /// Note: during complidation each keyframe produces one segment
    pub fn keyframe_name(&self, elapsed_seconds: f64) -> Result<&str> {
        self.trajectory.keyframe_name(elapsed_seconds)
    }

    /// Returns that segment's (keyframe) zero-based keyframe index. This identifies which stage Orion is executing.
    pub fn keyframe_index(&self, elapsed_seconds: f64) -> Result<usize> {
        self.trajectory.keyframe_index(elapsed_seconds)
    }

    /// Returns the number of compiled keyframe segments.
    pub fn keyframe_count(&self) -> usize {
        self.trajectory.keyframe_count()
    }

    ///Returns the compiled arrival time, measured from sequence start. Excludes that keyframe’s subsequent hold; returns None for an invalid index.
    pub fn keyframe_arrival_time(&self, index: usize) -> Option<f64> {
        self.trajectory.keyframe_arrival_time(index)
    }

    /// Returns total compiled travel and hold time, including timing adjustments.
    pub fn duration_seconds(&self) -> f64 {
        self.trajectory.duration_seconds()
    }

    /// Returns the compiled arrival time attached to that marker, or None if absent.
    pub fn marker_time(&self, marker: &str) -> Option<f64> {
        self.trajectory.marker_time(marker)
    }

    /// Returns all markers whose arrival times have passed by t. It is cumulative, so repeated calls can return the same markers.
    pub fn reached_markers(&self, elapsed_seconds: f64) -> Vec<String> {
        self.trajectory.reached_markers(elapsed_seconds)
    }

    /// Returns the compiler’s sampled estimate of the largest absolute joint speed across the trajectory. It is not measured motor speed.
    pub fn peak_velocity_rad_s(&self) -> f64 {
        self.trajectory.peak_velocity_rad_s()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ORION_JOINT_NAMES;
    use crate::motion::calibration::load_calibration_file;
    use crate::motion::shared::fixture_path;

    #[test]
    fn loads_v2_absolute_and_relative_catalog() {
        let root = env!("CARGO_MANIFEST_DIR");
        let poses = PoseLibrary::load(
            format!("{root}/../motion/config/poses.yaml"),
            &ORION_JOINT_NAMES,
        )
        .unwrap();
        let motions = MotionLibrary::load(format!("{root}/../motion/motions"), &poses).unwrap();
        let turn = motions.motion("look_at_right_expressive").unwrap();
        assert_eq!(turn.space, MotionSpace::Absolute);
        assert_eq!(turn.keyframes[0].arrival, KeyframeArrival::Through);
        assert_eq!(
            turn.keyframes.last().unwrap().arrival,
            KeyframeArrival::Settle
        );
        let idle = motions.motion("idle_breathe").unwrap();
        assert_eq!(idle.space, MotionSpace::AnchorRelative);
        assert!(idle.return_to_anchor);
    }

    #[test]
    fn rejects_v1_with_clear_migration_error() {
        let root = tempfile::tempdir().unwrap();
        fs::copy(
            fixture_path("unsupported_version.yaml", "motions"),
            root.path().join("old.yaml"),
        )
        .unwrap();
        let poses = PoseLibrary::load(
            format!("{}/../motion/config/poses.yaml", env!("CARGO_MANIFEST_DIR")),
            &ORION_JOINT_NAMES,
        )
        .unwrap();
        assert!(
            MotionLibrary::load(root.path(), &poses)
                .unwrap_err()
                .to_string()
                .contains("v2 required")
        );
    }

    #[test]
    fn rejects_unknown_fields_and_nonreturning_relative_motion() {
        let root = tempfile::tempdir().unwrap();
        let poses = PoseLibrary::load(
            format!("{}/../motion/config/poses.yaml", env!("CARGO_MANIFEST_DIR")),
            &ORION_JOINT_NAMES,
        )
        .unwrap();
        fs::copy(
            fixture_path("unknown_field.yaml", "motions"),
            root.path().join("unknown.yaml"),
        )
        .unwrap();
        assert!(
            MotionLibrary::load(root.path(), &poses)
                .unwrap_err()
                .to_string()
                .contains("unknown field")
        );

        fs::remove_file(root.path().join("unknown.yaml")).unwrap();
        fs::copy(
            fixture_path("nonreturning_relative.yaml", "motions"),
            root.path().join("drifting.yaml"),
        )
        .unwrap();
        assert!(
            MotionLibrary::load(root.path(), &poses)
                .unwrap_err()
                .to_string()
                .contains("return_to_anchor: true")
        );
    }

    #[test]
    fn expressive_turns_flow_through_every_internal_drawing_without_a_stop_plateau() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let poses =
            PoseLibrary::load(root.join("motion/config/poses.yaml"), &ORION_JOINT_NAMES).unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        for name in [
            "look_at_left_expressive",
            "look_at_right_expressive",
            "attention_left",
            "attention_right",
        ] {
            let definition = motions.motion(name).unwrap();
            let start = poses
                .pose(if name.starts_with("attention_") {
                    "home"
                } else {
                    "attentive"
                })
                .unwrap()
                .clone();
            let sequence = MotionSequence::new(definition, start).unwrap();
            for index in 0..sequence.keyframe_count() - 1 {
                let arrival = sequence.keyframe_arrival_time(index).unwrap();
                let before = sequence.sample_state((arrival - 0.002).max(0.0)).unwrap();
                let after = sequence.sample_state(arrival + 0.002).unwrap();
                let before_speed: f64 = before.velocities.values().map(|value| value.abs()).sum();
                let after_speed: f64 = after.velocities.values().map(|value| value.abs()).sum();
                assert!(
                    before_speed > 0.01,
                    "{name} stopped before keyframe {index}"
                );
                assert!(after_speed > 0.01, "{name} stopped after keyframe {index}");
            }
            assert!(sequence.peak_velocity_rad_s() <= STS3215_MAX_SPEED_RAD_S * 1.001);
        }
    }

    #[test]
    fn every_relative_character_clip_uniformly_scales_and_returns_to_each_anchor() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let poses =
            PoseLibrary::load(root.join("motion/config/poses.yaml"), &ORION_JOINT_NAMES).unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        let calibration = load_calibration_file(
            root.join("simulation/mujoco/config/servo_calibration.json"),
            &ORION_JOINT_NAMES,
        )
        .unwrap();
        let limits: Vec<JointLimit> = calibration
            .iter()
            .map(|joint| {
                let (lower_rad, upper_rad) = joint.safe_range_radians();
                JointLimit {
                    name: joint.name.clone(),
                    lower_rad,
                    upper_rad,
                }
            })
            .collect();
        let ranges: BTreeMap<String, (f64, f64)> = limits
            .iter()
            .map(|limit| (limit.name.clone(), (limit.lower_rad, limit.upper_rad)))
            .collect();
        let anchors = ["home", "attentive", "look_left", "look_right"];
        for (_, definition) in motions
            .iter()
            .filter(|(_, motion)| motion.space == MotionSpace::AnchorRelative)
        {
            for anchor_name in anchors {
                let anchor = poses.pose(anchor_name).unwrap().clone();
                let scale = definition
                    .uniform_amplitude_scale(&anchor, &limits)
                    .unwrap();
                let zero_velocity = anchor.keys().map(|joint| (joint.clone(), 0.0)).collect();
                let sequence = MotionSequence::compile_scaled(
                    definition,
                    anchor.clone(),
                    zero_velocity,
                    anchor.clone(),
                    scale,
                )
                .unwrap();
                for sample in 0..=100 {
                    let positions = sequence
                        .sample(sequence.duration_seconds() * sample as f64 / 100.0)
                        .unwrap();
                    for (joint, value) in positions {
                        let (lower, upper) = ranges[&joint];
                        assert!((lower - 1e-9..=upper + 1e-9).contains(&value));
                    }
                }
                let end = sequence.sample(sequence.duration_seconds()).unwrap();
                for joint in ORION_JOINT_NAMES {
                    assert!((end[joint] - anchor[joint]).abs() < 1e-9);
                }
            }
        }
    }

    #[test]
    fn every_built_in_pose_and_motion_sample_stays_inside_calibration() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let poses =
            PoseLibrary::load(root.join("motion/config/poses.yaml"), &ORION_JOINT_NAMES).unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        let calibration = load_calibration_file(
            root.join("simulation/mujoco/config/servo_calibration.json"),
            &ORION_JOINT_NAMES,
        )
        .unwrap();
        let limits: Vec<JointLimit> = calibration
            .iter()
            .map(|joint| {
                let (lower_rad, upper_rad) = joint.safe_range_radians();
                JointLimit {
                    name: joint.name.clone(),
                    lower_rad,
                    upper_rad,
                }
            })
            .collect();
        let ranges: BTreeMap<String, (f64, f64)> = limits
            .iter()
            .map(|limit| (limit.name.clone(), (limit.lower_rad, limit.upper_rad)))
            .collect();

        let assert_calibrated = |label: &str, positions: &JointPositions| {
            for joint in ORION_JOINT_NAMES {
                let value = positions[joint];
                let (lower, upper) = ranges[joint];
                assert!(
                    (lower - 1e-9..=upper + 1e-9).contains(&value),
                    "{label} places {joint} at {value:.6}, outside [{lower:.6}, {upper:.6}]"
                );
            }
        };

        for (pose_name, positions) in poses.iter() {
            assert_calibrated(&format!("pose '{pose_name}'"), positions);
        }

        let anchors = ["home", "attentive", "look_left", "look_right"];
        for (motion_name, definition) in motions.iter() {
            for anchor_name in anchors {
                let anchor = poses.pose(anchor_name).unwrap().clone();
                let scale = definition
                    .uniform_amplitude_scale(&anchor, &limits)
                    .unwrap();
                let start_velocity = anchor.keys().map(|joint| (joint.clone(), 0.0)).collect();
                let sequence = MotionSequence::compile_scaled(
                    definition,
                    anchor.clone(),
                    start_velocity,
                    anchor,
                    scale,
                )
                .unwrap();
                assert!(
                    sequence.peak_velocity_rad_s() <= STS3215_MAX_SPEED_RAD_S * 1.001,
                    "motion '{motion_name}' from '{anchor_name}' exceeded the STS3215 ceiling"
                );

                const RUNTIME_CONTROL_RATE_HZ: f64 = 50.0;
                let sample_count =
                    (sequence.duration_seconds() * RUNTIME_CONTROL_RATE_HZ).ceil() as usize;
                for sample_index in 0..=sample_count {
                    let time = (sample_index as f64 / RUNTIME_CONTROL_RATE_HZ)
                        .min(sequence.duration_seconds());
                    let positions = sequence.sample(time).unwrap();
                    assert_calibrated(
                        &format!("motion '{motion_name}' from '{anchor_name}' at {time:.3}s"),
                        &positions,
                    );
                }
            }
        }
    }
}
