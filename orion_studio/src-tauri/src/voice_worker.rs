use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
};

use serde::Serialize;
use tauri::State;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceWorkerConnection {
    pub url: String,
    pub token: String,
    pub asr_model: String,
}

struct RunningWorker {
    child: Child,
    connection: VoiceWorkerConnection,
}

#[derive(Default)]
pub struct VoiceWorkerManager {
    worker: Mutex<Option<RunningWorker>>,
}

impl Drop for VoiceWorkerManager {
    fn drop(&mut self) {
        if let Ok(worker) = self.worker.get_mut()
            && let Some(mut worker) = worker.take()
        {
            let _ = worker.child.kill();
            let _ = worker.child.wait();
        }
    }
}

fn worker_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|path| path.join("voice_worker"))
        .filter(|path| path.is_dir())
        .ok_or_else(|| "Could not locate Orion Studio's voice_worker directory.".to_owned())
}

fn configured_python(root: &Path) -> Result<PathBuf, String> {
    let path = std::env::var_os("ORION_STUDIO_VOICE_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(windows) {
                root.join(".venv").join("Scripts").join("python.exe")
            } else {
                root.join(".venv").join("bin").join("python")
            }
        });
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "Voice worker Python was not found at {}. Create voice_worker/.venv and install its pyproject first.",
            path.display()
        ))
    }
}

#[tauri::command]
pub fn start_voice_worker(
    manager: State<'_, VoiceWorkerManager>,
) -> Result<VoiceWorkerConnection, String> {
    let mut slot = manager
        .worker
        .lock()
        .map_err(|_| "Voice worker state is unavailable.".to_owned())?;

    if let Some(worker) = slot.as_mut() {
        if worker
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Ok(worker.connection.clone());
        }
        *slot = None;
    }

    let root = worker_root()?;
    let python = configured_python(&root)?;
    let asr_model = std::env::var("ORION_STUDIO_ASR_MODEL")
        .unwrap_or_else(|_| "Qwen/Qwen3-ASR-0.6B".to_owned());
    let agent_provider =
        std::env::var("ORION_STUDIO_AGENT_PROVIDER").unwrap_or_else(|_| "codex".to_owned());
    let agent_model = std::env::var("ORION_STUDIO_AGENT_MODEL").ok();
    let tts_model = std::env::var("ORION_STUDIO_TTS_MODEL")
        .unwrap_or_else(|_| "mlx-community/chatterbox-turbo-8bit".to_owned());
    let wake_model = std::env::var_os("ORION_STUDIO_WAKE_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("models").join("hey_orion_reference.rpw"));
    if !wake_model.is_file() {
        return Err(format!(
            "The commissioned Rustpotter wake reference was not found at {}.",
            wake_model.display()
        ));
    }
    let wake_threshold =
        std::env::var("ORION_STUDIO_WAKE_THRESHOLD").unwrap_or_else(|_| "0.400".to_owned());
    let parsed_threshold = wake_threshold
        .parse::<f32>()
        .map_err(|_| "ORION_STUDIO_WAKE_THRESHOLD must be a number.".to_owned())?;
    if !(0.0 < parsed_threshold && parsed_threshold <= 1.0) {
        return Err("ORION_STUDIO_WAKE_THRESHOLD must be in the range (0, 1].".to_owned());
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("Could not reserve a local voice-worker port: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    drop(listener);

    let token = uuid::Uuid::new_v4().simple().to_string();
    let wake_model = wake_model
        .to_str()
        .ok_or_else(|| "The wake-model path is not valid UTF-8.".to_owned())?;
    let mut arguments = vec![
        "-m".to_owned(),
        "orion_voice_worker.server".to_owned(),
        "--host".to_owned(),
        "127.0.0.1".to_owned(),
        "--port".to_owned(),
        port.to_string(),
        "--token".to_owned(),
        token.clone(),
        "--asr-model".to_owned(),
        asr_model.clone(),
        "--wake-model".to_owned(),
        wake_model.to_owned(),
        "--wake-threshold".to_owned(),
        wake_threshold,
        "--agent-provider".to_owned(),
        agent_provider,
        "--tts-model".to_owned(),
        tts_model,
    ];
    if let Some(agent_model) = agent_model {
        arguments.extend(["--agent-model".to_owned(), agent_model]);
    }
    let child = Command::new(&python)
        .args(arguments)
        .current_dir(&root)
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("Could not start the Studio voice worker: {error}"))?;

    let connection = VoiceWorkerConnection {
        url: format!("ws://127.0.0.1:{port}"),
        token,
        asr_model,
    };
    *slot = Some(RunningWorker {
        child,
        connection: connection.clone(),
    });
    Ok(connection)
}

#[tauri::command]
pub fn stop_voice_worker(manager: State<'_, VoiceWorkerManager>) -> Result<(), String> {
    let mut slot = manager
        .worker
        .lock()
        .map_err(|_| "Voice worker state is unavailable.".to_owned())?;
    if let Some(mut worker) = slot.take() {
        worker
            .child
            .kill()
            .map_err(|error| format!("Could not stop the voice worker: {error}"))?;
        let _ = worker.child.wait();
    }
    Ok(())
}
