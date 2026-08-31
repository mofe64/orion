use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

const USER_SCENE_DIRECTORY: &str = "scenes/user";
const USER_POSE_DIRECTORY: &str = "motion/user/poses";
const USER_MOTION_DIRECTORY: &str = "motion/motions/user";
const MAX_SCENE_BYTES: u64 = 262_144;
const JOINT_NAMES: [&str; 5] = [
    "base_yaw_joint",
    "shoulder_pitch_joint",
    "elbow_pitch_joint",
    "head_roll_joint",
    "head_pitch_joint",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SceneDocument {
    format_version: u32,
    scene: SceneDefinition,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PoseDocument {
    format_version: u32,
    units: String,
    poses: BTreeMap<String, PoseEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PoseEntry {
    description: String,
    positions: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MotionDocument {
    format_version: u32,
    motion: MotionEntry,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct MotionEntry {
    name: String,
    description: String,
    keyframes: Vec<MotionKeyframeEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct MotionKeyframeEntry {
    pose: String,
    duration: f64,
    hold: f64,
}

#[derive(Debug, Deserialize)]
struct ExistingMotionDocument {
    motion: Option<ExistingMotionEntry>,
}

#[derive(Debug, Deserialize)]
struct ExistingMotionEntry {
    name: String,
}

#[derive(Debug, Deserialize)]
struct MotionLimitsDocument {
    joints: BTreeMap<String, JointMotionLimit>,
}

#[derive(Debug, Deserialize)]
struct JointMotionLimit {
    operational_position: PositionRange,
}

#[derive(Debug, Deserialize)]
struct PositionRange {
    lower: f64,
    upper: f64,
}

#[derive(Debug, Deserialize)]
struct BuiltInPoseDocument {
    poses: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SceneDefinition {
    name: String,
    description: String,
    timeline: Vec<SceneEvent>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SceneEvent {
    at: f64,
    #[serde(flatten)]
    action: SceneAction,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SceneAction {
    PlayMotion {
        motion: String,
    },
    GotoPose {
        pose: String,
        duration_seconds: f64,
    },
    Light {
        red: u8,
        green: u8,
        blue: u8,
        white: u8,
        transition_seconds: f64,
    },
    Audio {
        cue: String,
    },
}

#[derive(Debug, Serialize)]
pub struct SavedScene {
    pub name: String,
    pub relative_path: String,
}

pub fn validate_project_root(path: &Path) -> Result<PathBuf, String> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("The Orion project root cannot contain '..'.".into());
    }
    let root = path.canonicalize().map_err(|error| {
        format!(
            "Could not open Orion project root '{}': {error}",
            path.display()
        )
    })?;
    for required in [
        "AGENTS.md",
        "description/urdf/orion.urdf",
        "motion/config/poses.yaml",
        "motion/config/motion_limits.yaml",
        "motion/motions",
        "scenes",
    ] {
        if !root.join(required).exists() {
            return Err(format!(
                "'{}' is not an Orion project root; missing {required}.",
                root.display()
            ));
        }
    }
    Ok(root)
}

pub fn save_user_scene(
    project_root: impl AsRef<Path>,
    document: &SceneDocument,
) -> Result<SavedScene, String> {
    let root = validate_project_root(project_root.as_ref())?;
    validate_scene(document)?;
    save_scene_to_root(&root, document)
}

pub fn load_user_scenes(project_root: impl AsRef<Path>) -> Result<Vec<SceneDocument>, String> {
    let root = validate_project_root(project_root.as_ref())?;
    let directory = root.join(USER_SCENE_DIRECTORY);
    if !directory.exists() {
        return Ok(Vec::new());
    }

    let mut paths = fs::read_dir(&directory)
        .map_err(|error| format!("Could not read '{}': {error}", directory.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            let path = entry.path();
            let extension = path.extension()?.to_str()?;
            (file_type.is_file() && matches!(extension, "yaml" | "yml")).then_some(path)
        })
        .collect::<Vec<_>>();
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let metadata = fs::metadata(&path)
                .map_err(|error| format!("Could not inspect '{}': {error}", path.display()))?;
            if metadata.len() > MAX_SCENE_BYTES {
                return Err(format!(
                    "User scene '{}' is larger than {} bytes.",
                    path.display(),
                    MAX_SCENE_BYTES
                ));
            }
            let yaml = fs::read_to_string(&path)
                .map_err(|error| format!("Could not read '{}': {error}", path.display()))?;
            let document: SceneDocument = serde_yaml::from_str(&yaml)
                .map_err(|error| format!("Could not parse '{}': {error}", path.display()))?;
            validate_scene(&document)?;

            for extension in ["yaml", "yml"] {
                if root
                    .join("scenes")
                    .join(format!("{}.{}", document.scene.name, extension))
                    .exists()
                {
                    return Err(format!(
                        "User scene '{}' shadows a built-in scene and was not loaded.",
                        document.scene.name
                    ));
                }
            }
            Ok(document)
        })
        .collect()
}

pub fn save_user_pose(
    project_root: impl AsRef<Path>,
    document: &PoseDocument,
) -> Result<SavedScene, String> {
    let root = validate_project_root(project_root.as_ref())?;
    let name = validate_pose(&root, document)?;
    let directory = root.join(USER_POSE_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "Could not create Studio pose directory '{}': {error}",
            directory.display()
        )
    })?;
    let yaml = serde_yaml::to_string(document)
        .map_err(|error| format!("Could not serialize the pose: {error}"))?;
    let path = directory.join(format!("{name}.yaml"));
    write_create_new(&path, yaml.as_bytes(), "pose", &name)?;
    Ok(SavedScene {
        name: name.clone(),
        relative_path: format!("{USER_POSE_DIRECTORY}/{name}.yaml"),
    })
}

pub fn load_user_poses(project_root: impl AsRef<Path>) -> Result<Vec<PoseDocument>, String> {
    let root = validate_project_root(project_root.as_ref())?;
    let directory = root.join(USER_POSE_DIRECTORY);
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut paths = yaml_files(&directory)?;
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let yaml = read_bounded(&path)?;
            let document: PoseDocument = serde_yaml::from_str(&yaml)
                .map_err(|error| format!("Could not parse '{}': {error}", path.display()))?;
            let name = validate_pose(&root, &document)?;
            if path.file_stem().and_then(|value| value.to_str()) != Some(name.as_str()) {
                return Err(format!(
                    "User pose filename '{}' does not match pose name '{name}'.",
                    path.display()
                ));
            }
            Ok(document)
        })
        .collect()
}

pub fn save_user_motion(
    project_root: impl AsRef<Path>,
    document: &MotionDocument,
) -> Result<SavedScene, String> {
    let root = validate_project_root(project_root.as_ref())?;
    let name = validate_motion(&root, document, false)?;
    let directory = root.join(USER_MOTION_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "Could not create Studio motion directory '{}': {error}",
            directory.display()
        )
    })?;
    let yaml = serde_yaml::to_string(document)
        .map_err(|error| format!("Could not serialize the motion: {error}"))?;
    let path = directory.join(format!("{name}.yaml"));
    write_create_new(&path, yaml.as_bytes(), "motion", &name)?;
    Ok(SavedScene {
        name: name.clone(),
        relative_path: format!("{USER_MOTION_DIRECTORY}/{name}.yaml"),
    })
}

