use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::pose::{JointPositions, PoseLibrary};
use crate::trajectory::JointTrajectory;
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct MotionKeyframe {
    pub pose_name: String,
    pub target: JointPositions,
    pub duration_seconds: f64,
    pub hold_seconds: f64,
}

#[derive(Clone, Debug)]
pub struct MotionDefinition {
    pub name: String,
    pub description: String,
    pub keyframes: Vec<MotionKeyframe>,
}

#[derive(Debug, Deserialize)]
struct MotionDocument {
    #[serde(default)]
    format_version: u32,
    motion: Option<MotionEntry>,
}

#[derive(Debug, Deserialize)]
struct MotionEntry {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    keyframes: Vec<KeyframeEntry>,
}

#[derive(Debug, Deserialize)]
struct KeyframeEntry {
    #[serde(default)]
    pose: String,
    duration: f64,
    #[serde(default)]
    hold: f64,
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
            "Could not parse motion file '{}': {error}",
            path.display()
        ))
    })?;
    let document: MotionDocument = serde_yaml::from_str(&contents).map_err(|error| {
        Error::Runtime(format!(
            "Could not parse motion file '{}': {error}",
            path.display()
        ))
    })?;
    if document.format_version != 1 {
        return Err(Error::Runtime(
            "Motion file must use format_version 1.".into(),
        ));
    }
    let Some(entry) = document.motion else {
        return Err(Error::Runtime(
            "Motion file must contain a motion mapping.".into(),
        ));
    };
    if entry.name.is_empty()
        || !entry
            .name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(Error::Runtime(
            "Motion name must be a semantic Orion name.".into(),
        ));
    }
    if entry.keyframes.is_empty() {
        return Err(Error::Runtime(format!(
            "Motion '{}' must contain keyframes.",
            entry.name
        )));
    }

    let mut keyframes = Vec::new();
    for keyframe in entry.keyframes {
        if keyframe.pose.is_empty()
            || !keyframe.duration.is_finite()
            || keyframe.duration <= 0.0
            || !keyframe.hold.is_finite()
            || keyframe.hold < 0.0
        {
            return Err(Error::Runtime(format!(
                "Motion '{}' keyframes require a pose, positive duration, and non-negative hold.",
                entry.name
            )));
        }
        let target = poses.pose(&keyframe.pose).map_err(|error| {
            Error::Runtime(format!(
                "Invalid pose reference in motion file '{}': {error}",
                path.display()
            ))
        })?;
        keyframes.push(MotionKeyframe {
            pose_name: keyframe.pose,
            target: target.clone(),
            duration_seconds: keyframe.duration,
            hold_seconds: keyframe.hold,
        });
    }
    Ok(MotionDefinition {
        name: entry.name,
        description: entry.description,
        keyframes,
    })
}

#[derive(Clone, Debug)]
struct Segment {
    pose_name: String,
    transition: JointTrajectory,
    target: JointPositions,
    starts_at: f64,
    arrives_at: f64,
    holds_until: f64,
}

#[derive(Clone, Debug)]
pub struct MotionSequence {
    name: String,
    segments: Vec<Segment>,
    duration_seconds: f64,
}

impl MotionSequence {
    pub fn new(motion: &MotionDefinition, start: JointPositions) -> Result<Self> {
        if motion.name.is_empty() || motion.keyframes.is_empty() || start.is_empty() {
            return Err(Error::InvalidArgument(
                "Motion sequence requires a name, start, and keyframes.".into(),
            ));
        }
        let mut segments = Vec::new();
        let mut segment_start = start;
        let mut starts_at = 0.0;
        for keyframe in &motion.keyframes {
            let arrives_at = starts_at + keyframe.duration_seconds;
            let holds_until = arrives_at + keyframe.hold_seconds;
            segments.push(Segment {
                pose_name: keyframe.pose_name.clone(),
                transition: JointTrajectory::new(
                    format!("{}:{}", motion.name, keyframe.pose_name),
                    segment_start,
                    keyframe.target.clone(),
                    keyframe.duration_seconds,
                )?,
                target: keyframe.target.clone(),
                starts_at,
                arrives_at,
                holds_until,
            });
            segment_start = keyframe.target.clone();
            starts_at = holds_until;
        }
        Ok(Self {
            name: motion.name.clone(),
            segments,
            duration_seconds: starts_at,
        })
    }

