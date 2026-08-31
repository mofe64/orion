use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

const USER_SCENE_DIRECTORY: &str = "scenes/user";
const MAX_SCENE_BYTES: u64 = 262_144;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SceneDocument {
    format_version: u32,
    scene: SceneDefinition,
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
        for path in ["description/urdf", "motion/config", "scenes"] {
            fs::create_dir_all(temporary.path().join(path)).unwrap();
        }
        for path in [
            "AGENTS.md",
            "description/urdf/orion.urdf",
            "motion/config/poses.yaml",
        ] {
            fs::write(temporary.path().join(path), "test").unwrap();
        }
        temporary
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
}
