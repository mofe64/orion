//! Within our runtime, a pose is a named set of target joint angles such as home, attentive, or thinking,
//! with optional metadata. The four structs defined here, `PoseLibrary`, `PoseDefinition`, `PoseEntry`, and `JointPositions`,
//! represent how poses are loaded from their Yaml defintions into the runtime.
//!
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::shared::{collect_yaml_files, is_semantic_name};
use crate::devices::lighting::LIGHTING_EFFECT_NAMES;
use crate::error::{OrionRuntimeError, Result};

pub type JointPositions = BTreeMap<String, f64>;

pub const POSE_FORMAT_VERSION: u32 = 2;

/// A PoseDocument represents the Yaml defintion file which contains all our predefined poses.
/// it fields describe the file format, angle units and the actual pose entries.
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

/// A PoseEntry represents a single pose definition within the Yaml definition file,
/// it includes the following fields:
/// - description: human-readable description of the pose
/// - tags: labels such as powered or idle_anchor
/// - idle_profile: optional name of the idle profile to use for this pose
/// - default_lighting: optional name of the default lighting effect to use for this pose
/// - positions: joint names mapped to target angles in radians
/// The name field is derived from the key in the Yaml file.
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

/// A PoseDefinition is the parsed and loaded pose entry from the Yaml definition file.
/// it carries the same metadata and positions as the PoseEntry
#[derive(Clone, Debug)]
pub struct PoseDefinition {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub idle_profile: Option<String>,
    pub default_lighting: Option<String>,
    pub positions: JointPositions,
}

/// The PoseLibrary is a collection of PoseDefinitions loaded from the Yaml definition file.
/// it stores a private map of pose names to PoseDefinitions.
/// Pose library does two jobs:
/// - loading pose files into memory
/// - letting the rest of the orion runtime retrieve our poses
#[derive(Clone, Debug)]
pub struct PoseLibrary {
    poses: BTreeMap<String, PoseDefinition>,
}

impl PoseLibrary {
    /// Loads a pose library from a single supplied yaml file path.
    /// joint_names supplies the expected robot joints, and every pose must contain exactly those joints.
    pub fn load(path: impl AsRef<Path>, joint_names: &[impl AsRef<str>]) -> Result<Self> {
        Self::load_files(vec![path.as_ref().to_path_buf()], joint_names)
    }

    /// Loads a pose library from the system poses file (our system poses) and the user's
    /// custom pose directory (if it exists).
    /// this lets the orion runtime keep its system poses such as home... while also loading
    /// user defined poses from the user's custom pose directory.
    /// missing user directory is allowed, since only system poses from the built-in directory are loaded.
    /// the collected file paths are sorted before loading,
    /// duplicate pose names cause an error, since user supplied poses cannot override system poses.
    pub fn load_with_user_directory(
        built_in_path: impl AsRef<Path>,
        user_directory: impl AsRef<Path>,
        joint_names: &[impl AsRef<str>],
    ) -> Result<Self> {
        let mut files = vec![built_in_path.as_ref().to_path_buf()];
        let user_directory = user_directory.as_ref();
        if user_directory.exists() {
            collect_yaml_files(user_directory, &mut files, "user pose library")?;
        }
        files.sort();
        Self::load_files(files, joint_names)
    }

    /// Internal private helper that acts as share loading implementation, for both public loaders.
    /// 1. coverts the expected joint names to a sorted set for comparison.
    /// 2. creates an empty map of pose definitions
    /// 3. calls load_pose_file for each path to parse, validate and insert it poses into the map.
    /// 4. rejects an empty pose library.
    /// 5. returns the completed pose library.
    /// # Errors
    ///
    /// Returns [`OrionRuntimeError::Io`] if the file cannot be read,
    /// [`OrionRuntimeError::Json`] if the file is not valid JSON, or [`OrionRuntimeError::Runtime`] if the file is not a valid pose definition.
    fn load_files(files: Vec<PathBuf>, joint_names: &[impl AsRef<str>]) -> Result<Self> {
        let expected: BTreeSet<String> = joint_names
            .iter()
            .map(|name| name.as_ref().to_owned())
            .collect();
        let mut poses: BTreeMap<String, PoseDefinition> = BTreeMap::new();
        for path in files {
            load_pose_file(&path, &expected, &mut poses)?;
        }
        if poses.is_empty() {
            return Err(OrionRuntimeError::Runtime(
                ("Pose library contains no poses.").into(),
            ));
        }
        Ok(Self { poses })
    }