    pub fn sample(&self, elapsed_seconds: f64) -> Result<JointPositions> {
        let segment = &self.segments[self.segment_index(elapsed_seconds)?];
        if elapsed_seconds < segment.arrives_at {
            segment
                .transition
                .sample(elapsed_seconds - segment.starts_at)
        } else {
            Ok(segment.target.clone())
        }
    }

    pub fn progress(&self, elapsed_seconds: f64) -> Result<f64> {
        if !elapsed_seconds.is_finite() {
            return Err(Error::InvalidArgument(
                "Motion elapsed time must be finite.".into(),
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

    pub fn keyframe_name(&self, elapsed_seconds: f64) -> Result<&str> {
        Ok(&self.segments[self.segment_index(elapsed_seconds)?].pose_name)
    }

    pub fn keyframe_index(&self, elapsed_seconds: f64) -> Result<usize> {
        self.segment_index(elapsed_seconds)
    }

    pub fn keyframe_count(&self) -> usize {
        self.segments.len()
    }

    pub fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }

    fn segment_index(&self, elapsed_seconds: f64) -> Result<usize> {
        if !elapsed_seconds.is_finite() {
            return Err(Error::InvalidArgument(
                "Motion elapsed time must be finite.".into(),
            ));
        }
        let clamped = elapsed_seconds.max(0.0);
        self.segments
            .iter()
            .enumerate()
            .find_map(|(index, segment)| {
                (clamped < segment.holds_until || index + 1 == self.segments.len()).then_some(index)
            })
            .ok_or_else(|| Error::InvalidState("Motion sequence contains no segment.".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ORION_JOINT_NAMES;

    fn positions(values: &[(&str, f64)]) -> JointPositions {
        values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value))
            .collect()
    }

    #[test]
    fn loads_nested_functional_and_expressive_motions() {
        let root = env!("CARGO_MANIFEST_DIR");
        let poses = PoseLibrary::load(
            format!("{root}/../motion/config/poses.yaml"),
            &ORION_JOINT_NAMES,
        )
        .unwrap();
        let motions = MotionLibrary::load(format!("{root}/../motion/motions"), &poses).unwrap();

        assert_eq!(motions.names().len(), 5);
        let right = motions.motion("look_at_right_expressive").unwrap();
        assert_eq!(right.keyframes.len(), 4);
        assert_eq!(right.keyframes[0].pose_name, "look_right_anticipation");
        assert_eq!(right.keyframes[0].duration_seconds, 2.0);
        assert_eq!(right.keyframes[3].pose_name, "look_right");
        assert!(motions.motion("missing").is_err());
    }

    #[test]
    fn samples_transitions_and_authored_holds() {
        let motion = MotionDefinition {
            name: "example".into(),
            description: String::new(),
            keyframes: vec![
                MotionKeyframe {
                    pose_name: "first".into(),
                    target: positions(&[("joint", 1.0)]),
                    duration_seconds: 2.0,
                    hold_seconds: 1.0,
                },
                MotionKeyframe {
                    pose_name: "second".into(),
                    target: positions(&[("joint", -1.0)]),
                    duration_seconds: 2.0,
                    hold_seconds: 0.5,
                },
            ],
        };
        let sequence = MotionSequence::new(&motion, positions(&[("joint", 0.0)])).unwrap();

        assert_eq!(sequence.sample(0.0).unwrap()["joint"], 0.0);
        assert_eq!(sequence.sample(1.0).unwrap()["joint"], 0.5);
        assert_eq!(sequence.sample(2.5).unwrap()["joint"], 1.0);
        assert_eq!(sequence.sample(4.0).unwrap()["joint"], 0.0);
        assert_eq!(sequence.sample(5.5).unwrap()["joint"], -1.0);
        assert_eq!(sequence.keyframe_name(2.5).unwrap(), "first");
        assert_eq!(sequence.keyframe_name(3.0).unwrap(), "second");
        assert_eq!(sequence.keyframe_index(3.0).unwrap(), 1);
        assert_eq!(sequence.keyframe_count(), 2);
        assert_eq!(sequence.duration_seconds(), 5.5);
        assert!(sequence.complete(5.5).unwrap());
    }
}
