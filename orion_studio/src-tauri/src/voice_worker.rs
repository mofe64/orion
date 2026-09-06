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
#[tauri::command]
pub fn start_voice_worker(
    manager: State<'_, VoiceWorkerManager>,
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
) -> Result<VoiceWorkerConnection, String> {
    let mut slot = manager.worker.lock().map_err(|e| e.to_string())?;
    let root = worker_root()?;
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
    let values = serde_json::json!({"pi_url":std::env::var("ORION_PI_VOICE_URL").unwrap_or(pi_url), "pi_token":pi_token, "gateway_url":gateway_url,
        "agent_model":settings.model,"agent_effort":settings.effort,
        "asr_model":resolve(&settings.asr_path,&settings.asr_model)?, "tts_model":resolve(&settings.tts_path,&settings.tts_model)?, "agent_provider":"codex", "cache_path":settings.cache_path});
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
    let mut command = Command::new(python(&root));
    if !settings.cache_path.is_empty() {
        let cache = crate::settings::expand_path(&settings.cache_path)?;
        command
            .env("HF_HOME", &cache)
            .env("HF_HUB_CACHE", cache.join("hub"));
    }
    let mut child = command
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
