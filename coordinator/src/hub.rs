use crate::{CoordinatorConfig, pipeline::microphone};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpStream, sync::broadcast};
use tokio_tungstenite::{
    accept_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

#[derive(Clone)]
pub(crate) struct Hub {
    state: Arc<Mutex<State>>,
    events: broadcast::Sender<String>,
}
#[derive(Default)]
struct State {
    ready: Option<Value>,
    replay: VecDeque<String>,
}
impl Hub {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            events: broadcast::channel(64).0,
        }
    }
    pub fn publish(&self, value: Value) {
        let mut state = self.state.lock().unwrap();
        if value["type"] == "ready" {
            state.ready = Some(value.clone());
            state.replay.clear();
        } else {
            if value["type"] == "wake.candidate" {
                state.replay.clear();
            }
            if value["type"] == "microphone.status"
                && let Some(ready) = state.ready.as_mut()
            {
                ready["muted"] = value["muted"].clone();
            }
            state.replay.push_back(value.to_string());
            while state.replay.len() > 32 {
                state.replay.pop_front();
            }
        }
        let _ = self.events.send(value.to_string());
    }
    fn snapshot(&self) -> Vec<String> {
        let state = self.state.lock().unwrap();
        state
            .ready
            .iter()
            .map(Value::to_string)
            .chain(state.replay.iter().cloned())
            .collect()
    }
    pub async fn attach(
        &self,
        stream: TcpStream,
        token: &str,
        config: &CoordinatorConfig,
    ) -> Result<(), String> {
        let options = WebSocketConfig::default()
            .max_message_size(Some(4096))
            .max_frame_size(Some(4096));
        let mut ws = tokio::time::timeout(
            Duration::from_secs(5),
            accept_async_with_config(stream, Some(options)),
        )
        .await
        .map_err(|_| "Observer handshake timed out")?
        .map_err(|e| e.to_string())?;
        let hello = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .map_err(|_| "Observer hello timed out")?
            .ok_or("Observer closed")?
            .map_err(|e| e.to_string())?;
        let hello: Value = serde_json::from_str(hello.to_text().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if hello["type"] != "hello" || hello["protocol"] != 7 || hello["token"] != token {
            return Err("Invalid observer credentials".into());
        }
        let mut events = self.events.subscribe();
        let (mut output, mut input) = ws.split();
        for raw in self.snapshot() {
            output
                .send(Message::text(raw))
                .await
                .map_err(|e| e.to_string())?;
        }
        loop {
            tokio::select! {
                message = events.recv() => {
                    let messages = match message { Ok(raw) => vec![raw], Err(broadcast::error::RecvError::Lagged(_)) => self.snapshot(), Err(_) => return Ok(()) };
                    for raw in messages {
                        tokio::time::timeout(Duration::from_secs(5), output.send(Message::text(raw))).await
                            .map_err(|_| "Slow observer")?.map_err(|e| e.to_string())?;
                    }
                },
                message = input.next() => {
                    let Some(message) = message else { return Ok(()); };
                    match message.map_err(|e| e.to_string())? {
                        Message::Text(raw) => {
                            let value: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
                            if value["type"] == "stop" { return Ok(()); }
                            if value["type"] == "microphone.mute" && let Some(muted) = value["muted"].as_bool() {
                                self.publish(microphone(config, Some(muted)).await?);
                            } else { return Err("UI cannot own playback".into()); }
                        },
                        Message::Close(_) => return Ok(()),
                        Message::Ping(data) => { output.send(Message::Pong(data)).await.map_err(|e| e.to_string())?; },
                        Message::Pong(_) => {},
                        _ => return Err("Observer cannot submit audio".into()),
                    }
                }
            }
        }
    }
}
