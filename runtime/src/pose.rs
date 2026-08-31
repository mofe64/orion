use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Error, Result};

pub type JointPositions = BTreeMap<String, f64>;

#[derive(Debug, Deserialize)]
struct PoseDocument {
    #[serde(default)]
    format_version: u32,
    #[serde(default)]
    units: String,
    #[serde(default)]
    poses: BTreeMap<String, PoseEntry>,
}

#[derive(Debug, Deserialize)]
struct PoseEntry {
    #[serde(default)]
    positions: JointPositions,
}

#[derive(Clone, Debug)]
pub struct PoseLibrary {
    poses: BTreeMap<String, JointPositions>,
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
        self.poses.iter()
    }

    pub fn pose(&self, name: &str) -> Result<&JointPositions> {
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
    poses: &mut BTreeMap<String, JointPositions>,
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

    if document.format_version != 1 || document.units != "radians" {
        return Err(Error::Runtime(
            "Pose library must use format_version 1 and radians.".into(),
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
        if poses.insert(pose_name.clone(), entry.positions).is_some() {
            return Err(Error::Runtime(format!(
                "Duplicate Orion pose name '{pose_name}' in {}.",
                path.display()
            )));
        }
    }
    Ok(())
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
            r#"format_version: 1
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
            r#"format_version: 1
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
}
