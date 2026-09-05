use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
};
use tauri::State;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceWorkerConnection {
    pub url: String,
    pub token: String,
    pub asr_model: String,
}
struct RunningWorker {
    child: Child,
    config: serde_json::Value,
    connection: VoiceWorkerConnection,
}
impl Drop for RunningWorker {
    fn drop(&mut self) {
        self.child.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[derive(Default)]
pub struct VoiceWorkerManager {
    worker: Mutex<Option<RunningWorker>>,
}
impl VoiceWorkerManager {
    pub fn shutdown(&self) {
        if let Ok(mut slot) = self.worker.lock() {
            *slot = None;
        }
    }
}
fn worker_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("voice_worker"))
        .filter(|p| p.is_dir())
        .ok_or_else(|| "Could not locate voice_worker.".into())
}
fn python(root: &Path) -> PathBuf {
    std::env::var_os("ORION_STUDIO_VOICE_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".venv/bin/python"))
}
fn config_path() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(|p| PathBuf::from(p).join(".config/orion/voice-settings.json"))
        .ok_or_else(|| "HOME is unavailable".into())
}
#[tauri::command]
pub fn load_voice_settings() -> Result<serde_json::Value, String> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(serde_json::json!({"model":"gpt-5.6-sol", "effort":"medium"}));
    }
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"model":config["agent_model"], "effort":config["agent_effort"]}))
}
#[tauri::command]
pub fn start_voice_worker(
    manager: State<'_, VoiceWorkerManager>,
    pi_url: String,
    pi_token: String,
    gateway_url: String,
    agent_model: Option<String>,
    agent_effort: Option<String>,
) -> Result<VoiceWorkerConnection, String> {
    let mut slot = manager.worker.lock().map_err(|e| e.to_string())?;
    let root = worker_root()?;
    let mut values = serde_json::json!({"pi_url":pi_url, "pi_token":pi_token, "gateway_url":gateway_url,
        "agent_model":agent_model.unwrap_or_else(|| "gpt-5.6-sol".into()),
        "agent_effort":agent_effort.unwrap_or_else(|| "medium".into()),
        "asr_model":"Qwen/Qwen3-ASR-0.6B", "tts_model":"mlx-community/chatterbox-turbo-8bit", "agent_provider":"codex"});
    for (environment, field) in [
        ("ORION_PI_VOICE_URL", "pi_url"),
        ("ORION_STUDIO_ASR_MODEL", "asr_model"),
        ("ORION_STUDIO_TTS_MODEL", "tts_model"),
        ("ORION_STUDIO_AGENT_PROVIDER", "agent_provider"),
    ] {
        if let Ok(value) = std::env::var(environment) {
            values[field] = value.into();
        }
    }
    if values["pi_token"].as_str().unwrap_or("").len() < 32
        || values["agent_model"]
            .as_str()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        return Err("A paired Pi and a reply model are required.".into());
    }
    if let Some(worker) = slot.as_mut() {
        if worker.config == values
            && worker
                .child
                .try_wait()
                .map_err(|e| e.to_string())?
                .is_none()
        {
            return Ok(worker.connection.clone());
        }
    }
    let settings_path = config_path()?;
    std::fs::create_dir_all(settings_path.parent().ok_or("Invalid settings path")?)
        .map_err(|e| e.to_string())?;
    std::fs::write(&settings_path, serde_json::json!({"agent_model":values["agent_model"], "agent_effort":values["agent_effort"]}).to_string()).map_err(|e| e.to_string())?;
    *slot = None; // Retire the previous child before starting another owner.
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    drop(listener);
    let connection = VoiceWorkerConnection {
        url: format!("ws://127.0.0.1:{port}"),
        token: uuid::Uuid::new_v4().simple().to_string(),
        asr_model: values["asr_model"].as_str().unwrap().into(),
    };
    let mut launch = values.clone();
    launch["token"] = connection.token.clone().into();
    launch["port"] = port.into();
    let mut child = Command::new(python(&root))
        .args(["-m", "orion_voice_worker.processor"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("Could not start Studio voice: {e}"))?;
    if let Err(error) = child
        .stdin
        .as_mut()
        .ok_or("Worker input unavailable")?
        .write_all(format!("{launch}\n").as_bytes())
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.to_string());
    }
    *slot = Some(RunningWorker {
        child,
        config: values,
        connection: connection.clone(),
    });
    Ok(connection)
}
#[tauri::command]
pub fn set_voice_microphone(
    manager: State<'_, VoiceWorkerManager>,
    muted: bool,
) -> Result<serde_json::Value, String> {
    let config = manager
        .worker
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .map(|worker| worker.config.clone())
        .ok_or("Open paired Studio before changing the microphone")?;
    let root = worker_root()?;
    let mut child = Command::new(python(&root))
        .args([
            "-m",
            "orion_voice_worker.processor",
            "--mute",
            if muted { "on" } else { "off" },
        ])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("Worker input unavailable")?
        .write_all(format!("{config}\n").as_bytes())
        .map_err(|e| e.to_string())?;
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
}
