use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

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
        let path = path.as_ref();
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

        let expected: BTreeSet<String> = joint_names
            .iter()
            .map(|name| name.as_ref().to_owned())
            .collect();
        let mut poses = BTreeMap::new();
        for (pose_name, entry) in document.poses {
            let present: BTreeSet<String> = entry.positions.keys().cloned().collect();
            if present != expected {
                return Err(Error::Runtime(format!(
                    "Pose '{pose_name}' joint names do not match Orion."
                )));
            }
            if entry.positions.values().any(|value| !value.is_finite()) {
                return Err(Error::Runtime(format!(
                    "Pose '{pose_name}' contains a non-finite target."
                )));
            }
            poses.insert(pose_name, entry.positions);
        }
        Ok(Self { poses })
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
}
