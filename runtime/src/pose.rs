use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::lighting::LIGHTING_EFFECT_NAMES;
use crate::{Error, Result};

pub type JointPositions = BTreeMap<String, f64>;

pub const POSE_FORMAT_VERSION: u32 = 2;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PoseDocument {
    #[serde(default)]
    format_version: u32,
    #[serde(default)]
    units: Option<String>,
    #[serde(default)]
    poses: BTreeMap<String, PoseEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PoseEntry {
    #[serde(default)]
    description: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    idle_profile: Option<String>,
    #[serde(default)]
    default_lighting: Option<String>,
    #[serde(default)]
    positions: JointPositions,
}

#[derive(Clone, Debug)]
pub struct PoseDefinition {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub idle_profile: Option<String>,
    pub default_lighting: Option<String>,
    pub positions: JointPositions,
}

#[derive(Clone, Debug)]
pub struct PoseLibrary {
    poses: BTreeMap<String, PoseDefinition>,
}

impl PoseLibrary {
    pub fn load(path: impl AsRef<Path>, joint_names: &[impl AsRef<str>]) -> Result<Self> {
        Self::load_files(vec![path.as_ref().to_path_buf()], joint_names)
    }

    pub fn load_with_user_directory(
        built_in_path: impl AsRef<Path>,
        user_directory: impl AsRef<Path>,
        joint_names: &[impl AsRef<str>],
    ) -> Result<Self> {
        let mut files = vec![built_in_path.as_ref().to_path_buf()];
        let user_directory = user_directory.as_ref();
        if user_directory.exists() {
            collect_yaml_files(user_directory, &mut files)?;
        }
        files.sort();
        Self::load_files(files, joint_names)
    }

    fn load_files(files: Vec<PathBuf>, joint_names: &[impl AsRef<str>]) -> Result<Self> {
        let expected: BTreeSet<String> = joint_names
            .iter()
            .map(|name| name.as_ref().to_owned())
            .collect();
        let mut poses = BTreeMap::new();
        for path in files {
            load_pose_file(&path, &expected, &mut poses)?;
        }
        if poses.is_empty() {
            return Err(Error::Runtime("Pose library contains no poses.".into()));
        }
        Ok(Self { poses })
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &JointPositions)> {
        self.poses
            .iter()
            .map(|(name, pose)| (name, &pose.positions))
    }

    pub fn pose(&self, name: &str) -> Result<&JointPositions> {
        self.poses
            .get(name)
            .map(|pose| &pose.positions)
            .ok_or_else(|| Error::InvalidArgument(format!("Unknown Orion pose: {name}")))
    }

    pub fn definition(&self, name: &str) -> Result<&PoseDefinition> {
        self.poses
            .get(name)
            .ok_or_else(|| Error::InvalidArgument(format!("Unknown Orion pose: {name}")))
    }

    pub fn names(&self) -> Vec<String> {
        self.poses.keys().cloned().collect()
    }
}

fn collect_yaml_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory).map_err(|error| {
        Error::Runtime(format!(
            "Could not read user pose library '{}': {error}",
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

fn load_pose_file(
    path: &Path,
    expected: &BTreeSet<String>,
    poses: &mut BTreeMap<String, PoseDefinition>,
) -> Result<()> {
    let contents = fs::read_to_string(path).map_err(|error| {
        Error::Runtime(format!(
            "Could not parse pose library '{}': {error}",
            path.display()
        ))
    })?;
    let document: PoseDocument = serde_yaml::from_str(&contents).map_err(|error| {
        Error::Runtime(format!(
            "Could not parse pose library '{}': {error}",
            path.display()
        ))
    })?;

    if document.format_version != POSE_FORMAT_VERSION
        || document
            .units
            .as_deref()
            .is_some_and(|units| units != "radians")
    {
        return Err(Error::Runtime(
            "Pose library must use format_version 2 with radian positions (v2 required).".into(),
        ));
    }
    if document.poses.is_empty() {
        return Err(Error::Runtime("Pose library contains no poses.".into()));
    }

    for (pose_name, entry) in document.poses {
        if pose_name.is_empty()
            || !pose_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(Error::Runtime(format!(
                "Pose name '{pose_name}' is not a semantic Orion name."
            )));
        }
        let present: BTreeSet<String> = entry.positions.keys().cloned().collect();
        if &present != expected {
            return Err(Error::Runtime(format!(
                "Pose '{pose_name}' joint names do not match Orion."
            )));
        }
        if entry.positions.values().any(|value| !value.is_finite()) {
            return Err(Error::Runtime(format!(
                "Pose '{pose_name}' contains a non-finite target."
            )));
        }
        if entry.tags.iter().any(|tag| !is_semantic_name(tag))
            || entry
                .idle_profile
                .as_deref()
                .is_some_and(|profile| !is_semantic_name(profile))
            || entry.default_lighting.as_deref().is_some_and(|effect| {
                !is_semantic_name(effect) || !LIGHTING_EFFECT_NAMES.contains(&effect)
            })
        {
            return Err(Error::Runtime(format!(
                "Pose '{pose_name}' contains an invalid semantic tag, idle profile, or lighting effect."
            )));
        }
        let definition = PoseDefinition {
            name: pose_name.clone(),
            description: entry.description,
            tags: entry.tags,
            idle_profile: entry.idle_profile,
            default_lighting: entry.default_lighting,
            positions: entry.positions,
        };
        if poses.insert(pose_name.clone(), definition).is_some() {
            return Err(Error::Runtime(format!(
                "Duplicate Orion pose name '{pose_name}' in {}.",
                path.display()
            )));
        }
    }
    Ok(())
}

fn is_semantic_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ORION_JOINT_NAMES;

    #[test]
    fn loads_orion_named_poses() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../motion/config/poses.yaml");
        let poses = PoseLibrary::load(path, &ORION_JOINT_NAMES).unwrap();

        assert_eq!(poses.pose("rest").unwrap().len(), 5);
        assert_eq!(poses.pose("home").unwrap()["shoulder_pitch_joint"], 0.0);
        assert!(poses.pose("missing").is_err());
    }

    #[test]
    fn merges_user_pose_files_without_allowing_duplicate_names() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("user");
        fs::create_dir_all(&user).unwrap();
        fs::write(
            user.join("studio_keyframe.yaml"),
            r#"format_version: 2
units: radians
poses:
  studio_keyframe:
    description: Studio-authored keyframe.
    positions:
      base_yaw_joint: 0.1
      shoulder_pitch_joint: -0.1
      elbow_pitch_joint: 0.2
      head_roll_joint: 0.0
      head_pitch_joint: -0.2
"#,
        )
        .unwrap();
        let built_in = concat!(env!("CARGO_MANIFEST_DIR"), "/../motion/config/poses.yaml");
        let poses =
            PoseLibrary::load_with_user_directory(built_in, &user, &ORION_JOINT_NAMES).unwrap();
        assert!(poses.pose("studio_keyframe").is_ok());

        fs::write(
            user.join("duplicate.yaml"),
            r#"format_version: 2
units: radians
poses:
  home:
    positions:
      base_yaw_joint: 0.0
      shoulder_pitch_joint: 0.0
      elbow_pitch_joint: 0.0
      head_roll_joint: 0.0
      head_pitch_joint: 0.0
"#,
        )
        .unwrap();
        assert!(
            PoseLibrary::load_with_user_directory(built_in, &user, &ORION_JOINT_NAMES)
                .unwrap_err()
                .to_string()
                .contains("Duplicate Orion pose")
        );
    }

    #[test]
    fn rejects_v1_and_unknown_pose_fields() {
        let root = tempfile::tempdir().unwrap();
        let v1 = root.path().join("v1.yaml");
        fs::write(&v1, "format_version: 1\nposes: {}\n").unwrap();
        assert!(
            PoseLibrary::load(&v1, &ORION_JOINT_NAMES)
                .unwrap_err()
                .to_string()
                .contains("v2 required")
        );

        let unknown = root.path().join("unknown.yaml");
        fs::write(
            &unknown,
            r#"format_version: 2
units: radians
legacy_pose_map: true
poses:
  home:
    positions:
      base_yaw_joint: 0.0
      shoulder_pitch_joint: 0.0
      elbow_pitch_joint: 0.0
      head_roll_joint: 0.0
      head_pitch_joint: 0.0
"#,
        )
        .unwrap();
        assert!(
            PoseLibrary::load(&unknown, &ORION_JOINT_NAMES)
                .unwrap_err()
                .to_string()
                .contains("unknown field")
        );
    }
}
