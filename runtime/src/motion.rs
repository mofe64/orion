use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::driver::JointLimit;
use crate::pose::{JointPositions, PoseLibrary};
use crate::style::MotionStyle;
use crate::trajectory::{
    CompiledTrajectory, STS3215_MAX_SPEED_RAD_S, TrajectoryWaypoint, WaypointArrival,
};
use crate::{Error, Result};

pub const MOTION_FORMAT_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionSpace {
    Absolute,
    AnchorRelative,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyframeArrival {
    Through,
    Settle,
}

impl From<KeyframeArrival> for WaypointArrival {
    fn from(value: KeyframeArrival) -> Self {
        match value {
            KeyframeArrival::Through => Self::Through,
            KeyframeArrival::Settle => Self::Settle,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MotionKeyframe {
    pub pose_name: Option<String>,
    /// Absolute positions for absolute motion; offsets for anchor-relative motion.
    pub target: JointPositions,
    pub duration_seconds: f64,
    pub arrival: KeyframeArrival,
    pub hold_seconds: f64,
    pub marker: Option<String>,
}

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
    pub fn markers(&self) -> Vec<String> {
        self.keyframes
            .iter()
            .filter_map(|keyframe| keyframe.marker.clone())
            .collect()
    }

    pub fn resolved_targets(&self, anchor: &JointPositions) -> Result<Vec<JointPositions>> {
        self.resolved_targets_with_scale(anchor, 1.0)
    }

    pub fn uniform_amplitude_scale(
        &self,
        anchor: &JointPositions,
        limits: &[JointLimit],
    ) -> Result<f64> {
        if self.space == MotionSpace::Absolute {
            return Ok(1.0);
        }
        let by_name: BTreeMap<&str, &JointLimit> = limits
            .iter()
            .map(|limit| (limit.name.as_str(), limit))
            .collect();
        let mut scale: f64 = 1.0;
        for keyframe in &self.keyframes {
            for (joint, offset) in &keyframe.target {
                let anchor_value = *anchor.get(joint).ok_or_else(|| {
                    Error::InvalidArgument(format!("Idle anchor omits Orion joint '{joint}'."))
                })?;
                let limit = by_name.get(joint.as_str()).ok_or_else(|| {
                    Error::InvalidArgument(format!("Calibration omits Orion joint '{joint}'."))
                })?;
                let styled_offset = offset * self.style.amplitude;
                if styled_offset > 0.0 {
                    scale = scale.min((limit.upper_rad - anchor_value) / styled_offset);
                } else if styled_offset < 0.0 {
                    scale = scale.min((limit.lower_rad - anchor_value) / styled_offset);
                }
            }
        }
        Ok(scale.clamp(0.0, 1.0))
    }

    pub(crate) fn resolved_targets_with_scale(
        &self,
        anchor: &JointPositions,
        scale: f64,
    ) -> Result<Vec<JointPositions>> {
        match self.space {
            MotionSpace::Absolute => Ok(self
                .keyframes
                .iter()
                .map(|keyframe| keyframe.target.clone())
                .collect()),
            MotionSpace::AnchorRelative => self
                .keyframes
                .iter()
                .map(|keyframe| {
                    let mut target = anchor.clone();
                    for (joint, offset) in &keyframe.target {
                        let value = target.get_mut(joint).ok_or_else(|| {
                            Error::InvalidArgument(format!(
                                "Relative motion '{}' contains unknown joint '{joint}'.",
                                self.name
                            ))
                        })?;
                        *value += offset * self.style.amplitude * scale;
                    }
                    Ok(target)
                })
                .collect(),
        }
    }

    pub fn final_target(&self, anchor: &JointPositions) -> Result<JointPositions> {
        self.resolved_targets(anchor)?
            .pop()
            .ok_or_else(|| Error::InvalidArgument("Motion has no final keyframe.".into()))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MotionDocument {
    #[serde(default)]
    format_version: u32,
    motion: Option<MotionEntry>,
}

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

#[derive(Clone, Debug)]
pub struct MotionLibrary {
    motions: BTreeMap<String, MotionDefinition>,
}

impl MotionLibrary {
    pub fn load(directory: impl AsRef<Path>, poses: &PoseLibrary) -> Result<Self> {
        let directory = directory.as_ref();
        if !directory.is_dir() {
            return Err(Error::Runtime(format!(
                "Motion library is not a directory: {}",
                directory.display()
            )));
        }
        let mut files = Vec::new();
        collect_yaml_files(directory, &mut files)?;
        files.sort();
        if files.is_empty() {
            return Err(Error::Runtime(format!(
                "Motion library contains no YAML files: {}",
                directory.display()
            )));
        }
        let mut motions = BTreeMap::new();
        for path in files {
            let motion = load_motion_file(&path, poses)?;
            if motions.insert(motion.name.clone(), motion).is_some() {
                return Err(Error::Runtime(format!(
                    "Duplicate Orion motion name in {}",
                    path.display()
                )));
            }
        }
        Ok(Self { motions })
    }

    pub fn motion(&self, name: &str) -> Result<&MotionDefinition> {
        self.motions
            .get(name)
            .ok_or_else(|| Error::InvalidArgument(format!("Unknown Orion motion: {name}")))
    }
    pub fn names(&self) -> Vec<String> {
        self.motions.keys().cloned().collect()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&String, &MotionDefinition)> {
        self.motions.iter()
    }
}

fn collect_yaml_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory).map_err(|error| {
        Error::Runtime(format!(
            "Could not read motion library '{}': {error}",
            directory.display()
        ))
    })? {
        let path = entry?.path();
        if path.is_dir() {
            collect_yaml_files(&path, files)?;
        } else if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yaml" | "yml")
        ) {
            files.push(path);
        }
    }
    Ok(())
}

fn load_motion_file(path: &Path, poses: &PoseLibrary) -> Result<MotionDefinition> {
    let contents = fs::read_to_string(path).map_err(|error| {
        Error::Runtime(format!(
            "Could not read motion file '{}': {error}",
            path.display()
        ))
    })?;
    let header: serde_yaml::Value = serde_yaml::from_str(&contents).map_err(|error| {
        Error::Runtime(format!(
            "Could not parse motion file '{}': {error}",
            path.display()
        ))
    })?;
    if header
        .get("format_version")
        .and_then(serde_yaml::Value::as_u64)
        != Some(MOTION_FORMAT_VERSION as u64)
    {
        return Err(Error::Runtime(
            "Motion file must use format_version 2 (v2 required).".into(),
        ));
    }
    let document: MotionDocument = serde_yaml::from_str(&contents).map_err(|error| {
        Error::Runtime(format!(
            "Could not parse motion file '{}': {error}",
            path.display()
        ))
    })?;
    if document.format_version != MOTION_FORMAT_VERSION {
        return Err(Error::Runtime(
            "Motion file must use format_version 2 (v2 required).".into(),
        ));
    }
    let entry = document
        .motion
        .ok_or_else(|| Error::Runtime("Motion file must contain a motion mapping.".into()))?;
    if !is_semantic_name(&entry.name) || entry.keyframes.is_empty() {
        return Err(Error::Runtime(
            "Motion requires a semantic name and at least one keyframe.".into(),
        ));
    }
    let style = MotionStyle::named(&entry.style).map_err(|error| {
        Error::Runtime(format!(
            "Motion '{}' has invalid style: {error}",
            entry.name
        ))
    })?;
    if entry.space == MotionSpace::AnchorRelative && !entry.return_to_anchor {
        return Err(Error::Runtime(format!(
            "Anchor-relative motion '{}' must set return_to_anchor: true.",
            entry.name
        )));
    }
    if entry.space == MotionSpace::Absolute && entry.return_to_anchor {
        return Err(Error::Runtime(format!(
            "Absolute motion '{}' cannot set return_to_anchor.",
            entry.name
        )));
    }
    let mut markers = BTreeSet::new();
    let mut keyframes = Vec::with_capacity(entry.keyframes.len());
    for keyframe in entry.keyframes {
        if !keyframe.duration.is_finite()
            || keyframe.duration <= 0.0
            || !keyframe.hold.is_finite()
            || keyframe.hold < 0.0
        {
            return Err(Error::Runtime(format!(
                "Motion '{}' keyframes require positive duration and non-negative hold.",
                entry.name
            )));
        }
        if keyframe.arrival == KeyframeArrival::Through && keyframe.hold > 0.0 {
            return Err(Error::Runtime(format!(
                "Motion '{}' cannot hold a through keyframe.",
                entry.name
            )));
        }
        if keyframe
            .marker
            .as_deref()
            .is_some_and(|marker| !is_semantic_name(marker) || !markers.insert(marker.to_owned()))
        {
            return Err(Error::Runtime(format!(
                "Motion '{}' markers must be unique semantic names.",
                entry.name
            )));
        }
        let (pose_name, target) = match entry.space {
            MotionSpace::Absolute => {
                if keyframe.offsets.is_some() {
                    return Err(Error::Runtime(format!(
                        "Absolute motion '{}' keyframes use pose, not offsets.",
                        entry.name
                    )));
                }
                let pose = keyframe
                    .pose
                    .filter(|pose| !pose.is_empty())
                    .ok_or_else(|| {
                        Error::Runtime(format!(
                            "Absolute motion '{}' keyframe requires a pose.",
                            entry.name
                        ))
                    })?;
                let target = poses
                    .pose(&pose)
                    .map_err(|error| {
                        Error::Runtime(format!(
                            "Invalid pose reference in '{}': {error}",
                            path.display()
                        ))
                    })?
                    .clone();
                (Some(pose), target)
            }
            MotionSpace::AnchorRelative => {
                if keyframe.pose.is_some() {
                    return Err(Error::Runtime(format!(
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
                    return Err(Error::Runtime(format!(
                        "Anchor-relative motion '{}' contains an invalid joint offset.",
                        entry.name
                    )));
                }
                (None, offsets)
            }
        };
        keyframes.push(MotionKeyframe {
            pose_name,
            target,
            duration_seconds: keyframe.duration,
            arrival: keyframe.arrival,
            hold_seconds: keyframe.hold,
            marker: keyframe.marker,
        });
    }
    if keyframes
        .last()
        .is_some_and(|keyframe| keyframe.arrival != KeyframeArrival::Settle)
    {
        return Err(Error::Runtime(format!(
            "Motion '{}' final keyframe must settle.",
            entry.name
        )));
    }
    if entry.space == MotionSpace::AnchorRelative
        && keyframes
            .last()
            .is_some_and(|keyframe| keyframe.target.values().any(|offset| offset.abs() > 1e-12))
    {
        return Err(Error::Runtime(format!(
            "Anchor-relative motion '{}' must finish at zero offsets.",
            entry.name
        )));
    }
    Ok(MotionDefinition {
        name: entry.name,
        description: entry.description,
        space: entry.space,
        style,
        return_to_anchor: entry.return_to_anchor,
        keyframes,
    })
}

fn is_semantic_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[derive(Clone, Debug)]
pub struct MotionSequence {
    trajectory: CompiledTrajectory,
}

impl MotionSequence {
    pub fn new(motion: &MotionDefinition, start: JointPositions) -> Result<Self> {
        let velocity = start.keys().map(|joint| (joint.clone(), 0.0)).collect();
        Self::compile(motion, start.clone(), velocity, start)
    }

    pub fn compile(
        motion: &MotionDefinition,
        start: JointPositions,
        start_velocity: JointPositions,
        anchor: JointPositions,
    ) -> Result<Self> {
        Self::compile_scaled(motion, start, start_velocity, anchor, 1.0)
    }

    pub fn compile_scaled(
        motion: &MotionDefinition,
        start: JointPositions,
        start_velocity: JointPositions,
        anchor: JointPositions,
        amplitude_scale: f64,
    ) -> Result<Self> {
        Self::compile_scaled_inner(motion, start, start_velocity, anchor, amplitude_scale, None)
    }

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

    fn compile_scaled_inner(
        motion: &MotionDefinition,
        start: JointPositions,
        start_velocity: JointPositions,
        anchor: JointPositions,
        amplitude_scale: f64,
        limits: Option<&[JointLimit]>,
    ) -> Result<Self> {
        if !amplitude_scale.is_finite() || !(0.0..=1.0).contains(&amplitude_scale) {
            return Err(Error::InvalidArgument(
                "Motion amplitude scale must be between zero and one.".into(),
            ));
        }
        let targets = motion.resolved_targets_with_scale(&anchor, amplitude_scale)?;
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
        Ok(Self { trajectory })
    }

    pub fn sample(&self, elapsed_seconds: f64) -> Result<JointPositions> {
        self.trajectory.sample(elapsed_seconds)
    }
    pub fn sample_state(
        &self,
        elapsed_seconds: f64,
    ) -> Result<crate::trajectory::TrajectorySample> {
        self.trajectory.sample_state(elapsed_seconds)
    }
    pub fn progress(&self, elapsed_seconds: f64) -> Result<f64> {
        self.trajectory.progress(elapsed_seconds)
    }
    pub fn complete(&self, elapsed_seconds: f64) -> Result<bool> {
        self.trajectory.complete(elapsed_seconds)
    }
    pub fn name(&self) -> &str {
        self.trajectory.name()
    }
    pub fn keyframe_name(&self, elapsed_seconds: f64) -> Result<&str> {
        self.trajectory.keyframe_name(elapsed_seconds)
    }
    pub fn keyframe_index(&self, elapsed_seconds: f64) -> Result<usize> {
        self.trajectory.keyframe_index(elapsed_seconds)
    }
    pub fn keyframe_count(&self) -> usize {
        self.trajectory.keyframe_count()
    }
    pub fn keyframe_arrival_time(&self, index: usize) -> Option<f64> {
        self.trajectory.keyframe_arrival_time(index)
    }
    pub fn duration_seconds(&self) -> f64 {
        self.trajectory.duration_seconds()
    }
    pub fn marker_time(&self, marker: &str) -> Option<f64> {
        self.trajectory.marker_time(marker)
    }
    pub fn reached_markers(&self, elapsed_seconds: f64) -> Vec<String> {
        self.trajectory.reached_markers(elapsed_seconds)
    }
    pub fn peak_velocity_rad_s(&self) -> f64 {
        self.trajectory.peak_velocity_rad_s()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ORION_JOINT_NAMES;

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
        fs::write(
            root.path().join("old.yaml"),
            "format_version: 1\nmotion: {name: old, keyframes: []}\n",
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
        fs::write(
            root.path().join("unknown.yaml"),
            r#"format_version: 2
motion:
  name: unknown
  description: Rejected legacy field.
  space: absolute
  style: expressive_turn
  return_to_anchor: false
  interpolation: linear
  keyframes:
    - {pose: home, duration: 0.5, arrival: settle}
"#,
        )
        .unwrap();
        assert!(
            MotionLibrary::load(root.path(), &poses)
                .unwrap_err()
                .to_string()
                .contains("unknown field")
        );

        fs::remove_file(root.path().join("unknown.yaml")).unwrap();
        fs::write(
            root.path().join("drifting.yaml"),
            r#"format_version: 2
motion:
  name: drifting
  description: Relative clips must preserve their anchor.
  space: anchor_relative
  style: living_idle
  return_to_anchor: false
  keyframes:
    - duration: 0.5
      arrival: settle
      offsets: {head_pitch_joint: 0.1}
"#,
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
        let calibration = crate::calibration::load_calibration_file(
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
        let calibration = crate::calibration::load_calibration_file(
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
