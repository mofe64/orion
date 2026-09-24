use orion_service::{
    Backend, Request,
    settings::{VoiceSettings, expand_path},
};

#[tauri::command]
pub async fn load_voice_settings(
    backend: tauri::State<'_, Backend>,
) -> Result<serde_json::Value, String> {
    backend.request(Request::LoadSettings).await
}

#[tauri::command]
pub async fn load_voice_history(
    backend: tauri::State<'_, Backend>,
    session_id: Option<String>,
    before: Option<String>,
) -> Result<serde_json::Value, String> {
    backend.request(Request::History { session_id, before }).await
}
#[tauri::command]
pub async fn save_voice_settings(
    backend: tauri::State<'_, Backend>,
    settings: VoiceSettings,
) -> Result<serde_json::Value, String> {
    backend.request(Request::SaveSettings(settings)).await
}
#[tauri::command]
pub async fn voice_model_locations(
    backend: tauri::State<'_, Backend>,
    settings: VoiceSettings,
) -> Result<serde_json::Value, String> {
    backend.request(Request::ModelLocations(settings)).await
}
#[tauri::command]
pub async fn choose_voice_folder(initial: Option<String>) -> Result<Option<String>, String> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title("Choose voice model folder");
    if let Some(path) = initial.filter(|p| !p.is_empty()) {
        let path = expand_path(&path)?;
        if path.is_dir() {
            dialog = dialog.set_directory(path);
        }
    }
    Ok(dialog
        .pick_folder()
        .await
        .map(|folder| folder.path().to_string_lossy().into_owned()))
}