pub fn load_user_motions(project_root: impl AsRef<Path>) -> Result<Vec<MotionDocument>, String> {
    let root = validate_project_root(project_root.as_ref())?;
    let directory = root.join(USER_MOTION_DIRECTORY);
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut paths = yaml_files(&directory)?;
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let yaml = read_bounded(&path)?;
            let document: MotionDocument = serde_yaml::from_str(&yaml)
                .map_err(|error| format!("Could not parse '{}': {error}", path.display()))?;
            let name = validate_motion(&root, &document, true)?;
            if path.file_stem().and_then(|value| value.to_str()) != Some(name.as_str()) {
                return Err(format!(
                    "User motion filename '{}' does not match motion name '{name}'.",
                    path.display()
                ));
            }
            Ok(document)
        })
        .collect()
}

fn validate_pose(root: &Path, document: &PoseDocument) -> Result<String, String> {
    if document.format_version != 1 || document.units != "radians" {
        return Err("Studio poses must use format_version 1 and radians.".into());
    }
    if document.poses.len() != 1 {
        return Err("A Studio user-pose file must contain exactly one pose.".into());
    }
    let (name, pose) = document.poses.first_key_value().unwrap();
    validate_semantic_name(name, "pose")?;
    let expected: BTreeSet<String> = JOINT_NAMES.iter().map(|name| (*name).to_owned()).collect();
    let present: BTreeSet<String> = pose.positions.keys().cloned().collect();
    if present != expected {
        return Err("A pose must contain every Orion joint exactly once.".into());
    }

    let built_in_yaml = fs::read_to_string(root.join("motion/config/poses.yaml"))
        .map_err(|error| format!("Could not read built-in poses: {error}"))?;
    let built_in: BuiltInPoseDocument = serde_yaml::from_str(&built_in_yaml)
        .map_err(|error| format!("Could not parse built-in poses: {error}"))?;
    if built_in.poses.contains_key(name) {
        return Err(format!(
            "'{name}' is a built-in pose. Choose a new name; Studio never shadows commissioned poses."
        ));
    }

    let limits_yaml = fs::read_to_string(root.join("motion/config/motion_limits.yaml"))
        .map_err(|error| format!("Could not read tracked motion limits: {error}"))?;
    let limits: MotionLimitsDocument = serde_yaml::from_str(&limits_yaml)
        .map_err(|error| format!("Could not parse tracked motion limits: {error}"))?;
    if limits.joints.keys().cloned().collect::<BTreeSet<_>>() != expected {
        return Err("Tracked motion limits do not contain the Orion joint contract.".into());
    }
    for (joint, value) in &pose.positions {
        let range = &limits.joints[joint].operational_position;
        if !value.is_finite()
            || !range.lower.is_finite()
            || !range.upper.is_finite()
            || range.lower >= range.upper
            || *value < range.lower
            || *value > range.upper
        {
            return Err(format!(
                "Pose position for {joint} must stay between {:.6} and {:.6} radians.",
                range.lower, range.upper
            ));
        }
    }
    Ok(name.clone())
}

