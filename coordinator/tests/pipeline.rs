use futures_util::{SinkExt, StreamExt};
use orion_agent::{AgentConfig, AgentService};
use orion_coordinator::{Coordinator, CoordinatorConfig, SpeechConfig};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
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
    cue_headers: Vec<Option<String>>,
    ended: bool,
    cancellations: Vec<u64>,
    allow_complete: bool,
    runs: u64,
    fail_playback: bool,
    lighting: Vec<Value>,
    robot_operations: Vec<Value>,
    reject_lighting: bool,
    reject_robot: Option<String>,
    reject_upload: Option<String>,
    buffered_ms: Option<i64>,
    playback_clock: Option<PlaybackClock>,
}
#[derive(Default)]
struct PlaybackClock {
    received_ms: u64,
    first_upload: Option<Instant>,
    last_upload: Option<Instant>,
    started: Option<Instant>,
    deferred: bool,
    start_after_received_ms: Option<u64>,
    error: Option<&'static str>,
    max_upload_gap: Duration,
    saw_playing_backpressure: bool,
}
impl PlaybackClock {
    fn accept(&mut self, audio_bytes: usize) {
        let now = Instant::now();
        self.first_upload.get_or_insert(now);
        if let Some(last) = self.last_upload.replace(now) {
            self.max_upload_gap = self.max_upload_gap.max(now.duration_since(last));
        }
        self.received_ms += audio_bytes as u64 / 48;
        if self
            .start_after_received_ms
            .is_some_and(|threshold| self.received_ms >= threshold)
        {
            self.deferred = false;
        }
    }
    fn status(&mut self, ended: bool) -> Value {
        if self.started.is_none() && !self.deferred && (self.received_ms >= 2000 || ended) {
            self.started = Some(Instant::now());
        }
        let played_ms = self
            .started
            .map(|at| at.elapsed().as_millis() as u64)
            .unwrap_or(0);
        if !ended {
            if self
                .last_upload
                .is_some_and(|at| at.elapsed() > Duration::from_secs(10))
            {
                self.error = Some("upload_timeout");
            } else if self.started.is_some() && played_ms > self.received_ms + 120 {
                self.error = Some("buffer_exhausted");
            }
        }
        let buffered_ms = self.received_ms as i64 - played_ms as i64;
        self.saw_playing_backpressure |= self.started.is_some() && buffered_ms > 16_000;
        json!({"state":if self.error.is_some() {"failed"} else if ended && played_ms >= self.received_ms {"completed"} else if self.started.is_some() {"playing"} else {"queued"},
            "error":self.error, "first_playback_ms":self.started.map(|_|25), "buffered_ms":buffered_ms})
    }
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
            soul_path: None,
            memory_path: None,
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
            "conversationWindow":true,"toolFeedback":true, "wake":{"provider":"rustpotter","model":"pi.rpw","threshold":0.4}})).await;
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
    let mut status = "200 OK";
    let value = if path == "/api/v2/operations" {
        let value: Value = serde_json::from_slice(&body).unwrap();
        if value["operation"] == "lamp_effect" {
            state.lighting.push(value["settings"].clone());
            json!({"accepted":!state.reject_lighting,"result":{"ok":!state.reject_lighting}})
        } else if value["operation"] == "lamp_status" {
            json!({"accepted":true,"result":{"ok":true,"lamp":{"brightness":35,"effect":"solid","colors":[[255,0,0,0]]}}})
        } else if value["operation"] == "sleep" || value["operation"] == "routines" {
            state.robot_operations.push(value.clone());
            if let Some(reason) = &state.reject_robot {
                status = "409 Conflict";
                json!({"error":{"message":reason}})
            } else {
                json!({"accepted":true,"result":{"ok":true,"sleep_after_reply":value["operation"]=="sleep"}})
            }
        } else {
            state.cancellations.push(value["run_id"].as_u64().unwrap());
            json!({"ok":true})
        }
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
        if let Some(clock) = &mut state.playback_clock {
            clock.accept(body.len() - 44);
        }
        state.cue_headers.push(
            headers
                .iter()
                .find(|(k, _)| k == "x-orion-speech-cues")
                .map(|(_, v)| v.clone()),
        );
        state.uploads.push((path, request_id, body));
        if let Some(reason) = &state.reject_upload {
            status = "400 Bad Request";
            json!({"error":{"message":reason}})
        } else {
            json!({"run_id":state.runs})
        }
    } else if path.ends_with("/end") {
        state.ended = true;
        json!({"ok":true})
    } else if state.playback_clock.is_some() {
        let ended = state.ended;
        state.playback_clock.as_mut().unwrap().status(ended)
    } else {
        json!({"state":if state.fail_playback { "failed" } else if state.ended && state.allow_complete {"completed"} else {"playing"}, "first_playback_ms":25, "buffered_ms":state.buffered_ms})
    };
    drop(state);
    let body = value.to_string();
    socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
}

