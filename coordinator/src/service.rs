use crate::{SpeechConfig, hub::Hub, pipeline, speech::SpeechRuntime};
use orion_agent::AgentHandle;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{net::TcpListener, sync::Arc, thread::JoinHandle, time::Duration};
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct CoordinatorConfig {
    pub pi_url: String,
    pub pi_token: String,
    pub gateway_url: String,
    pub speech: SpeechConfig,
}
impl CoordinatorConfig {
    fn validate(&self) -> Result<(), String> {
        for (raw, schemes) in [
            (&self.pi_url, &["ws"][..]),
            (&self.gateway_url, &["http", "https"][..]),
        ] {
            let url = url::Url::parse(raw).map_err(|e| e.to_string())?;
            if !schemes.contains(&url.scheme())
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err("Invalid coordinator endpoint or embedded credentials".into());
            }
        }
        if self.pi_token.len() < 32 {
            return Err("A paired Pi is required".into());
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoordinatorConnection {
    pub url: String,
    pub token: String,
    pub asr_model: String,
}
struct Control {
    muted: bool,
    reply: std::sync::mpsc::Sender<Result<Value, String>>,
}
pub struct Coordinator {
    connection: CoordinatorConnection,
    control: mpsc::Sender<Control>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl Coordinator {
    pub fn start(config: CoordinatorConfig, agent: AgentHandle) -> Result<Self, String> {
        config.validate()?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let connection = CoordinatorConnection {
            url: format!(
                "ws://127.0.0.1:{}",
                listener.local_addr().map_err(|e| e.to_string())?.port()
            ),
            token: uuid::Uuid::new_v4().simple().to_string(),
            asr_model: config.speech.asr_model.clone(),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let (stop, mut stopped) = oneshot::channel();
        let (control, mut controls) = mpsc::channel::<Control>(8);
        let token = connection.token.clone();
        let thread = std::thread::Builder::new().name("orion-coordinator".into()).spawn(move || runtime.block_on(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(listener) else { return; };
            let hub = Hub::new();
            let speech = Arc::new(SpeechRuntime::new(config.speech.clone()));
            let (shutdown, shutdown_rx) = watch::channel(false);
            let mut processing = tokio::spawn(pipeline::run(config.clone(), agent, speech.clone(), hub.clone(), shutdown_rx));
            let mut tasks = tokio::task::JoinSet::new();
            let monitor_config = config.clone(); let monitor_hub = hub.clone();
            tasks.spawn(async move {
                let mut previous = Value::Null;
                loop {
                    if let Ok(status) = pipeline::microphone(&monitor_config, None).await && status != previous {
                        previous = status.clone(); monitor_hub.publish(status);
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
            loop {
                tokio::select! {
                    _ = &mut stopped => break,
                    _ = &mut processing => { hub.publish(pipeline::error("coordinator_stopped", "Voice coordinator stopped", false)); break; },
                    Some(_) = tasks.join_next() => {},
                    request = controls.recv() => {
                        let Some(request) = request else { break; };
                        let config = config.clone(); let hub = hub.clone();
                        tasks.spawn(async move {
                            let result = pipeline::microphone(&config, Some(request.muted)).await;
                            if let Ok(status) = &result { hub.publish(status.clone()); }
                            let _ = request.reply.send(result);
                        });
                    },
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break; };
                        if tasks.len() >= 16 { continue; }
                        let hub = hub.clone(); let token = token.clone(); let config = config.clone();
                        tasks.spawn(async move { let _ = hub.attach(stream, &token, &config).await; });
                    }
                }
            }
            let _ = shutdown.send(true);
            if !processing.is_finished() && tokio::time::timeout(Duration::from_secs(7), &mut processing).await.is_err() {
                processing.abort(); let _ = processing.await;
            }
            tasks.abort_all(); while tasks.join_next().await.is_some() {}
            speech.close().await;
        })).map_err(|e| e.to_string())?;
        Ok(Self {
            connection,
            control,
            stop: Some(stop),
            thread: Some(thread),
        })
    }
    pub fn connection(&self) -> CoordinatorConnection {
        self.connection.clone()
    }
    pub fn is_running(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }
    pub fn set_microphone(&self, muted: bool) -> Result<Value, String> {
        let (reply, receive) = std::sync::mpsc::channel();
        self.control
            .try_send(Control { muted, reply })
            .map_err(|_| "Coordinator is busy or stopped")?;
        receive
            .recv_timeout(Duration::from_secs(12))
            .map_err(|_| "Microphone control timed out")?
    }
}
impl Drop for Coordinator {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