fn validate_motion(
    root: &Path,
    document: &MotionDocument,
    allow_existing_user: bool,
) -> Result<String, String> {
    if document.format_version != 1 {
        return Err("Studio motions must use format_version 1.".into());
    }
    validate_semantic_name(&document.motion.name, "motion")?;
    if document.motion.keyframes.is_empty() {
        return Err("A motion must contain at least one keyframe.".into());
    }

    let built_in_yaml = fs::read_to_string(root.join("motion/config/poses.yaml"))
        .map_err(|error| format!("Could not read built-in poses: {error}"))?;
    let built_in: BuiltInPoseDocument = serde_yaml::from_str(&built_in_yaml)
        .map_err(|error| format!("Could not parse built-in poses: {error}"))?;
    let mut pose_names = built_in.poses.keys().cloned().collect::<BTreeSet<_>>();
    for pose in load_user_poses(root)? {
        pose_names.extend(pose.poses.keys().cloned());
    }
    for keyframe in &document.motion.keyframes {
        validate_semantic_name(&keyframe.pose, "pose")?;
        if !pose_names.contains(&keyframe.pose) {
            return Err(format!(
                "Motion '{}' references unknown pose '{}'.",
                document.motion.name, keyframe.pose
            ));
        }
        validate_duration(keyframe.duration, "Keyframe duration", false)?;
        validate_duration(keyframe.hold, "Keyframe hold", true)?;
    }

    if !allow_existing_user {
        let mut files = Vec::new();
        collect_yaml_files_recursive(&root.join("motion/motions"), &mut files)?;
        for path in files {
            let yaml = read_bounded(&path)?;
            let existing: ExistingMotionDocument = serde_yaml::from_str(&yaml)
                .map_err(|error| format!("Could not parse '{}': {error}", path.display()))?;
            if existing
                .motion
                .is_some_and(|motion| motion.name == document.motion.name)
            {
                return Err(format!(
                    "An Orion motion named '{}' already exists. Choose a new name.",
                    document.motion.name
                ));
            }
        }
    }
    Ok(document.motion.name.clone())
}

