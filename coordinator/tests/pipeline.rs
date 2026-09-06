use futures_util::{SinkExt, StreamExt};
use orion_agent::{AgentConfig, AgentService};
use orion_coordinator::{Coordinator, CoordinatorConfig, SpeechConfig};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, mpsc},
    task::JoinSet,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, accept_async, connect_async, tungstenite::Message,
};

type PiSocket = WebSocketStream<TcpStream>;
type Observer = WebSocketStream<MaybeTlsStream<TcpStream>>;
const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NEXT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
#[derive(Default)]
struct GatewayState {
    uploads: Vec<(String, String, Vec<u8>)>,
    ended: bool,
    cancellations: Vec<u64>,
    allow_complete: bool,
    runs: u64,
    fail_playback: bool,
}
struct Harness {
    coordinator: Option<Coordinator>,
    agent: AgentService,
    pi: PiSocket,
    observer: Observer,
    gateway: Arc<Mutex<GatewayState>>,
    receive_pi: mpsc::Receiver<PiSocket>,
    tasks: JoinSet<()>,
}
async fn send<S>(socket: &mut WebSocketStream<S>, value: Value)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    socket.send(Message::text(value.to_string())).await.unwrap();
}
async fn next<S>(socket: &mut WebSocketStream<S>) -> Value
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(raw) = message {
            return serde_json::from_str(&raw).unwrap();
        }
    }
}
async fn until<S>(socket: &mut WebSocketStream<S>, kind: &str) -> Value
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let value = next(socket).await;
        if value["type"] == kind {
            return value;
        }
        assert_ne!(value["type"], "worker.error", "{value}");
    }
}
impl Harness {
    async fn new() -> Self {
        let mut tasks = JoinSet::new();
        let gateway = Arc::new(Mutex::new(GatewayState {
            allow_complete: true,
            ..Default::default()
        }));
        let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gateway_url = format!("http://{}", http.local_addr().unwrap());
        let state = gateway.clone();
        tasks.spawn(async move {
            let mut handlers = JoinSet::new();
            loop {
                tokio::select! {
                    connection = http.accept() => {
                        let (stream, _) = connection.unwrap(); let state = state.clone();
                        handlers.spawn(async move { http_request(stream, state).await; });
                    },
                    Some(_) = handlers.join_next(), if !handlers.is_empty() => {},
                }
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let pi_url = format!("ws://{}", listener.local_addr().unwrap());
        let (send_pi, mut receive_pi) = mpsc::channel(1);
        tasks.spawn(async move {
            let mut handlers = JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let (stream, _) = connection.unwrap(); let send_pi = send_pi.clone();
                        handlers.spawn(async move {
                            let mut socket = accept_async(stream).await.unwrap();
                            let hello = next(&mut socket).await;
                            assert_eq!(hello["token"], TOKEN);
                            if hello["role"] == "control" {
                                send(&mut socket, json!({"type":"microphone.status","muted":false})).await;
                                if let Some(Ok(Message::Text(raw))) = socket.next().await {
                                    let value:Value = serde_json::from_str(&raw).unwrap();
                                    send(&mut socket,json!({"type":"microphone.status","muted":value["muted"]})).await;
                                }
                            } else { send_pi.send(socket).await.unwrap(); }
                        });
                    },
                    Some(_) = handlers.join_next(), if !handlers.is_empty() => {},
                }
            }
        });
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let agent = AgentService::start(AgentConfig {
            model: "test-model".into(),
            effort: "high".into(),
            codex_bin: Some(root.join("../agent/tests/fixtures/codex.py")),
        })
        .unwrap();
        let coordinator = Coordinator::start(
            CoordinatorConfig {
                pi_url,
                pi_token: TOKEN.into(),
                gateway_url,
                speech: SpeechConfig {
                    python: "python3".into(),
                    root: root.join("tests/fixtures"),
                    asr_model: "fixture".into(),
                    tts_model: "fixture".into(),
                    cache_path: String::new(),
                },
            },
            agent.handle(),
        )
        .unwrap();
        let connection = coordinator.connection();
        let mut pi = tokio::time::timeout(Duration::from_secs(5), receive_pi.recv())
            .await
            .unwrap()
            .unwrap();
        send(&mut pi, json!({"type":"ready", "protocol":1, "sampleRate":16000, "channels":1, "encoding":"pcm_s16le", "muted":false,
            "conversationWindow":true, "wake":{"provider":"rustpotter","model":"pi.rpw","threshold":0.4}})).await;
        let (mut observer, _) = connect_async(&connection.url).await.unwrap();
        send(
            &mut observer,
            json!({"type":"hello","protocol":7,"token":connection.token}),
        )
        .await;
        until(&mut observer, "ready").await;
        Self {
            coordinator: Some(coordinator),
            agent,
            pi,
            observer,
            gateway,
            receive_pi,
            tasks,
        }
    }
    async fn utterance(&mut self, sid: &str, purpose: &str, text: &str) {
        let mut pcm = text.as_bytes().to_vec();
        if !pcm.len().is_multiple_of(2) {
            pcm.push(0);
        }
        send(
            &mut self.pi,
            json!({"type":"utterance", "sessionId":sid, "purpose":purpose, "bytes":pcm.len()}),
        )
        .await;
        self.pi.send(Message::Binary(pcm.into())).await.unwrap();
    }
    async fn wake(&mut self, text: &str) {
        send(
            &mut self.pi,
            json!({"type":"wake.candidate", "sessionId":SID, "name":"hey_orion", "score":0.8}),
        )
        .await;
        self.utterance(SID, "wake_and_command", text).await;
    }
    async fn stop(&mut self) {
        let coordinator = self.coordinator.take();
        // Let fake Pi/gateway handlers run while synchronous Drop waits for cleanup.
        tokio::task::spawn_blocking(move || drop(coordinator))
            .await
            .unwrap();
        self.tasks.abort_all();
        while self.tasks.join_next().await.is_some() {}
    }
}
async fn http_request(mut socket: TcpStream, state: Arc<Mutex<GatewayState>>) {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        if socket.read_exact(&mut byte).await.is_err() {
            return;
        }
        header.push(byte[0]);
    }
    let header = String::from_utf8(header).unwrap();
    let mut lines = header.lines();
    let path = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    let headers: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.to_lowercase(), v.trim().to_owned()))
        .collect();
    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "authorization" && v == &format!("Bearer {TOKEN}"))
    );
    let size = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .map(|(_, v)| v.parse().unwrap())
        .unwrap_or(0);
    let mut body = vec![0; size];
    socket.read_exact(&mut body).await.unwrap();
    let mut state = state.lock().await;
    let value = if path == "/api/v2/operations" {
        let value: Value = serde_json::from_slice(&body).unwrap();
        state.cancellations.push(value["run_id"].as_u64().unwrap());
        json!({"ok":true})
    } else if path == "/api/v2/speech/stream" || path.contains("/chunks/") {
        let request_id = headers
            .iter()
            .find(|(k, _)| k == "x-orion-voice-request-id")
            .unwrap()
            .1
            .clone();
        assert_eq!(&body[..4], b"RIFF");
        if path.ends_with("/stream") {
            state.runs += 1;
            state.ended = false;
        }
        state.uploads.push((path, request_id, body));
        json!({"run_id":state.runs})
    } else if path.ends_with("/end") {
        state.ended = true;
        json!({"ok":true})
    } else {
        json!({"state":if state.fail_playback { "failed" } else if state.ended && state.allow_complete {"completed"} else {"playing"}, "first_playback_ms":25})
    };
    drop(state);
    let body = value.to_string();
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
}

