use orion_studio_service::{Backend, Request, pairing::Pairing};

#[tauri::command]
pub async fn load_pairing(backend: tauri::State<'_, Backend>) -> Result<serde_json::Value, String> {
    backend.request(Request::LoadPairing).await
}
#[tauri::command]
pub async fn save_pairing(
    backend: tauri::State<'_, Backend>,
    pairing: Pairing,
) -> Result<serde_json::Value, String> {
    backend.request(Request::SavePairing(pairing)).await
}
#[tauri::command]
pub async fn forget_pairing(
    backend: tauri::State<'_, Backend>,
) -> Result<serde_json::Value, String> {
    backend.request(Request::ForgetPairing).await
}