fn collect_yaml_files_recursive(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("Could not read '{}': {error}", directory.display()))?
    {
        let path = entry
            .map_err(|error| format!("Could not read motion directory entry: {error}"))?
            .path();
        if path.is_dir() {
            collect_yaml_files_recursive(&path, files)?;
        } else if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yaml" | "yml")
        ) {
            files.push(path);
        }
    }
    Ok(())
}

fn yaml_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let files = fs::read_dir(directory)
        .map_err(|error| format!("Could not read '{}': {error}", directory.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            let path = entry.path();
            let extension = path.extension()?.to_str()?;
            (file_type.is_file() && matches!(extension, "yaml" | "yml")).then_some(path)
        })
        .collect::<Vec<_>>();
    Ok(files)
}

fn read_bounded(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Could not inspect '{}': {error}", path.display()))?;
    if metadata.len() > MAX_SCENE_BYTES {
        return Err(format!(
            "User asset '{}' is larger than {} bytes.",
            path.display(),
            MAX_SCENE_BYTES
        ));
    }
    fs::read_to_string(path)
        .map_err(|error| format!("Could not read '{}': {error}", path.display()))
}

fn write_create_new(path: &Path, bytes: &[u8], label: &str, name: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "A user {label} named '{name}' already exists. Use a new name; Studio never overwrites {label}s."
                )
            } else {
                format!("Could not create user {label} '{}': {error}", path.display())
            }
        })?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Could not write user {label} '{}': {error}", path.display()))
}

fn validate_scene(document: &SceneDocument) -> Result<(), String> {
    if document.format_version != 1 {
        return Err("Studio currently saves scene format_version 1 only.".into());
    }
    validate_semantic_name(&document.scene.name, "scene")?;
    if document.scene.timeline.is_empty() {
        return Err("A scene must contain at least one timeline event.".into());
    }

    let mut previous_at = 0.0;
    for (index, event) in document.scene.timeline.iter().enumerate() {
        if !event.at.is_finite() || event.at < 0.0 || (index > 0 && event.at < previous_at) {
            return Err("Scene event times must be finite, non-negative, and ordered.".into());
        }
        previous_at = event.at;
        match &event.action {
            SceneAction::PlayMotion { motion } => validate_semantic_name(motion, "motion")?,
            SceneAction::GotoPose {
                pose,
                duration_seconds,
            } => {
                validate_semantic_name(pose, "pose")?;
                validate_duration(*duration_seconds, "Pose duration", false)?;
            }
            SceneAction::Light {
                transition_seconds, ..
            } => validate_duration(*transition_seconds, "Light transition", true)?,
            SceneAction::Audio { cue } => validate_semantic_name(cue, "audio cue")?,
        }
    }
    Ok(())
}

fn validate_semantic_name(name: &str, label: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(format!(
            "The {label} name must contain only letters, numbers, underscores, and hyphens."
        ));
    }
    Ok(())
}

fn validate_duration(value: f64, label: &str, allow_zero: bool) -> Result<(), String> {
    if !value.is_finite() || value > 300.0 || value < 0.0 || (!allow_zero && value == 0.0) {
        let lower = if allow_zero { "0" } else { "greater than 0" };
        return Err(format!(
            "{label} must be {lower} and no more than 300 seconds."
        ));
    }
    Ok(())
}