#[tokio::test]
async fn followup_keeps_agent_context_but_uses_new_audio_and_upload_identity() {
    let mut h = Harness::new().await;
    let conversation = h.agent.handle().info().await.unwrap().conversation_id;
    h.wake("Hey Orion, tell me about Mars").await;
    let first = until(&mut h.pi, "session.finish").await;
    assert_eq!(first["sessionId"], SID);
    assert_eq!(first["conversationWindow"], true);
    send(
        &mut h.pi,
        json!({"type":"conversation.ready","sessionId":SID}),
    )
    .await;
    send(
        &mut h.pi,
        json!({"type":"command.candidate","sessionId":NEXT,"previousSessionId":SID}),
    )
    .await;
    h.utterance(NEXT, "command", "How far away is it?").await;
    assert_eq!(until(&mut h.pi, "session.finish").await["sessionId"], NEXT);
    let mut replies = Vec::new();
    while replies.len() < 2 {
        let message = next(&mut h.observer).await;
        if message["type"] == "agent.response" {
            replies.push(message);
        }
    }
    assert_eq!(replies[0]["text"], "Reply 1: tell me about Mars");
    assert_eq!(replies[1]["text"], "Reply 2: How far away is it?");
    assert_ne!(replies[0]["requestId"], replies[1]["requestId"]);
    assert_eq!(
        h.agent.handle().info().await.unwrap().conversation_id,
        conversation
    );
    let state = h.gateway.lock().await;
    assert_eq!(state.uploads[0].1, format!("voice:{SID}"));
    assert_eq!(state.uploads[2].1, format!("voice:{NEXT}"));
    drop(state);
    h.stop().await;
}

#[tokio::test]
async fn false_wake_never_reaches_agent_and_bare_wake_waits_for_command() {
    let mut h = Harness::new().await;
    h.wake("That is an onion").await;
    until(&mut h.pi, "session.reject").await;
    assert!(h.gateway.lock().await.uploads.is_empty());
    h.wake("Hey Orion").await;
    assert_eq!(until(&mut h.pi, "wake.confirmed").await["followup"], true);
    h.utterance(SID, "command", "Hello").await;
    until(&mut h.pi, "session.finish").await;
    assert_eq!(
        until(&mut h.observer, "agent.response").await["text"],
        "Reply 1: Hello"
    );
    h.stop().await;
}