    /// Returns an iterator over the pose definitions in this library.
    /// because our library is backed by a BTreeMap, the iterator is sorted by pose name.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &JointPositions)> {
        self.poses
            .iter()
            .map(|(name, pose)| (name, &pose.positions))
    }

    /// Returns one pose's joint targets.
    /// It returns &JointPositions: a borrowed map of joint names to angles in radians.
    /// # Errors
    ///
    /// Returns [`OrionRuntimeError::InvalidArgument`] if the pose is not found.
    pub fn pose(&self, name: &str) -> Result<&JointPositions> {
        self.poses
            .get(name)
            .map(|pose| &pose.positions)
            .ok_or_else(|| {
                OrionRuntimeError::InvalidArgument(format!("Unknown Orion pose: {name}"))
            })
    }

    /// Retrieves one pose with all its metadata.
    ///
    /// # Errors
    ///
    /// Returns [`OrionRuntimeError::InvalidArgument`] if the pose is not found.
    pub fn definition(&self, name: &str) -> Result<&PoseDefinition> {
        self.poses.get(name).ok_or_else(|| {
            OrionRuntimeError::InvalidArgument(format!("Unknown Orion pose: {name}"))
        })
    }

    /// Returns a list of all pose names in the library.
    /// It clones the names, so that the caller can keep or modify the returned list
    /// without modifying the library's internal state.
    pub fn names(&self) -> Vec<String> {
        self.poses.keys().cloned().collect()
    }
}

