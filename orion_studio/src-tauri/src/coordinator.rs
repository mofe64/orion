use orion_coordinator::{Coordinator, CoordinatorConfig, CoordinatorConnection, SpeechConfig};
use std::{path::Path, sync::Mutex};
use tauri::State;

struct Running {
    config: CoordinatorConfig,
    model: String,
    effort: String,
    coordinator: Coordinator,
}
#[derive(Default)]
pub struct CoordinatorManager(Mutex<Option<Running>>);
impl CoordinatorManager {
    pub fn shutdown(&self) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = None;
        }
    }
}

// Retain the UI command name and observer protocol during the ownership move.
#[tauri::command]
pub fn start_voice_worker(
    manager: State<'_, CoordinatorManager>,
    agent_manager: State<'_, crate::agent::AgentManager>,
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
) -> Result<CoordinatorConnection, String> {
    let mut settings = crate::settings::load_voice_settings()?;
    if let Some(value) = agent_model {
        settings.model = value;
    }
    if let Some(value) = agent_effort {
        settings.effort = value;
    }
    if let Some(value) = asr_model {
        settings.asr_model = value;
    }
    if let Some(value) = tts_model {
        settings.tts_model = value;
    }
    if let Some(value) = asr_path {
        settings.asr_path = value;
    }
    if let Some(value) = tts_path {
        settings.tts_path = value;
    }
    if let Some(value) = cache_path {
        settings.cache_path = value;
    }
    settings.validate()?;
    let resolve = |path: &str, model: &str| -> Result<String, String> {
        if path.is_empty() {
            Ok(model.into())
        } else {
            Ok(crate::settings::expand_path(path)?
                .to_string_lossy()
                .into_owned())
        }
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../speech")
        .canonicalize()
        .map_err(|e| format!("Could not locate speech worker: {e}"))?;
    let config = CoordinatorConfig {
        pi_url: std::env::var("ORION_PI_VOICE_URL").unwrap_or(pi_url),
        pi_token,
        gateway_url,
        speech: SpeechConfig {
            python: std::env::var_os("ORION_STUDIO_VOICE_PYTHON")
                .map(Into::into)
                .unwrap_or_else(|| root.join(".venv/bin/python")),
            root,
            asr_model: resolve(&settings.asr_path, &settings.asr_model)?,
            tts_model: resolve(&settings.tts_path, &settings.tts_model)?,
            cache_path: if settings.cache_path.is_empty() {
                String::new()
            } else {
                crate::settings::expand_path(&settings.cache_path)?
                    .to_string_lossy()
                    .into_owned()
            },
        },
    };
    let mut slot = manager.0.lock().map_err(|e| e.to_string())?;
    if let Some(running) = slot.as_ref()
        && running.config == config
        && running.model == settings.model
        && running.effort == settings.effort
        && running.coordinator.is_running()
    {
        return Ok(running.coordinator.connection());
    }
    // Stop the previous Pi processing owner before attaching its replacement.
    *slot = None;
    let agent = agent_manager.ensure(&settings.model, &settings.effort)?;
    let coordinator = Coordinator::start(config.clone(), agent)?;
    let connection = coordinator.connection();
    *slot = Some(Running {
        config,
        model: settings.model,
        effort: settings.effort,
        coordinator,
    });
    Ok(connection)
}
#[tauri::command]
pub fn set_voice_microphone(
    manager: State<'_, CoordinatorManager>,
    muted: bool,
) -> Result<serde_json::Value, String> {
    manager
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .ok_or("Open paired Studio before changing the microphone")?
        .coordinator
        .set_microphone(muted)
}
