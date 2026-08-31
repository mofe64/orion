use std::path::Path;

use orion_studio_scene_store::{MotionDocument, PoseDocument, SavedScene, SceneDocument};

#[tauri::command]
fn default_project_root() -> Result<String, String> {
    let root = if let Some(path) = std::env::var_os("ORION_PROJECT_ROOT") {
        orion_studio_scene_store::validate_project_root(Path::new(&path))?
    } else {
        let development_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| "Could not resolve the Orion development root.".to_owned())?;
        orion_studio_scene_store::validate_project_root(development_root)?
    };
    Ok(root.to_string_lossy().into_owned())
}

#[tauri::command]
fn save_user_scene(project_root: String, document: SceneDocument) -> Result<SavedScene, String> {
    orion_studio_scene_store::save_user_scene(project_root, &document)
}

#[tauri::command]
fn load_user_scenes(project_root: String) -> Result<Vec<SceneDocument>, String> {
    orion_studio_scene_store::load_user_scenes(project_root)
}

#[tauri::command]
fn save_user_pose(project_root: String, document: PoseDocument) -> Result<SavedScene, String> {
    orion_studio_scene_store::save_user_pose(project_root, &document)
}

#[tauri::command]
fn load_user_poses(project_root: String) -> Result<Vec<PoseDocument>, String> {
    orion_studio_scene_store::load_user_poses(project_root)
}

#[tauri::command]
fn save_user_motion(project_root: String, document: MotionDocument) -> Result<SavedScene, String> {
    orion_studio_scene_store::save_user_motion(project_root, &document)
}

#[tauri::command]
fn load_user_motions(project_root: String) -> Result<Vec<MotionDocument>, String> {
    orion_studio_scene_store::load_user_motions(project_root)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            default_project_root,
            save_user_scene,
            load_user_scenes,
            save_user_pose,
            load_user_poses,
            save_user_motion,
            load_user_motions
        ])
        .run(tauri::generate_context!())
        .expect("error while running Orion Studio");
}
