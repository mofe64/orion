pub mod agent;
pub mod coordinator;
pub mod pairing;
pub mod rpc;
pub mod settings;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub fn project_root() -> Result<PathBuf, String> {
    let root = std::env::var_os("ORION_PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));
    root.canonicalize()
        .map_err(|e| format!("Could not locate Orion project: {e}"))
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
        let pairing = pairing::load_pairing()
            .await?
            .ok_or("Pair Orion in Studio to enable voice")?;
        let mut url = url::Url::parse(&pairing.url).map_err(|_| "Invalid saved gateway")?;
        url.set_scheme("ws").map_err(|_| "Invalid voice URL")?;
        url.set_port(Some(7448)).map_err(|_| "Invalid voice port")?;
        self.start(StartOptions {
            pi_url: url.to_string(),
            gateway_url: pairing.url,
            pi_token: pairing.token,
            ..Default::default()
        })
    }
    pub async fn dispatch(&self, request: Request) -> Result<Value, String> {
        let _guard = if matches!(
            &request,
            Request::Start(_)
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
                "coordinator_running": self.coordinator.is_running(),
                "error": *self.error.lock().map_err(|_| "Service unavailable")?,
                "project_root": project_root()?,
                "revision": std::env::var("ORION_RELEASE_REVISION").unwrap_or_else(|_| "development".into())})),
            Request::Start(options) => self.start(options),
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
                let settings = settings::save_voice_settings(settings)?;
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

/// Studio attaches to the installed owner; an uninstalled desktop may own its own host.
#[derive(Default)]
pub struct Backend {
    local: Mutex<Option<Arc<Host>>>,
    directory: Option<PathBuf>,
}
impl Backend {
    /// Select an isolated owner directory for an embedding or integration harness.
    pub fn at(directory: PathBuf) -> Self {
        Self {
            directory: Some(directory),
            ..Default::default()
        }
    }
    pub async fn request(&self, request: Request) -> Result<Value, String> {
        let directory = self
            .directory
            .clone()
            .map(Ok)
            .unwrap_or_else(rpc::service_home)?;
        if directory.join("installed").exists() || directory.join("connection.json").exists() {
            // Surface service errors to the UI; never create a second owner as a retry.
            return tokio::task::spawn_blocking(move || rpc::call(&directory, &request))
                .await
                .map_err(|_| "Background service request failed")?;
        }
        let host = {
            let mut local = self
                .local
                .lock()
                .map_err(|_| "Studio service unavailable")?;
            if local.is_none() {
                *local = Some(Arc::new(Host::new(&directory)?));
            }
            local.as_ref().unwrap().clone()
        };
        host.dispatch(request).await
    }
    pub fn shutdown(&self) {
        if let Ok(mut host) = self.local.lock() {
            *host = None;
        }
    }
}
