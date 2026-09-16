use orion_service::{Backend, Request, StartOptions};
use tauri::State;

// Retain the UI command name and observer protocol during the ownership move.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Preserve the existing named Tauri command arguments.
pub async fn start_voice_worker(
    backend: State<'_, Backend>,
    pi_url: String,
    pi_token: String,
    gateway_url: String,
    agent_model: Option<String>,
    agent_effort: Option<String>,
    asr_model: Option<String>,
    tts_model: Option<String>,
    asr_path: Option<String>,
    tts_path: Option<String>,
    cache_path: Option<String>,
) -> Result<serde_json::Value, String> {
    backend
        .request(Request::Start(StartOptions {
            pi_url,
            pi_token,
            gateway_url,
            agent_model,
            agent_effort,
            asr_model,
            tts_model,
            asr_path,
            tts_path,
            cache_path,
        }))
        .await
}
#[tauri::command]
pub async fn set_voice_microphone(
    backend: State<'_, Backend>,
    muted: bool,
) -> Result<serde_json::Value, String> {
    backend.request(Request::Microphone { muted }).await
}