#[tokio::test]
async fn reaction_cues_follow_sentence_chunks_without_reaching_speech_or_history() {
    let mut h = Harness::new().await;
    h.wake("Hey Orion, reaction-cues-fixture").await;
    until(&mut h.pi, "session.finish").await;
    let response = until(&mut h.observer, "agent.response").await;
    assert_eq!(response["text"], "Yes. Fine. No.");
    assert_eq!(response["cues"], json!(["agree", "disagree", "agree"]));
    let state = h.gateway.lock().await;
    assert_eq!(state.uploads.len(), 6, "Trailing cue must upload no audio");
    assert_eq!(
        state.cue_headers,
        [
            Some("agree".into()),
            None,
            None,
            None,
            Some("disagree".into()),
            None
        ]
    );
    assert_eq!(state.uploads[0].0, "/api/v2/speech/stream");
    assert_eq!(state.uploads[4].0, "/api/v2/speech/1/chunks/4");
    drop(state);
    h.stop().await;
}

#[tokio::test]
async fn prefix_wakes_early_but_only_complete_audio_reaches_agent() {
    let mut h = Harness::new().await;
    send(
        &mut h.pi,
        json!({"type":"wake.candidate","sessionId":SID,"name":"hey_orion","score":0.8}),
    )
    .await;
    h.utterance(
        SID,
        "wake_prefix",
        "Hey Orion, this truncated prefix must not run",
    )
    .await;
    assert_eq!(until(&mut h.pi, "wake.verified").await["accepted"], true);
    assert_eq!(
        until(&mut h.observer, "wake.confirmed").await["early"],
        true
    );
    assert!(h.gateway.lock().await.uploads.is_empty());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), h.observer.next())
            .await
            .is_err()
    );
    h.utterance(
        SID,
        "wake_and_command",
        "Hey Orion, keep every command word",
    )
    .await;
    until(&mut h.pi, "session.finish").await;
    assert_eq!(
        until(&mut h.observer, "agent.response").await["text"],
        "Reply 1: keep every command word"
    );
    h.stop().await;
}

#[tokio::test]
async fn acoustic_verdict_confirms_early_without_asr_prefix() {
    let mut h = Harness::new().await;
    send(
        &mut h.pi,
        json!({"type":"wake.candidate","sessionId":SID,"name":"hey_orion","score":0.4,"acousticVerification":true}),
    )
    .await;
    send(
        &mut h.pi,
        json!({"type":"wake.verified","sessionId":SID,"accepted":true,"source":"acoustic","score":0.9,"verifierMs":0}),
    )
    .await;
    let confirmed = until(&mut h.observer, "wake.confirmed").await;
    assert_eq!(
        (confirmed["early"].clone(), confirmed["source"].clone()),
        (json!(true), json!("acoustic"))
    );
    // The prefix pass is replaced, so a late prefix would be a protocol error.
    h.utterance(SID, "wake_and_command", "He Orion, keep every command word")
        .await;
    until(&mut h.pi, "session.finish").await;
    assert_eq!(
        until(&mut h.observer, "agent.response").await["text"],
        "Reply 1: keep every command word"
    );
    h.stop().await;
}

