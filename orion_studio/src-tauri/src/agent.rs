use orion_studio_service::{Backend, Request};

#[tauri::command]
pub async fn load_agent_profile(
    backend: tauri::State<'_, Backend>,
) -> Result<serde_json::Value, String> {
    backend.request(Request::Profile(None)).await
}
#[tauri::command]
pub async fn change_agent_profile(
    backend: tauri::State<'_, Backend>,
    change: orion_agent::profile::ProfileChange,
) -> Result<serde_json::Value, String> {
    backend.request(Request::Profile(Some(change))).await
}
