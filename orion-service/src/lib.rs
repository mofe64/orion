pub mod agent;
pub mod coordinator;
pub mod pairing;
mod remote;
pub mod rpc;
pub mod settings;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Mutex};

pub fn project_root() -> Result<PathBuf, String> {
    let root = std::env::var_os("ORION_PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));
    root.canonicalize()
        .map_err(|e| format!("Could not locate Orion project: {e}"))
}

pub fn onboard() -> bool {
    std::env::var("ORION_ONBOARD").as_deref() == Ok("1")
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartOptions {
    pub pi_url: String,
    pub pi_token: String,
    pub gateway_url: String,
    pub agent_model: Option<String>,
    pub agent_effort: Option<String>,
    pub asr_model: Option<String>,
    pub tts_model: Option<String>,
    pub asr_path: Option<String>,
    pub tts_path: Option<String>,
    pub cache_path: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "method",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Request {
    Status,
    Observe,
    StartSaved,
    Start(StartOptions),
    Microphone { muted: bool },
    LoadPairing,
    SavePairing(pairing::Pairing),
    ForgetPairing,
    LoadSettings,
    SaveSettings(settings::VoiceSettings),
    ModelLocations(settings::VoiceSettings),
    Profile(Option<orion_agent::profile::ProfileChange>),
}

pub struct Host {
    coordinator: coordinator::CoordinatorManager,
    agent: agent::AgentManager,
    desired: Mutex<Option<StartOptions>>,
    lifecycle: tokio::sync::Mutex<()>,
    error: Mutex<Option<String>>,
    _owner: rpc::Owner,
}
impl Host {
    pub fn new(directory: &std::path::Path) -> Result<Self, String> {
        Ok(Self {
            coordinator: Default::default(),
            agent: Default::default(),
            desired: Default::default(),
            lifecycle: Default::default(),
            error: Default::default(),
            _owner: rpc::Owner::acquire(directory)?,
        })
    }
    pub fn shutdown(&self) {
        self.coordinator.shutdown();
        self.agent.shutdown();
    }
    fn start(&self, options: StartOptions) -> Result<Value, String> {
        let result =
            coordinator::start_voice_worker(&self.coordinator, &self.agent, options.clone())?;
        *self.desired.lock().map_err(|_| "Service unavailable")? = Some(options);
        *self.error.lock().map_err(|_| "Service unavailable")? = None;
        serde_json::to_value(result).map_err(|e| e.to_string())
    }
    pub async fn maintain(&self) {
        let _guard = self.lifecycle.lock().await;
        if self.coordinator.is_running() {
            return;
        }
        let desired = self.desired.lock().unwrap().clone();
        let result = match desired {
            Some(options) => self.start(options),
            None => self.start_saved().await,
        };
        let error = result.err();
        let mut previous = self.error.lock().unwrap();
        if *previous != error {
            if let Some(message) = &error {
                eprintln!("orion-studio: {message}");
            }
            *previous = error;
        }
    }
    async fn start_saved(&self) -> Result<Value, String> {
        if onboard() {
            let token_path = PathBuf::from(std::env::var_os("HOME").ok_or("Home is unavailable")?)
                .join(".config/orion/studio-token");
            return self.start(StartOptions {
                pi_url: "ws://127.0.0.1:7448".into(),
                gateway_url: "http://127.0.0.1:7447".into(),
                pi_token: std::fs::read_to_string(token_path)
                    .map_err(|e| e.to_string())?
                    .trim()
                    .into(),
                ..Default::default()
            });
        }
        Err("Automatic voice startup is only supported on the Pi (ORION_ONBOARD=1)".into())
    }

    pub async fn dispatch(&self, request: Request) -> Result<Value, String> {
        let _guard = if matches!(
            &request,
            Request::Start(_)
                | Request::StartSaved
                | Request::SavePairing(_)
                | Request::ForgetPairing
                | Request::SaveSettings(_)
        ) {
            Some(self.lifecycle.lock().await)
        } else {
            None
        };
        match request {
            Request::Status => Ok(json!({"protocol": rpc::PROTOCOL, "pid": std::process::id(),
                "onboard": onboard(),
                "coordinator_running": self.coordinator.is_running(),
                "error": *self.error.lock().map_err(|_| "Service unavailable")?,
                "project_root": project_root()?,
                "revision": std::env::var("ORION_RELEASE_REVISION").unwrap_or_else(|_| "development".into())})),
            Request::Start(options) => self.start(options),
            Request::StartSaved => self.start_saved().await,
            Request::Observe => Ok(self.coordinator.events()),
            Request::Microphone { muted } => {
                coordinator::set_voice_microphone(&self.coordinator, muted)
            }
            Request::LoadPairing => {
                serde_json::to_value(pairing::load_pairing().await?).map_err(|e| e.to_string())
            }
            Request::SavePairing(pairing) => {
                pairing::save_pairing(pairing).await?;
                self.coordinator.shutdown();
                *self.desired.lock().unwrap() = None;
                if let Err(error) = self.start_saved().await {
                    *self.error.lock().unwrap() = Some(error);
                }
                Ok(Value::Null)
            }
            Request::ForgetPairing => {
                pairing::forget_pairing().await?;
                self.shutdown();
                *self.desired.lock().unwrap() = None;
                Ok(Value::Null)
            }
            Request::LoadSettings => {
                serde_json::to_value(settings::load_voice_settings()?).map_err(|e| e.to_string())
            }
            Request::SaveSettings(settings) => {
                let mut previous = settings::load_voice_settings()?;
                previous.tts_voice = settings.tts_voice.clone();
                let voice_only = previous == settings;
                let settings = settings::save_voice_settings(settings)?;
                if voice_only {
                    self.coordinator.set_voice(&settings.tts_voice);
                    return serde_json::to_value(settings).map_err(|e| e.to_string());
                }
                self.coordinator.shutdown();
                *self.desired.lock().unwrap() = None;
                if let Err(error) = self.start_saved().await {
                    *self.error.lock().unwrap() = Some(error);
                }
                serde_json::to_value(settings).map_err(|e| e.to_string())
            }
            Request::ModelLocations(settings) => {
                serde_json::to_value(settings::voice_model_locations(settings)?)
                    .map_err(|e| e.to_string())
            }
            Request::Profile(change) => {
                serde_json::to_value(self.agent.profile_handle()?.profile(change).await?)
                    .map_err(|e| e.to_string())
            }
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Desktop Studio is a client of the paired Pi; it never owns local inference.
#[derive(Default)]
pub struct Backend;
impl Backend {
    pub async fn request(&self, request: Request) -> Result<Value, String> {
        match request {
            Request::LoadPairing => {
                serde_json::to_value(pairing::load_pairing().await?).map_err(|e| e.to_string())
            }
            Request::SavePairing(value) => {
                pairing::save_pairing(value).await?;
                Ok(Value::Null)
            }
            Request::ForgetPairing => {
                pairing::forget_pairing().await?;
                Ok(Value::Null)
            }
            request => remote::request(&request).await,
        }
    }
}