/// Loads a single pose file from the given path and adds returns all the pose definitions in it.
fn load_pose_file(
    path: &Path,
    expected: &BTreeSet<String>,
    poses: &mut BTreeMap<String, PoseDefinition>,
) -> Result<()> {
    let contents: String = fs::read_to_string(path).map_err(|error| {
        OrionRuntimeError::Runtime(format!(
            "Could not parse pose library '{}': {error}",
            path.display()
        ))
    })?;
    let document: PoseDocument = serde_yaml::from_str(&contents).map_err(|error| {
        OrionRuntimeError::Runtime(format!(
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
        return Err(OrionRuntimeError::Runtime(
            ("Pose library must use format_version 2 with radian positions (v2 required).").into(),
        ));
    }

    if document.poses.is_empty() {
        return Err(OrionRuntimeError::Runtime(
            ("Pose library contains no poses.").into(),
        ));
    }

    for (pose_name, entry) in document.poses {
        if !is_semantic_name(&pose_name) {
            return Err(OrionRuntimeError::Runtime(format!(
                "Pose name '{pose_name}' is not a semantic Orion name."
            )));
        }

        let present: BTreeSet<String> = entry.positions.keys().cloned().collect();
        if &present != expected {
            return Err(OrionRuntimeError::Runtime(format!(
                "Pose '{pose_name}' joint names do not match Orion."
            )));
        }

        if entry.positions.values().any(|value| !value.is_finite()) {
            return Err(OrionRuntimeError::Runtime(format!(
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
            return Err(OrionRuntimeError::Runtime(format!(
                "Pose '{pose_name}' contains a invalid semantic tag, idle profile, or lighting effect."
            )));
        }
        let defintion = PoseDefinition {
            name: pose_name.clone(),
            description: entry.description,
            tags: entry.tags,
            idle_profile: entry.idle_profile,
            default_lighting: entry.default_lighting,
            positions: entry.positions,
        };
        if poses.insert(pose_name.clone(), defintion).is_some() {
            return Err(OrionRuntimeError::Runtime(format!(
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
    use crate::motion::shared::fixture_path;

    fn expected_joints() -> BTreeSet<String> {
        ORION_JOINT_NAMES
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

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
        fs::copy(
            fixture_path("studio_keyframe.yaml", "poses"),
            user.join("studio_keyframe.yaml"),
        )
        .unwrap();
        let built_in = concat!(env!("CARGO_MANIFEST_DIR"), "/../motion/config/poses.yaml");
        let poses =
            PoseLibrary::load_with_user_directory(built_in, &user, &ORION_JOINT_NAMES).unwrap();
        assert!(poses.pose("studio_keyframe").is_ok());

        fs::copy(
            fixture_path("duplicate.yaml", "poses"),
            user.join("duplicate.yaml"),
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
        let v1 = fixture_path("unsupported_version.yaml", "poses");
        assert!(
            PoseLibrary::load(&v1, &ORION_JOINT_NAMES)
                .unwrap_err()
                .to_string()
                .contains("v2 required")
        );

        let unknown = fixture_path("unknown_field.yaml", "poses");
        assert!(
            PoseLibrary::load(&unknown, &ORION_JOINT_NAMES)
                .unwrap_err()
                .to_string()
                .contains("unknown field")
        );
    }

    #[test]
    fn definition_preserves_metadata_and_pose_returns_its_positions() {
        let library =
            PoseLibrary::load(fixture_path("metadata.yaml", "poses"), &ORION_JOINT_NAMES).unwrap();
        let definition = library.definition("studio_keyframe").unwrap();
        assert_eq!(definition.name, "studio_keyframe");
        assert_eq!(definition.description, "Studio-authored keyframe.");
        assert_eq!(definition.tags, ["studio", "keyframe-1"]);
        assert_eq!(definition.idle_profile.as_deref(), Some("studio_idle"));
        assert_eq!(
            definition.default_lighting.as_deref(),
            Some("warm_idle_breathe")
        );
        let positions = library.pose("studio_keyframe").unwrap();
        let expected: JointPositions = [
            ("base_yaw_joint", 0.1),
            ("shoulder_pitch_joint", -0.1),
            ("elbow_pitch_joint", 0.2),
            ("head_roll_joint", 0.0),
            ("head_pitch_joint", -0.2),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect();
        assert_eq!(positions, &expected);
        assert_eq!(positions, &definition.positions);
    }

    #[test]
    fn lookups_report_unknown_pose_as_invalid_argument() {
        let library =
            PoseLibrary::load(fixture_path("metadata.yaml", "poses"), &ORION_JOINT_NAMES).unwrap();
        for error in [
            library.pose("missing").unwrap_err(),
            library.definition("missing").unwrap_err(),
        ] {
            assert!(
                matches!(error, OrionRuntimeError::InvalidArgument(ref message)
                if message == "Unknown Orion pose: missing")
            );
        }
    }

    #[test]
    fn iter_and_names_return_all_poses_in_name_order() {
        let library = PoseLibrary::load_files(
            vec![
                fixture_path("studio_keyframe.yaml", "poses"),
                fixture_path("duplicate.yaml", "poses"),
            ],
            &ORION_JOINT_NAMES,
        )
        .unwrap();
        assert_eq!(library.names(), ["home", "studio_keyframe"]);
        let entries: Vec<_> = library.iter().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "home");
        assert_eq!(entries[1].0, "studio_keyframe");
        assert_eq!(entries[0].1.len(), 5);
        assert!(entries[0].1.values().all(|value| *value == 0.0));
        assert_eq!(entries[1].1["base_yaw_joint"], 0.1);
        let mut names = library.names();
        names.clear();
        assert_eq!(library.names(), ["home", "studio_keyframe"]);
    }

    #[test]
    fn user_directory_may_be_missing_or_empty() {
        let root = tempfile::tempdir().unwrap();
        for user in [root.path().join("missing"), root.path().to_path_buf()] {
            let library = PoseLibrary::load_with_user_directory(
                fixture_path("metadata.yaml", "poses"),
                user,
                &ORION_JOINT_NAMES,
            )
            .unwrap();
            assert_eq!(library.names(), ["studio_keyframe"]);
        }
    }

    #[test]
    fn user_directory_loads_nested_yml_and_propagates_directory_errors() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::copy(
            fixture_path("studio_keyframe.yaml", "poses"),
            nested.join("pose.yml"),
        )
        .unwrap();
        let library = PoseLibrary::load_with_user_directory(
            fixture_path("duplicate.yaml", "poses"),
            root.path(),
            &ORION_JOINT_NAMES,
        )
        .unwrap();
        assert_eq!(library.names(), ["home", "studio_keyframe"]);
        let error = PoseLibrary::load_with_user_directory(
            fixture_path("duplicate.yaml", "poses"),
            fixture_path("metadata.yaml", "poses"),
            &ORION_JOINT_NAMES,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Could not read user pose library")
        );
    }

    #[test]
    fn load_files_rejects_no_files() {
        let error = PoseLibrary::load_files(Vec::new(), &ORION_JOINT_NAMES).unwrap_err();
        assert!(matches!(error, OrionRuntimeError::Runtime(ref message)
            if message == "Pose library contains no poses."));
    }

    #[test]
    fn load_pose_file_applies_optional_field_defaults() {
        let mut poses = BTreeMap::new();
        load_pose_file(
            &fixture_path("defaults.yaml", "poses"),
            &expected_joints(),
            &mut poses,
        )
        .unwrap();
        let definition = &poses["studio_keyframe"];
        assert_eq!(poses.len(), 1);
        assert!(definition.description.is_empty());
        assert!(definition.tags.is_empty());
        assert_eq!(definition.idle_profile, None);
        assert_eq!(definition.default_lighting, None);
        assert_eq!(definition.positions.len(), 5);
    }

    #[test]
    fn load_pose_file_reports_missing_file_and_malformed_yaml_with_path() {
        let root = tempfile::tempdir().unwrap();
        for path in [
            root.path().join("missing.yaml"),
            fixture_path("malformed.yaml", "poses"),
        ] {
            let mut poses = BTreeMap::new();
            let error = load_pose_file(&path, &expected_joints(), &mut poses).unwrap_err();
            assert!(matches!(error, OrionRuntimeError::Runtime(_)));
            let message = error.to_string();
            assert!(message.contains("Could not parse pose library"));
            assert!(message.contains(&path.display().to_string()));
            assert!(poses.is_empty());
        }
    }

    #[test]
    fn load_pose_file_rejects_invalid_documents() {
        let cases = [
            ("unsupported_version.yaml", "v2 required"),
            ("missing_version.yaml", "v2 required"),
            ("wrong_units.yaml", "v2 required"),
            ("empty_library.yaml", "contains no poses"),
            ("unknown_field.yaml", "unknown field"),
            ("unknown_pose_field.yaml", "unknown field"),
            ("invalid_pose_name.yaml", "not a semantic Orion name"),
            ("empty_pose_name.yaml", "not a semantic Orion name"),
            ("missing_joint.yaml", "joint names do not match"),
            ("extra_joint.yaml", "joint names do not match"),
            ("nan_position.yaml", "non-finite target"),
            ("infinite_position.yaml", "non-finite target"),
            (
                "invalid_tag.yaml",
                "invalid semantic tag, idle profile, or lighting effect",
            ),
            (
                "invalid_idle_profile.yaml",
                "invalid semantic tag, idle profile, or lighting effect",
            ),
            (
                "invalid_lighting_name.yaml",
                "invalid semantic tag, idle profile, or lighting effect",
            ),
            (
                "unknown_lighting.yaml",
                "invalid semantic tag, idle profile, or lighting effect",
            ),
        ];
        for (fixture, expected_message) in cases {
            let mut poses = BTreeMap::new();
            let error = load_pose_file(
                &fixture_path(fixture, "poses"),
                &expected_joints(),
                &mut poses,
            )
            .expect_err(fixture);
            assert!(
                matches!(error, OrionRuntimeError::Runtime(_)),
                "{fixture}: {error}"
            );
            assert!(
                error.to_string().contains(expected_message),
                "{fixture}: {error}"
            );
            assert!(poses.is_empty(), "{fixture} inserted an invalid pose");
        }
    }

    #[test]
    fn load_pose_file_reports_duplicate_name_and_file() {
        let path = fixture_path("studio_keyframe.yaml", "poses");
        let mut poses = BTreeMap::new();
        load_pose_file(&path, &expected_joints(), &mut poses).unwrap();
        let error = load_pose_file(&path, &expected_joints(), &mut poses).unwrap_err();
        assert!(matches!(error, OrionRuntimeError::Runtime(_)));
        let message = error.to_string();
        assert!(message.contains("Duplicate Orion pose name 'studio_keyframe'"));
        assert!(message.contains(&path.display().to_string()));
    }
}