fn save_scene_to_root(root: &Path, document: &SceneDocument) -> Result<SavedScene, String> {
    for extension in ["yaml", "yml"] {
        let built_in = root
            .join("scenes")
            .join(format!("{}.{}", document.scene.name, extension));
        if built_in.exists() {
            return Err(format!(
                "'{}' is a built-in scene. Choose a new Save As name; Studio never shadows commissioned scenes.",
                document.scene.name
            ));
        }
    }
    let directory = root.join(USER_SCENE_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "Could not create Studio scene directory '{}': {error}",
            directory.display()
        )
    })?;

    let yaml = serde_yaml::to_string(document)
        .map_err(|error| format!("Could not serialize the scene: {error}"))?;
    let path = directory.join(format!("{}.yaml", document.scene.name));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "A user scene named '{}' already exists. Use Save As with a new name; Studio never overwrites scenes.",
                    document.scene.name
                )
            } else {
                format!("Could not create user scene '{}': {error}", path.display())
            }
        })?;
    file.write_all(yaml.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Could not write user scene '{}': {error}", path.display()))?;

    Ok(SavedScene {
        name: document.scene.name.clone(),
        relative_path: format!("{USER_SCENE_DIRECTORY}/{}.yaml", document.scene.name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(name: &str) -> SceneDocument {
        SceneDocument {
            format_version: 1,
            scene: SceneDefinition {
                name: name.into(),
                description: "A Studio test scene.".into(),
                timeline: vec![SceneEvent {
                    at: 0.0,
                    action: SceneAction::GotoPose {
                        pose: "home".into(),
                        duration_seconds: 2.0,
                    },
                }],
            },
        }
    }

    fn project() -> tempfile::TempDir {
        let temporary = tempfile::tempdir().unwrap();
        for path in [
            "description/urdf",
            "motion/config",
            "motion/motions",
            "scenes",
        ] {
            fs::create_dir_all(temporary.path().join(path)).unwrap();
        }
        for path in ["AGENTS.md", "description/urdf/orion.urdf"] {
            fs::write(temporary.path().join(path), "test").unwrap();
        }
        fs::write(
            temporary.path().join("motion/config/poses.yaml"),
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
        fs::write(
            temporary.path().join("motion/config/motion_limits.yaml"),
            r#"joints:
  base_yaw_joint:
    operational_position: { lower: -1.0, upper: 1.0 }
  shoulder_pitch_joint:
    operational_position: { lower: -1.0, upper: 1.0 }
  elbow_pitch_joint:
    operational_position: { lower: -1.0, upper: 1.0 }
  head_roll_joint:
    operational_position: { lower: -1.0, upper: 1.0 }
  head_pitch_joint:
    operational_position: { lower: -1.0, upper: 1.0 }
"#,
        )
        .unwrap();
        temporary
    }

    fn pose(name: &str, base: f64) -> PoseDocument {
        PoseDocument {
            format_version: 1,
            units: "radians".into(),
            poses: [(
                name.into(),
                PoseEntry {
                    description: "A Studio pose.".into(),
                    positions: JOINT_NAMES
                        .iter()
                        .map(|joint| {
                            (
                                (*joint).into(),
                                if *joint == "base_yaw_joint" {
                                    base
                                } else {
                                    0.0
                                },
                            )
                        })
                        .collect(),
                },
            )]
            .into_iter()
            .collect(),
        }
    }

    fn motion(name: &str, pose: &str) -> MotionDocument {
        MotionDocument {
            format_version: 1,
            motion: MotionEntry {
                name: name.into(),
                description: "A Studio motion.".into(),
                keyframes: vec![MotionKeyframeEntry {
                    pose: pose.into(),
                    duration: 1.5,
                    hold: 0.2,
                }],
            },
        }
    }

    #[test]
    fn rejects_malformed_unordered_and_zero_duration_scenes() {
        let mut document = scene("bad name");
        assert!(validate_scene(&document).is_err());
        document.scene.name = "valid_name".into();
        document.scene.timeline[0].action = SceneAction::GotoPose {
            pose: "home".into(),
            duration_seconds: 0.0,
        };
        assert!(validate_scene(&document).is_err());
        document.scene.timeline.push(SceneEvent {
            at: -1.0,
            action: SceneAction::Audio {
                cue: "acknowledge".into(),
            },
        });
        assert!(validate_scene(&document).is_err());
    }

    #[test]
    fn saves_only_new_user_scene_files() {
        let project = project();
        let document = scene("studio_test");
        let saved = save_user_scene(project.path(), &document).unwrap();
        assert_eq!(saved.relative_path, "scenes/user/studio_test.yaml");
        assert!(project.path().join(&saved.relative_path).exists());
        assert!(save_user_scene(project.path(), &document).is_err());
    }

    #[test]
    fn loads_validated_user_scenes_after_restart() {
        let project = project();
        let document = scene("studio_test");
        save_user_scene(project.path(), &document).unwrap();

        let loaded = load_user_scenes(project.path()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].scene.name, "studio_test");

        fs::write(
            project.path().join("scenes/user/invalid.yaml"),
            "format_version: 99\nscene: {}\n",
        )
        .unwrap();
        assert!(load_user_scenes(project.path()).is_err());
    }

    #[test]
    fn refuses_to_shadow_a_built_in_scene() {
        let project = project();
        fs::write(project.path().join("scenes/commissioned.yaml"), "existing").unwrap();
        assert!(save_user_scene(project.path(), &scene("commissioned")).is_err());
        assert!(
            !project
                .path()
                .join("scenes/user/commissioned.yaml")
                .exists()
        );
    }

    #[test]
    fn rejects_a_directory_that_is_not_an_orion_project() {
        let temporary = tempfile::tempdir().unwrap();
        assert!(validate_project_root(temporary.path()).is_err());
    }

    #[test]
    fn saves_and_reloads_a_calibration_bounded_user_pose() {
        let project = project();
        let document = pose("studio_keyframe", 0.4);
        let saved = save_user_pose(project.path(), &document).unwrap();
        assert_eq!(
            saved.relative_path,
            "motion/user/poses/studio_keyframe.yaml"
        );
        assert_eq!(load_user_poses(project.path()).unwrap().len(), 1);
        assert!(save_user_pose(project.path(), &document).is_err());
    }

    #[test]
    fn refuses_out_of_range_and_built_in_user_poses() {
        let project = project();
        assert!(save_user_pose(project.path(), &pose("too_far", 1.1)).is_err());
        assert!(save_user_pose(project.path(), &pose("home", 0.0)).is_err());
        assert!(
            !project
                .path()
                .join("motion/user/poses/too_far.yaml")
                .exists()
        );
        assert!(!project.path().join("motion/user/poses/home.yaml").exists());
    }

    #[test]
    fn saves_and_reloads_a_named_user_motion() {
        let project = project();
        save_user_pose(project.path(), &pose("studio_keyframe", 0.4)).unwrap();
        let document = motion("studio_motion", "studio_keyframe");
        let saved = save_user_motion(project.path(), &document).unwrap();
        assert_eq!(
            saved.relative_path,
            "motion/motions/user/studio_motion.yaml"
        );
        assert_eq!(load_user_motions(project.path()).unwrap().len(), 1);
        assert!(save_user_motion(project.path(), &document).is_err());
    }

    #[test]
    fn refuses_a_motion_with_an_unknown_pose() {
        let project = project();
        assert!(save_user_motion(project.path(), &motion("bad_motion", "missing_pose")).is_err());
        assert!(
            !project
                .path()
                .join("motion/motions/user/bad_motion.yaml")
                .exists()
        );
    }
}
