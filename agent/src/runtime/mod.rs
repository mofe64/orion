use crate::{AgentConfig, AgentInfo, providers::codex::Codex};
use std::{thread::JoinHandle, time::Duration};
use tokio::sync::{mpsc, oneshot};

/// Cloneable in-process client. Each call has its own reply channel; dropping
/// that call cancels it without delivering its result to another caller.
#[derive(Clone)]
pub struct AgentHandle(mpsc::Sender<Request>);

impl AgentHandle {
    pub fn same_runtime(&self, other: &Self) -> bool {
        self.0.same_channel(&other.0)
    }

    async fn request(
        &self,
        text: Option<String>,
        events: Option<mpsc::Sender<crate::AgentEvent>>,
    ) -> Result<Reply, String> {
        let (reply, receive) = oneshot::channel();
        self.0
            .send(Request {
                text,
                events,
                profile: None,
                reply,
            })
            .await
            .map_err(|_| "Agent runtime stopped")?;
        receive.await.map_err(|_| "Agent request cancelled")?
    }

    pub async fn profile(
        &self,
        change: Option<crate::profile::ProfileChange>,
    ) -> Result<crate::profile::Profile, String> {
        let (reply, receive) = oneshot::channel();
        self.0
            .send(Request {
                text: None,
                events: None,
                profile: Some(change),
                reply,
            })
            .await
            .map_err(|_| "Agent runtime stopped")?;
        match receive.await.map_err(|_| "Profile request cancelled")?? {
            Reply::Profile(profile) => Ok(profile),
            _ => Err("Invalid profile response".into()),
        }
    }

    pub async fn info(&self) -> Result<AgentInfo, String> {
        match self.request(None, None).await? {
            Reply::Info(info) => Ok(info),
            _ => Err("Invalid agent status response".into()),
        }
    }

    pub async fn respond(&self, text: &str) -> Result<String, String> {
        self.respond_with_events(text, None).await
    }

    pub async fn respond_with_events(
        &self,
        text: &str,
        events: Option<mpsc::Sender<crate::AgentEvent>>,
    ) -> Result<String, String> {
        if text.trim().is_empty() || text.len() > 64 * 1024 {
            return Err("Agent input is empty or too large".into());
        }
        match self.request(Some(text.trim().into()), events).await? {
            Reply::Text(text) => Ok(text),
            _ => Err("Invalid agent text response".into()),
        }
    }
}

struct Request {
    text: Option<String>,
    events: Option<mpsc::Sender<crate::AgentEvent>>,
    profile: Option<Option<crate::profile::ProfileChange>>,
    reply: oneshot::Sender<Result<Reply, String>>,
}
enum Reply {
    Profile(crate::profile::Profile),
    Info(AgentInfo),
    Text(String),
}

/// Owns the independent agent executor and Codex child. No local network server.
/// Keep this alive when restarting a coordinator or reloading speech models.
pub struct AgentService {
    handle: AgentHandle,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl AgentService {
    pub fn start(config: AgentConfig) -> Result<Self, String> {
        if config.model.trim().is_empty() || config.effort.trim().is_empty() {
            return Err("Agent model and effort cannot be empty".into());
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let (send, mut receive) = mpsc::channel::<Request>(8);
        let (stop, mut stopped) = oneshot::channel();
        let thread = std::thread::Builder::new().name("orion-agent".into()).spawn(move || runtime.block_on(async move {
            let mut client = None;
            loop {
                let request = tokio::select! {
                    _ = &mut stopped => break,
                    request = receive.recv() => match request { Some(request) => request, None => break },
                };
                let Request { text, events, profile, mut reply } = request;
                if reply.is_closed() { continue; }
                let result = tokio::select! {
                    biased;
                    _ = &mut stopped => break,
                    _ = reply.closed() => continue,
                    result = tokio::time::timeout(Duration::from_secs(120), dispatch(&config, &mut client, text, events, profile)) =>
                        result.unwrap_or_else(|_| Err("Agent request timed out; conversation reset".into())),
                };
                let _ = reply.send(result);
            }
            if let Some(mut client) = client { client.close().await; }
        })).map_err(|e| e.to_string())?;
        Ok(Self {
            handle: AgentHandle(send),
            stop: Some(stop),
            thread: Some(thread),
        })
    }
    pub fn handle(&self) -> AgentHandle {
        self.handle.clone()
    }
}

async fn dispatch(
    config: &AgentConfig,
    slot: &mut Option<Codex>,
    text: Option<String>,
    events: Option<mpsc::Sender<crate::AgentEvent>>,
    profile: Option<Option<crate::profile::ProfileChange>>,
) -> Result<Reply, String> {
    if let Some(change) = profile {
        if let Some(change) = change {
            crate::profile::change(config, change)?;
            // Take before awaiting: cancellation cannot retain the old context.
            if let Some(mut client) = slot.take() {
                client.close().await;
            }
        }
        return crate::profile::load(config).map(Reply::Profile);
    }
    // Cancellation owns and drops an uncertain child before another turn starts.
    let mut client = match slot.take() {
        Some(client) => client,
        None => Codex::connect(config).await?,
    };
    let result = match text {
        Some(text) => client
            .respond(&text, events.as_ref())
            .await
            .map(Reply::Text),
        None => Ok(Reply::Info(client.info.clone())),
    };
    if result.is_ok() {
        *slot = Some(client);
    } else {
        client.close().await;
    }
    result
}

impl Drop for AgentService {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