#[tokio::test]
async fn acoustic_rejection_clears_session_before_next_wake() {
    let mut h = Harness::new().await;
    send(
        &mut h.pi,
        json!({"type":"wake.candidate","sessionId":SID,"name":"hey_orion","score":0.4,"acousticVerification":true}),
    )
    .await;
    send(
        &mut h.pi,
        json!({"type":"wake.verified","sessionId":SID,"accepted":false,"source":"acoustic","score":0.1,"verifierMs":1000}),
    )
    .await;
    let rejected = until(&mut h.observer, "wake.rejected").await;
    assert_eq!(rejected["sessionId"], SID);
    assert_eq!(rejected["source"], "acoustic");
    assert_eq!(rejected["text"], "");
    assert_eq!(until(&mut h.pi, "session.cancel").await["sessionId"], SID);
    assert!(h.gateway.lock().await.uploads.is_empty());
    // Reuse the same connection: a stale session would reject this candidate
    // as overlapping, before either ASR or the agent could run.
    send(
        &mut h.pi,
        json!({"type":"wake.candidate","sessionId":NEXT,"name":"hey_orion","score":0.8,"acousticVerification":true}),
    )
    .await;
    send(
        &mut h.pi,
        json!({"type":"wake.verified","sessionId":NEXT,"accepted":true,"source":"acoustic","score":0.9,"verifierMs":0}),
    )
    .await;
    assert_eq!(
        until(&mut h.observer, "wake.confirmed").await["sessionId"],
        NEXT
    );
    h.utterance(NEXT, "wake_and_command", "Hey Orion, complete request")
        .await;
    assert_eq!(until(&mut h.pi, "session.finish").await["sessionId"], NEXT);
    assert_eq!(
        until(&mut h.observer, "agent.response").await["text"],
        "Reply 1: complete request"
    );
    h.stop().await;
}

#[tokio::test]
async fn uncertain_prefix_falls_back_to_full_wake_confirmation() {
    let mut h = Harness::new().await;
    send(
        &mut h.pi,
        json!({"type":"wake.candidate","sessionId":SID,"name":"hey_orion","score":0.8}),
    )
    .await;
    h.utterance(SID, "wake_prefix", "Hey Ryan").await;
    assert_eq!(until(&mut h.pi, "wake.verified").await["accepted"], false);
    h.utterance(SID, "wake_and_command", "Hey Orion, complete request")
        .await;
    until(&mut h.pi, "session.finish").await;
    assert_eq!(
        until(&mut h.observer, "agent.response").await["text"],
        "Reply 1: complete request"
    );
    h.stop().await;
}

#[tokio::test]
async fn rejected_full_recording_never_executes_even_after_prefix_confirmation() {
    let mut h = Harness::new().await;
    send(
        &mut h.pi,
        json!({"type":"wake.candidate","sessionId":SID,"name":"hey_orion","score":0.8}),
    )
    .await;
    h.utterance(SID, "wake_prefix", "Hey Orion").await;
    until(&mut h.pi, "wake.verified").await;
    h.utterance(SID, "wake_and_command", "background television")
        .await;
    until(&mut h.pi, "session.reject").await;
    until(&mut h.observer, "wake.rejected").await;
    assert!(h.gateway.lock().await.uploads.is_empty());
    h.stop().await;
}

#[tokio::test]
async fn streamed_sentences_share_one_playback_run_without_duplicate_speech() {
    let mut h = Harness::new().await;
    h.wake("Hey Orion, stream-fixture").await;
    until(&mut h.pi, "session.finish").await;
    assert_eq!(
        until(&mut h.observer, "agent.response").await["text"],
        "Let us begin. Take a breath."
    );
    let state = h.gateway.lock().await;
    assert_eq!(state.runs, 1);
    assert_eq!(state.uploads.len(), 4); // Two chunks per sentence, with no duplicate final synthesis.
    drop(state);
    h.stop().await;
}

#[tokio::test]
async fn recording_limit_never_sends_an_incomplete_command_to_the_agent() {
    let mut h = Harness::new().await;
    send(
        &mut h.pi,
        json!({"type":"wake.candidate","sessionId":SID,"name":"hey_orion","score":0.8}),
    )
    .await;
    let pcm = b"Hey Orion, incomplete request!".to_vec();
    send(&mut h.pi, json!({"type":"utterance","sessionId":SID,"purpose":"wake_and_command","bytes":pcm.len(),"endReason":"max_duration"})).await;
    h.pi.send(Message::Binary(pcm.into())).await.unwrap();
    until(&mut h.pi, "session.cancel").await;
    assert_eq!(next(&mut h.observer).await["type"], "wake.candidate");
    assert_eq!(next(&mut h.observer).await["code"], "utterance_too_long");
    assert!(h.gateway.lock().await.uploads.is_empty());
    h.wake("Hey Orion, complete request").await;
    until(&mut h.pi, "session.finish").await;
    assert_eq!(
        until(&mut h.observer, "agent.response").await["text"],
        "Reply 1: complete request"
    );
    h.stop().await;
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
        "conversationWindow":true,"toolFeedback":true, "wake":{"provider":"rustpotter","model":"pi.rpw","threshold":0.4}})).await;
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