#[tokio::test]
async fn completion_waits_for_pi_and_observer_detach_does_not_stop_processing() {
    let mut h = Harness::new().await;
    h.gateway.lock().await.allow_complete = false;
    h.wake("Hey Orion, long reply").await;
    until(&mut h.pi, "session.playing").await;
    send(&mut h.observer, json!({"type":"stop"})).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(250), h.pi.next())
            .await
            .is_err()
    );
    h.gateway.lock().await.allow_complete = true;
    until(&mut h.pi, "session.finish").await;
    assert_eq!(h.gateway.lock().await.uploads.len(), 10);
    h.stop().await;
}

#[tokio::test]
async fn failed_startup_synthesis_does_not_upload_partial_audio() {
    let mut h = Harness::new().await;
    h.wake("Hey Orion, tts-fail").await;
    until(&mut h.pi, "session.cancel").await;
    assert!(h.gateway.lock().await.uploads.is_empty());
    h.wake("Hey Orion, recovered").await;
    until(&mut h.pi, "session.finish").await;
    h.stop().await;
}

#[tokio::test]
async fn expiry_cancels_native_inference_and_next_turn_uses_fresh_worker() {
    for command in ["hang-asr", "hang", "hang-tts"] {
        let mut h = Harness::new().await;
        h.wake(&format!("Hey Orion, {command}")).await;
        if command == "hang-tts" {
            until(&mut h.pi, "session.playing").await;
        } else {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        send(&mut h.pi, json!({"type":"session.expired","sessionId":SID})).await;
        loop {
            if next(&mut h.observer).await["type"] == "worker.error" {
                break;
            }
        }
        h.wake("Hey Orion, recovered").await;
        until(&mut h.pi, "session.finish").await;
        if command == "hang-tts" {
            assert_eq!(h.gateway.lock().await.cancellations, vec![1]);
        }
        h.stop().await;
    }
}

#[tokio::test]
async fn app_shutdown_cancels_owned_pi_playback() {
    let mut h = Harness::new().await;
    h.gateway.lock().await.allow_complete = false;
    h.wake("Hey Orion, hello").await;
    until(&mut h.pi, "session.playing").await;
    h.stop().await;
    assert_eq!(h.gateway.lock().await.cancellations, vec![1]);
}

#[tokio::test]
async fn pi_disconnect_cancels_playback_and_reconnect_preserves_agent_conversation() {
    let mut h = Harness::new().await;
    let conversation = h.agent.handle().info().await.unwrap().conversation_id;
    h.gateway.lock().await.allow_complete = false;
    h.wake("Hey Orion, first").await;
    until(&mut h.pi, "session.playing").await;
    h.pi.close(None).await.unwrap();
    let mut pi = tokio::time::timeout(Duration::from_secs(5), h.receive_pi.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(h.gateway.lock().await.cancellations, vec![1]);
    send(&mut pi, json!({"type":"ready", "protocol":1, "sampleRate":16000, "channels":1, "encoding":"pcm_s16le", "muted":false,
        "conversationWindow":true, "wake":{"provider":"rustpotter","model":"pi.rpw","threshold":0.4}})).await;
    h.pi = pi;
    h.gateway.lock().await.allow_complete = true;
    h.wake("Hey Orion, second").await;
    until(&mut h.pi, "session.finish").await;
    assert_eq!(
        h.agent.handle().info().await.unwrap().conversation_id,
        conversation
    );
    h.stop().await;
}

#[tokio::test]
async fn failed_pi_playback_cancels_the_run_and_does_not_open_invitation() {
    let mut h = Harness::new().await;
    h.gateway.lock().await.fail_playback = true;
    h.wake("Hey Orion, hello").await;
    until(&mut h.pi, "session.cancel").await;
    assert_eq!(h.gateway.lock().await.cancellations, vec![1]);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), h.pi.next())
            .await
            .is_err()
    );
    h.gateway.lock().await.fail_playback = false;
    h.wake("Hey Orion, recovered").await;
    until(&mut h.pi, "session.finish").await;
    h.stop().await;
}

#[tokio::test]
async fn observer_cannot_fabricate_playback_completion() {
    let mut h = Harness::new().await;
    h.gateway.lock().await.allow_complete = false;
    h.wake("Hey Orion, hello").await;
    until(&mut h.pi, "session.playing").await;
    send(
        &mut h.observer,
        json!({"type":"playback.finished", "requestId":1}),
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(200), h.pi.next())
            .await
            .is_err()
    );
    h.gateway.lock().await.allow_complete = true;
    until(&mut h.pi, "session.finish").await;
    h.stop().await;
}