#[tokio::test]
async fn search_acknowledgement_does_not_finish_session_or_open_followup() {
    let mut h = Harness::new().await;
    h.wake("Hey Orion, search-fixture").await;
    until(&mut h.pi, "session.playing").await;
    // The next processing transition must follow acknowledgement playback,
    // before the final response starts, with no session.finish in between.
    let next_message = next(&mut h.pi).await;
    assert_eq!(next_message["type"], "session.processing");
    assert_eq!(next_message["sessionId"], SID);
    until(&mut h.pi, "session.playing").await;
    let finish = next(&mut h.pi).await;
    assert_eq!(finish["type"], "session.finish");
    assert_eq!(finish["conversationWindow"], true);
    assert_eq!(h.gateway.lock().await.runs, 2);
    h.stop().await;
}

#[tokio::test]
async fn lighting_translates_parameters_and_only_confirms_gateway_success() {
    for reject in [false, true] {
        let mut h = Harness::new().await;
        h.gateway.lock().await.reject_lighting = reject;
        h.wake(r#"Hey Orion, tool:{"name":"set_lighting","arguments":{"mood":"warm_red","brightness":30}}"#).await;
        until(&mut h.pi, "session.finish").await;
        let response = until(&mut h.observer, "agent.response").await;
        if reject {
            assert!(
                response["text"]
                    .as_str()
                    .unwrap()
                    .contains("did not confirm")
            );
        } else {
            assert!(response["text"].as_str().unwrap().contains("applied"));
        }
        let state = h.gateway.lock().await;
        assert_eq!(state.lighting.len(), 1);
        assert_eq!(state.lighting[0]["brightness"], 0.3);
        assert_eq!(state.lighting[0]["colors"][1], json!([255, 0, 0, 0]));
        drop(state);
        h.stop().await;
    }
}

#[tokio::test]
async fn sleep_tool_uses_current_session_and_closes_after_acknowledgement() {
    let mut h = Harness::new().await;
    h.wake(r#"Hey Orion, tool:{"name":"go_to_sleep","arguments":{}}"#)
        .await;
    until(&mut h.pi, "session.playing").await;
    let finish = until(&mut h.pi, "session.finish").await;
    assert_ne!(finish["conversationWindow"], true);
    assert_eq!(
        h.gateway.lock().await.robot_operations[0],
        json!({"operation":"sleep","session_id":SID})
    );
    h.stop().await;
}
#[tokio::test]
async fn explicit_sleep_calls_tool_before_promising_rest() {
    let mut h = Harness::new().await;
    // Qwen rendered the user's "go to sleep" as "go through sleep" on the Pi.
    h.wake("Hey Orion, go through sleep.").await;
    let tool = until(&mut h.observer, "agent.tool").await;
    assert_eq!(tool["name"], "go_to_sleep");
    assert_eq!(tool["success"], true);
    let answer = until(&mut h.observer, "agent.response").await;
    assert!(answer["text"].as_str().unwrap().contains("go to sleep"));
    let finish = until(&mut h.pi, "session.finish").await;
    assert_ne!(finish["conversationWindow"], true);
    assert_eq!(
        h.gateway.lock().await.robot_operations,
        [json!({"operation":"sleep","session_id":SID})]
    );
    h.stop().await;
}
#[tokio::test]
async fn rejected_explicit_sleep_does_not_claim_rest() {
    let mut h = Harness::new().await;
    h.gateway.lock().await.reject_robot =
        Some("Sleep requires the current confirmed voice session".into());
    h.wake("Hey Orion, go to sleep.").await;
    let tool = until(&mut h.observer, "agent.tool").await;
    assert_eq!(tool["name"], "go_to_sleep");
    assert_eq!(tool["success"], false);
    let answer = until(&mut h.observer, "agent.response").await;
    assert!(
        answer["text"]
            .as_str()
            .unwrap()
            .contains("couldn't go to sleep")
    );
    let finish = until(&mut h.pi, "session.finish").await;
    assert_eq!(finish["conversationWindow"], true);
    h.stop().await;
}
#[tokio::test]
async fn timer_request_reaches_the_pi_and_is_visible_in_history_events() {
    let mut h = Harness::new().await;
    h.wake(r#"Hey Orion, tool:{"name":"set_timer","arguments":{"seconds":300,"label":"Tea"}}"#)
        .await;
    until(&mut h.pi, "session.finish").await;
    let operation = h.gateway.lock().await.robot_operations[0].clone();
    assert_eq!(operation["operation"], "routines");
    assert_eq!(
        operation["request"],
        json!({"action":"timer","seconds":300.0,"label":"Tea"})
    );
    let tool = until(&mut h.observer, "agent.tool").await;
    assert_eq!(tool["name"], "set_timer");
    assert_eq!(tool["arguments"]["seconds"], 300);
    assert_eq!(tool["success"], true);
    h.stop().await;
}
#[tokio::test]
async fn alarm_interrupt_retires_playback_and_accepts_a_later_wake() {
    let mut h = Harness::new().await;
    h.gateway.lock().await.allow_complete = false;
    h.wake("Hey Orion, reply please").await;
    until(&mut h.pi, "session.playing").await;
    send(
        &mut h.pi,
        json!({"type":"session.interrupted","sessionId":SID,"reason":"alarm"}),
    )
    .await;
    until(&mut h.observer, "session.interrupted").await;
    assert!(!h.gateway.lock().await.cancellations.is_empty());
    h.gateway.lock().await.allow_complete = true;
    h.wake("Hey Orion, hello again").await;
    until(&mut h.pi, "session.finish").await;
    h.stop().await;
}

#[tokio::test]
async fn barge_in_outside_responding_preserves_the_pi_connection_and_session() {
    let mut h = Harness::new().await;
    send(
        &mut h.pi,
        json!({"type":"wake.candidate","sessionId":SID,"name":"hey_orion","score":0.8}),
    )
    .await;
    until(&mut h.observer, "wake.candidate").await;
    // A late interruption during wake capture must leave the session usable.
    send(
        &mut h.pi,
        json!({"type":"session.interrupted","sessionId":SID,"reason":"barge_in"}),
    )
    .await;
    h.utterance(SID, "wake_and_command", "Hey Orion").await;
    let confirmed = until(&mut h.pi, "wake.confirmed").await;
    assert_eq!(confirmed["sessionId"], SID);
    assert_eq!(confirmed["followup"], true);
    until(&mut h.observer, "command.started").await;
    // The same rule applies while capturing the command after a bare wake.
    send(
        &mut h.pi,
        json!({"type":"session.interrupted","sessionId":SID,"reason":"barge_in"}),
    )
    .await;
    h.utterance(SID, "command", "hello after ignored interruption")
        .await;
    let answer = until(&mut h.observer, "agent.response").await;
    assert_eq!(answer["sessionId"], SID);
    assert_eq!(answer["text"], "Reply 1: hello after ignored interruption");
    assert_eq!(until(&mut h.pi, "session.finish").await["sessionId"], SID);
    assert!(h.gateway.lock().await.cancellations.is_empty());
    h.stop().await;
}

#[tokio::test]
async fn barge_in_stops_speech_preserves_the_running_agent_and_queues_the_next_command() {
    let mut h = Harness::new().await;
    let conversation = h.agent.handle().info().await.unwrap().conversation_id;
    h.gateway.lock().await.allow_complete = false;
    h.wake("Hey Orion, barge-in-fixture").await;
    until(&mut h.pi, "session.playing").await;
    send(
        &mut h.pi,
        json!({"type":"session.interrupted","sessionId":SID,"reason":"barge_in"}),
    )
    .await;
    let interrupted = until(&mut h.observer, "session.interrupted").await;
    assert_eq!(interrupted["reason"], "barge_in");
    assert_eq!(h.gateway.lock().await.cancellations, [1]);
    let old_uploads = h.gateway.lock().await.uploads.len();
    // The first agent turn is still generating when the new capture arrives.
    send(&mut h.pi, json!({"type":"wake.candidate","sessionId":NEXT,"name":"hey_orion","score":0.8,"acousticVerification":true})).await;
    send(
        &mut h.pi,
        json!({"type":"wake.verified","sessionId":NEXT,"accepted":true,"source":"acoustic"}),
    )
    .await;
    h.utterance(NEXT, "wake_and_command", "Hey Orion, what were you saying?")
        .await;
    let processing = until(&mut h.pi, "session.processing").await;
    assert_eq!(processing["sessionId"], NEXT);
    let first = until(&mut h.observer, "agent.response").await;
    assert_eq!(first["sessionId"], SID);
    assert!(first["text"].as_str().unwrap().ends_with("Unspoken tail."));
    let second = until(&mut h.observer, "agent.response").await;
    assert_eq!(second["sessionId"], NEXT);
    let answer = second["text"].as_str().unwrap();
    assert!(answer.starts_with("Reply 2:"), "{answer}");
    assert!(answer.contains("The previous reply was interrupted before it finished."));
    assert!(answer.contains("User request: what were you saying?"));
    h.gateway.lock().await.allow_complete = true;
    let finish = until(&mut h.pi, "session.finish").await;
    assert_eq!(
        finish["sessionId"], NEXT,
        "Retired completion must not finish the newer session"
    );
    let state = h.gateway.lock().await;
    assert_eq!(
        state
            .uploads
            .iter()
            .filter(|(_, source, _)| source == &format!("voice:{SID}"))
            .count(),
        old_uploads
    );
    drop(state);
    assert_eq!(
        h.agent.handle().info().await.unwrap().conversation_id,
        conversation
    );
    h.stop().await;
}

#[tokio::test]
async fn barge_in_during_search_acknowledgement_preserves_the_turn() {
    let mut h = Harness::new().await;
    h.gateway.lock().await.allow_complete = false;
    h.wake("Hey Orion, search-fixture").await;
    until(&mut h.pi, "session.playing").await;
    send(
        &mut h.pi,
        json!({"type":"session.interrupted","sessionId":SID,"reason":"barge_in"}),
    )
    .await;
    // Draining the old turn and cancelling the gateway run are independent;
    // either observer event may arrive first.
    let (mut interrupted, mut completed) = (false, false);
    while !interrupted || !completed {
        let value = next(&mut h.observer).await;
        match value["type"].as_str() {
            Some("session.interrupted") => {
                assert_eq!(value["reason"], "barge_in");
                interrupted = true;
            }
            Some("agent.response") => {
                assert_eq!(value["sessionId"], SID);
                completed = true;
            }
            Some("worker.error") => panic!("{value}"),
            _ => {}
        }
    }
    assert!(!h.gateway.lock().await.cancellations.is_empty());
    h.gateway.lock().await.allow_complete = true;
    h.wake("Hey Orion, hello again").await;
    let answer = until(&mut h.observer, "agent.response").await;
    assert!(answer["text"].as_str().unwrap().starts_with("Reply 2:"));
    until(&mut h.pi, "session.finish").await;
    h.stop().await;
}

#[tokio::test]
async fn runtime_alarm_reason_reaches_the_agent_tool_result_unchanged() {
    let mut h = Harness::new().await;
    h.gateway.lock().await.reject_robot = Some("Alarm must be in the next 366 days".into());
    let before = h.agent.handle().info().await.unwrap().conversation_id;
    h.wake(r#"Hey Orion, tool:{"name":"set_alarm","arguments":{"at":"2030-01-01T09:00:00+00:00","label":"wake"}}"#).await;
    let tool = until(&mut h.observer, "agent.tool").await;
    assert_eq!(
        tool["result"]["error"],
        "Alarm must be in the next 366 days"
    );
    assert_eq!(tool["success"], false);
    until(&mut h.pi, "session.finish").await;
    assert_eq!(
        h.agent.handle().info().await.unwrap().conversation_id,
        before
    );
    h.stop().await;
}

#[tokio::test]
async fn long_unpunctuated_answer_synthesizes_every_bounded_piece() {
    let mut h = Harness::new().await;
    let command = format!("Hey Orion, {}", "something ".repeat(1200));
    h.wake(&command).await;
    let response = until(&mut h.observer, "agent.response").await;
    assert_eq!(
        response["text"].as_str().unwrap(),
        format!("Reply 1: {}", "something ".repeat(1200).trim())
    );
    until(&mut h.pi, "session.finish").await;
    // Fixture emits two seconds per bounded TTS input: the reply exceeds 800 chars.
    let state = h.gateway.lock().await;
    assert!(
        state
            .uploads
            .iter()
            .map(|(_, _, wav)| wav.len() - 44)
            .sum::<usize>()
            > 150 * 48_000
    );
    assert_eq!(state.runs, 1);
    assert!(state.ended);
    drop(state);
    h.stop().await;
}

#[tokio::test]
async fn get_lighting_returns_manual_state_to_the_agent() {
    let mut h = Harness::new().await;
    h.wake(r#"Hey Orion, tool:{"name":"get_lighting","arguments":{}}"#)
        .await;
    let tool = until(&mut h.observer, "agent.tool").await;
    assert_eq!(tool["name"], "get_lighting");
    assert_eq!(tool["result"]["lamp"]["brightness"], 35);
    assert_eq!(tool["success"], true);
    until(&mut h.pi, "session.finish").await;
    h.stop().await;
}

#[tokio::test]
async fn long_playback_renews_the_listener_lease_until_completion() {
    let mut h = Harness::new().await;
    h.gateway.lock().await.allow_complete = false;
    h.wake("Hey Orion, reply please").await;
    until(&mut h.pi, "session.playing").await;
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let value = h.pi.next().await.unwrap().unwrap();
            if let Message::Text(raw) = value {
                let value: Value = serde_json::from_str(&raw).unwrap();
                if value["type"] == "session.keepalive" {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    h.gateway.lock().await.allow_complete = true;
    until(&mut h.pi, "session.finish").await;
    h.stop().await;
}

#[tokio::test]
async fn slow_synthesis_does_not_backpressure_agent_generation() {
    let mut h = Harness::new().await;
    h.wake("Hey Orion, many-sentences-fixture").await;
    // The first TTS input hangs after six chunks. The entire Codex turn must
    // still complete promptly, including more text events than the old queue.
    let response = until(&mut h.observer, "agent.response").await;
    assert!(response["text"].as_str().unwrap().ends_with("Sentence 39."));
    h.stop().await;
}

#[tokio::test]
async fn preplayback_wait_renews_without_repeating_processing_or_playing() {
    let mut h = Harness::new().await;
    h.wake("Hey Orion, preplay-tts").await;
    until(&mut h.pi, "session.processing").await;
    let renewal = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Message::Text(raw) = h.pi.next().await.unwrap().unwrap() {
                let value: Value = serde_json::from_str(&raw).unwrap();
                assert_ne!(value["type"], "session.processing");
                assert_ne!(value["type"], "session.playing");
                if value["type"] == "session.keepalive" {
                    break value;
                }
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(renewal["sessionId"], SID);
    assert!(h.gateway.lock().await.uploads.is_empty());
    until(&mut h.pi, "session.finish").await;
    h.stop().await;
}

#[tokio::test]
async fn uploads_wait_for_playback_to_drain_instead_of_accumulating_the_reply() {
    let mut h = Harness::new().await;
    h.gateway.lock().await.buffered_ms = Some(18_000);
    h.wake("Hey Orion, long long long long long").await;
    until(&mut h.pi, "session.playing").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(h.gateway.lock().await.uploads.len(), 1);
    assert!(!h.gateway.lock().await.ended);
    h.gateway.lock().await.buffered_ms = Some(0);
    until(&mut h.pi, "session.finish").await;
    assert_eq!(h.gateway.lock().await.uploads.len(), 10);
    assert!(h.gateway.lock().await.ended);
    h.stop().await;
}

#[tokio::test]
async fn speech_sanity_failure_reaches_the_user_and_next_model_turn() {
    let mut h = Harness::new().await;
    let reason = "Speech stream exceeds the 30-minute audio sanity limit; playback cannot complete this answer.";
    h.gateway.lock().await.reject_upload = Some(reason.into());
    h.wake("Hey Orion, reply please").await;
    let failure = loop {
        let value = next(&mut h.observer).await;
        if value["type"] == "worker.error" {
            break value;
        }
    };
    assert!(failure["message"].as_str().unwrap().contains(reason));
    let answer = h.agent.handle().respond("What happened?").await.unwrap();
    assert!(answer.contains("Previous speech delivery failed:"));
    assert!(answer.contains(reason));
    h.stop().await;
}

#[tokio::test]
async fn slower_than_realtime_generation_waits_for_the_complete_reply_and_plays_without_underrun() {
    let mut h = Harness::new().await;
    h.gateway.lock().await.playback_clock = Some(PlaybackClock::default());
    let requested = Instant::now();
    h.wake("Hey Orion, slow-reply-tts").await;
    tokio::time::timeout(Duration::from_secs(55), async {
        loop {
            if let Message::Text(raw) = h.pi.next().await.unwrap().unwrap() {
                let value: Value = serde_json::from_str(&raw).unwrap();
                assert_ne!(value["type"], "session.cancel", "{value}");
                if value["type"] == "session.playing" {
                    let state = h.gateway.lock().await;
                    let clock = state.playback_clock.as_ref().unwrap();
                    // Twenty one-second chunks take at least 21 seconds to generate.
                    // The old 12-second release would fail this check and exhaust
                    // the fake runtime, which consumes PCM at real-time speed.
                    assert!(
                        clock.first_upload.unwrap().duration_since(requested)
                            >= Duration::from_secs(21)
                    );
                }
                if value["type"] == "session.finish" {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    {
        let state = h.gateway.lock().await;
        let clock = state.playback_clock.as_ref().unwrap();
        assert_eq!(state.uploads.len(), 20);
        assert_eq!(state.runs, 1);
        assert!(state.ended);
        assert_eq!(clock.error, None);
        assert!(
            clock.saw_playing_backpressure,
            "burst exceeds sixteen seconds while playing"
        );
        assert!(clock.max_upload_gap < Duration::from_secs(10));
        assert!(state.cancellations.is_empty());
    }
    h.stop().await;
}

#[tokio::test]
async fn complete_reply_finishes_uploading_when_runtime_playback_is_deferred() {
    let mut h = Harness::new().await;
    h.gateway.lock().await.playback_clock = Some(PlaybackClock {
        deferred: true,
        ..Default::default()
    });
    h.wake("Hey Orion, latched-burst-tts").await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !h.gateway.lock().await.ended {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    {
        let mut state = h.gateway.lock().await;
        assert_eq!(state.uploads.len(), 20);
        let clock = state.playback_clock.as_mut().unwrap();
        assert_eq!(clock.received_ms, 20_000);
        assert_eq!(clock.started, None);
        assert_eq!(clock.error, None);
        assert!(clock.max_upload_gap < Duration::from_secs(10));
        // Advance only fixture playback after proving the queued burst reached
        // its end marker; no real hardware or long home movement is needed.
        clock.deferred = false;
        clock.started = Some(Instant::now() - Duration::from_secs(20));
    }
    until(&mut h.pi, "session.finish").await;
    h.stop().await;
}

#[tokio::test]
async fn a_complete_buffered_reply_over_thirty_minutes_fails_before_any_upload() {
    let mut h = Harness::new().await;
    h.wake("Hey Orion, over-limit-tts").await;
    let failure = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Message::Text(raw) = h.observer.next().await.unwrap().unwrap() {
                let value: Value = serde_json::from_str(&raw).unwrap();
                if value["type"] == "worker.error" {
                    break value;
                }
            }
        }
    })
    .await
    .unwrap();
    let reason = "Speech stream exceeds the 30-minute audio sanity limit; playback cannot complete this answer.";
    assert!(failure["message"].as_str().unwrap().contains(reason));
    assert!(h.gateway.lock().await.uploads.is_empty());
    assert!(!h.gateway.lock().await.ended);
    let answer = h.agent.handle().respond("What happened?").await.unwrap();
    assert!(answer.contains(reason));
    h.stop().await;
}

#[tokio::test]
async fn a_queued_complete_burst_keeps_uploading_when_playback_starts_midway() {
    let mut h = Harness::new().await;
    h.gateway.lock().await.playback_clock = Some(PlaybackClock {
        deferred: true,
        start_after_received_ms: Some(30_000),
        ..Default::default()
    });
    h.wake("Hey Orion, transition-burst-tts").await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !h.gateway.lock().await.ended {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    {
        let mut state = h.gateway.lock().await;
        assert_eq!(state.uploads.len(), 50);
        let clock = state.playback_clock.as_mut().unwrap();
        assert!(
            clock.started.is_some(),
            "playback must start before the burst ends"
        );
        assert!(clock.saw_playing_backpressure);
        assert_eq!(clock.error, None);
        assert!(clock.max_upload_gap < Duration::from_secs(10));
        clock.started = Some(Instant::now() - Duration::from_secs(50));
    }
    until(&mut h.pi, "session.finish").await;
    h.stop().await;
}
