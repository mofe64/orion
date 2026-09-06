use crate::{
    CoordinatorConfig,
    buffer::StartupBuffer,
    gateway::{ActiveRun, Gateway},
    hub::Hub,
    session::{Phase, Session, after_wake},
    speech::{Chunk, SpeechRuntime},
};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use orion_agent::AgentHandle;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    net::TcpStream,
    sync::{Mutex, mpsc, watch},
    task::JoinSet,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
#[derive(Clone)]
struct Pi(Arc<Mutex<SplitSink<Socket, Message>>>);
impl Pi {
    async fn send(&self, value: Value) -> Result<(), String> {
        tokio::time::timeout(
            Duration::from_secs(5),
            self.0.lock().await.send(Message::text(value.to_string())),
        )
        .await
        .map_err(|_| "Pi send timed out")?
        .map_err(|e| e.to_string())
    }
    async fn receive(&self, input: &mut SplitStream<Socket>) -> Result<Message, String> {
        loop {
            match input
                .next()
                .await
                .ok_or("Pi disconnected")?
                .map_err(|e| e.to_string())?
            {
                Message::Ping(data) => self
                    .0
                    .lock()
                    .await
                    .send(Message::Pong(data))
                    .await
                    .map_err(|e| e.to_string())?,
                Message::Pong(_) => {}
                Message::Close(_) => return Err("Pi disconnected".into()),
                message => return Ok(message),
            }
        }
    }
}
async fn connect(
    config: &CoordinatorConfig,
    control: bool,
) -> Result<(Pi, SplitStream<Socket>, Value), String> {
    let options = WebSocketConfig::default()
        .max_message_size(Some(18 * 32000))
        .max_frame_size(Some(18 * 32000));
    let (socket, _) = connect_async_with_config(&config.pi_url, Some(options), false)
        .await
        .map_err(|e| e.to_string())?;
    let (output, mut input) = socket.split();
    let pi = Pi(Arc::new(Mutex::new(output)));
    let mut hello = json!({"type":"hello", "protocol":1, "token":config.pi_token});
    if control {
        hello["role"] = "control".into();
    }
    pi.send(hello).await?;
    let ready = decode(pi.receive(&mut input).await?)?;
    Ok((pi, input, ready))
}
fn decode(message: Message) -> Result<Value, String> {
    if let Message::Text(text) = message {
        serde_json::from_str(&text).map_err(|e| e.to_string())
    } else {
        Err("Expected Pi JSON control message".into())
    }
}
pub(crate) async fn microphone(
    config: &CoordinatorConfig,
    muted: Option<bool>,
) -> Result<Value, String> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (pi, mut input, mut status) = connect(config, true).await?;
        if let Some(muted) = muted {
            pi.send(json!({"type":"microphone.mute", "muted":muted}))
                .await?;
            status = decode(pi.receive(&mut input).await?)?;
        }
        let muted = status["muted"]
            .as_bool()
            .ok_or("Invalid Pi microphone status")?;
        Ok(json!({"type":"microphone.status", "muted":muted}))
    })
    .await
    .map_err(|_| "Pi microphone control timed out")?
}
pub(crate) fn error(code: &str, message: &str, recoverable: bool) -> Value {
    json!({"type":"worker.error", "code":code, "message":message, "recoverable":recoverable})
}
fn event(hub: &Hub, sid: &str, mut message: Value) {
    message["sessionId"] = sid.into();
    hub.publish(message);
}
fn timing(hub: &Hub, sid: &str, stage: &str, ms: f64) {
    event(
        hub,
        sid,
        json!({"type":"stage.timing", "stage":stage, "durationMs":ms}),
    );
}

pub(crate) async fn run(
    config: CoordinatorConfig,
    agent: AgentHandle,
    speech: Arc<SpeechRuntime>,
    hub: Hub,
    mut stop: watch::Receiver<bool>,
) {
    let gateway = match Gateway::new(&config.gateway_url, &config.pi_token) {
        Ok(gateway) => gateway,
        Err(message) => {
            hub.publish(error("gateway_failed", &message, false));
            return;
        }
    };
    let mut request_id = 0;
    loop {
        let initialization = async { Ok::<_, String>((speech.info().await?, agent.info().await?)) };
        let result = tokio::select! {
            _ = stop.changed() => return,
            result = initialization => result,
        };
        match result {
            Ok((models, info)) => {
                if let Err(message) = connected(
                    &config,
                    &agent,
                    &speech,
                    &hub,
                    &gateway,
                    &models,
                    json!(info),
                    &mut request_id,
                    &mut stop,
                )
                .await
                    && !*stop.borrow()
                {
                    hub.publish(error("pi_unavailable", &message, true));
                }
            }
            Err(message) => hub.publish(error("model_load_failed", &message, false)),
        }
        if *stop.borrow() {
            return;
        }
        tokio::select! { _ = stop.changed() => return, _ = tokio::time::sleep(Duration::from_secs(2)) => {} }
    }
}

enum Completed {
    Transcript {
        sid: String,
        purpose: String,
        transcript: Value,
        elapsed: f64,
    },
    Response,
}

#[allow(clippy::too_many_arguments)]
async fn connected(
    config: &CoordinatorConfig,
    agent: &AgentHandle,
    speech: &Arc<SpeechRuntime>,
    hub: &Hub,
    gateway: &Gateway,
    models: &Value,
    agent_info: Value,
    next_request: &mut u64,
    stop: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let connection = tokio::time::timeout(Duration::from_secs(10), connect(config, false));
    let (pi, mut input, ready) = tokio::select! {
        _ = stop.changed() => return Ok(()),
        result = connection => result.map_err(|_| "Pi connection timed out")??,
    };
    if ready["type"] != "ready"
        || ready["protocol"] != 1
        || ready["sampleRate"] != 16000
        || ready["channels"] != 1
        || ready["encoding"] != "pcm_s16le"
        || !ready["wake"].is_object()
    {
        return Err("Unsupported Pi listener contract".into());
    }
    let window = ready["conversationWindow"] == true;
    let tool_feedback = ready["toolFeedback"] == true;
    hub.publish(
        json!({"type":"ready", "protocol":7, "muted":ready["muted"], "asr":models["asr"],
        "tts":models["tts"], "wake":ready["wake"], "agent":agent_info}),
    );
    let mut session: Option<Session> = None;
    let mut followup: Option<String> = None;
    let mut jobs = JoinSet::<Result<Completed, String>>::new();
    let active: ActiveRun = Arc::new(Mutex::new(None));
    let result = async {
        loop {
            tokio::select! {
                biased;
                _ = stop.changed() => return Ok(()),
                Some(result) = jobs.join_next(), if !jobs.is_empty() => {
                    let current = session.as_mut().ok_or("A job completed without an active session")?;
                    let sid = current.id.clone();
                    match result.map_err(|e| e.to_string())? {
                        Err(message) => {
                            gateway.cancel(&active).await;
                            pi.send(json!({"type":"session.cancel", "sessionId":sid})).await?;
                            event(hub, &sid, error("voice_request_failed", &message, true));
                            session = None; followup = None;
                        },
                        Ok(Completed::Transcript { sid: job_sid, purpose, transcript, elapsed }) => {
                            if job_sid != sid || current.phase != Phase::Transcribing { return Err("Stale transcription result".into()); }
                            timing(hub, &sid, &format!("transcription_{purpose}"), elapsed);
                            let raw = transcript["text"].as_str().ok_or("Invalid transcript text")?.trim();
                            let command = if purpose == "wake_and_command" {
                                match after_wake(raw) {
                                    None => {
                                        pi.send(json!({"type":"session.reject", "sessionId":sid})).await?;
                                        event(hub, &sid, json!({"type":"wake.rejected", "text":raw}));
                                        session = None;
                                        continue;
                                    },
                                    Some(command) => {
                                        current.phase = if command.is_empty() { Phase::Command } else { Phase::Responding };
                                        pi.send(json!({"type":"wake.confirmed", "sessionId":sid, "followup":command.is_empty()})).await?;
                                        event(hub, &sid, json!({"type":"wake.confirmed", "text":raw, "hasCommand":!command.is_empty()}));
                                        if command.is_empty() {
                                            event(hub, &sid, json!({"type":"command.started"})); continue;
                                        }
                                        command
                                    }
                                }
                            } else { raw.to_owned() };
                            if command.is_empty() {
                                pi.send(json!({"type":"session.cancel", "sessionId":sid})).await?;
                                event(hub, &sid, error("transcription_failed", "No command was heard", true));
                                session = None; continue;
                            }
                            current.phase = Phase::Responding;
                            event(hub, &sid, json!({"type":"transcript.final", "text":command, "rawText":raw,
                                "language":transcript["language"], "durationMs":elapsed}));
                            pi.send(json!({"type":"session.processing", "sessionId":sid})).await?;
                            let agent = agent.clone(); let speech = speech.clone(); let gateway = gateway.clone();
                            let hub = hub.clone(); let pi = pi.clone(); let active = active.clone(); let request = *next_request;
                            jobs.spawn(async move { response(&agent, &speech, &gateway, &pi, &hub, &sid, request, &command, active, tool_feedback).await.map(|_| Completed::Response) });
                        },
                        Ok(Completed::Response) => {
                            let request = *next_request;
                            followup = window.then_some(sid.clone()); session = None;
                            let mut finish = json!({"type":"session.finish", "sessionId":sid});
                            if window { finish["conversationWindow"] = true.into(); }
                            pi.send(finish).await?;
                            event(hub, &sid, json!({"type":"speech.completed", "requestId":request}));
                        }
                    }
                },
                message = pi.receive(&mut input) => {
                    let message = decode(message?)?;
                    let sid = message["sessionId"].as_str().ok_or("Pi event has no session ID")?;
                    match message["type"].as_str().ok_or("Pi event has no type")? {
                        "wake.candidate" | "command.candidate" => {
                            if session.is_some() || !jobs.is_empty() { return Err("Overlapping Pi voice session".into()); }
                            let continuation = message["type"] == "command.candidate";
                            if continuation && (followup.as_deref() != message["previousSessionId"].as_str() || followup.is_none() || followup.as_deref() == Some(sid)) {
                                return Err("Unexpected conversation continuation".into());
                            }
                            session = Some(Session::new(sid, if continuation { Phase::Command } else { Phase::Wake })?);
                            followup = None;
                            if continuation { event(hub, sid, json!({"type":"command.started"})); } else { hub.publish(message); }
                        },
                        "conversation.ready" | "conversation.closed" => {
                            if followup.as_deref() != Some(sid) { return Err("Stale conversation window event".into()); }
                            let open = message["type"] == "conversation.ready";
                            hub.publish(json!({"type":"conversation.window", "active":open}));
                            if !open { followup = None; }
                        },
                        "utterance" => {
                            let current = session.as_mut().ok_or("Utterance without wake session")?;
                            let purpose = message["purpose"].as_str().ok_or("Missing utterance purpose")?.to_owned();
                            let size = message["bytes"].as_u64().ok_or("Missing utterance length")?;
                            current.accept(sid, &purpose, size)?;
                            let audio = tokio::select! {
                                _ = stop.changed() => return Ok(()),
                                result = tokio::time::timeout(Duration::from_secs(5), pi.receive(&mut input)) => result.map_err(|_| "Pi audio timed out")??,
                            };
                            let Message::Binary(pcm) = audio else { return Err("Expected Pi PCM16 bytes".into()); };
                            if pcm.len() as u64 != size { return Err("Pi PCM16 length mismatch".into()); }
                            *next_request += 1;
                            event(hub, sid, json!({"type":"transcription.started", "purpose":purpose, "captureMs":message["captureMs"]}));
                            let speech = speech.clone(); let sid = sid.to_owned();
                            jobs.spawn(async move {
                                let started = Instant::now();
                                let transcript = speech.transcribe(pcm.to_vec()).await?;
                                Ok(Completed::Transcript { sid, purpose, transcript, elapsed:started.elapsed().as_secs_f64()*1000. })
                            });
                        },
                        "session.expired" => {
                            if session.as_ref().map(|session| session.id.as_str()) != Some(sid) { return Err("Stale Pi expiry".into()); }
                            jobs.abort_all(); while jobs.join_next().await.is_some() {}
                            gateway.cancel(&active).await;
                            event(hub, sid, error("session_expired", "Orion voice session timed out or was muted.", true));
                            session = None; followup = None;
                        },
                        _ => return Err("Unknown Pi event".into()),
                    }
                }
            }
        }
    }.await;
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    gateway.cancel(&active).await;
    if let Some(session) = session {
        let _ = pi
            .send(json!({"type":"session.cancel", "sessionId":session.id}))
            .await;
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn response(
    agent: &AgentHandle,
    speech: &SpeechRuntime,
    gateway: &Gateway,
    pi: &Pi,
    hub: &Hub,
    sid: &str,
    request: u64,
    command: &str,
    active: ActiveRun,
    tool_feedback: bool,
) -> Result<(), String> {
    event(
        hub,
        sid,
        json!({"type":"agent.started", "requestId":request}),
    );
    let started = Instant::now();
    let (send, mut receive) = mpsc::channel(8);
    let activity = async {
        while let Some(activity) = receive.recv().await {
            match activity {
                orion_agent::AgentEvent::SearchStarted => {
                    event(
                        hub,
                        sid,
                        json!({"type":"agent.progress","requestId":request,"message":"Searching the web"}),
                    );
                    if tool_feedback {
                        speak(
                            speech,
                            gateway,
                            pi,
                            hub,
                            sid,
                            request,
                            "I’ll search for that now.".into(),
                            active.clone(),
                            true,
                        )
                        .await?;
                        pi.send(json!({"type":"session.processing","sessionId":sid}))
                            .await?;
                    }
                }
                orion_agent::AgentEvent::SetLighting { parameters, reply } => {
                    if reply.is_closed() {
                        continue;
                    }
                    let result = gateway.set_lighting(parameters).await;
                    let _ = reply.send(result);
                }
            }
        }
        Ok::<(), String>(())
    };
    let (text, ()) = tokio::try_join!(agent.respond_with_events(command, Some(send)), activity)?;
    event(
        hub,
        sid,
        json!({"type":"agent.response", "requestId":request, "text":text, "durationMs":started.elapsed().as_secs_f64()*1000.}),
    );
    speak(speech, gateway, pi, hub, sid, request, text, active, false).await
}

#[allow(clippy::too_many_arguments)]
async fn speak(
    speech: &SpeechRuntime,
    gateway: &Gateway,
    pi: &Pi,
    hub: &Hub,
    sid: &str,
    request: u64,
    text: String,
    active: ActiveRun,
    intermediate: bool,
) -> Result<(), String> {
    if !intermediate {
        event(
            hub,
            sid,
            json!({"type":"synthesis.started", "requestId":request}),
        );
    }
    let (run_send, mut run_receive) = watch::channel(None::<u64>);
    let (end_send, mut end_receive) = watch::channel(false);
    let synthesize = async {
        let (send, receive) = mpsc::channel(8);
        let produce = speech.synthesize(text, send);
        let upload = upload_chunks(receive, gateway, &active, sid, request, hub, run_send);
        let (synthesis_ms, (run_id, sequence)) = tokio::try_join!(produce, upload)?;
        gateway
            .request(
                &format!("/api/v2/speech/{run_id}/end"),
                Some(json!({"sequence":sequence})),
            )
            .await?;
        let _ = end_send.send(true);
        timing(hub, sid, "synthesisTotalMs", synthesis_ms);
        Ok::<(), String>(())
    };
    let playback = async {
        let run = loop {
            if let Some(run) = *run_receive.borrow_and_update() {
                break run;
            }
            run_receive
                .changed()
                .await
                .map_err(|_| "Synthesis stopped before playback")?;
        };
        tokio::time::timeout(Duration::from_secs(150), async {
            let mut playing = false;
            loop {
                let status = gateway
                    .request(&format!("/api/v2/speech/{run}"), None)
                    .await?;
                if let Some(ms) = status["first_playback_ms"].as_f64()
                    && !playing
                {
                    if !ms.is_finite() || ms < 0. {
                        return Err("Invalid Pi playback timing".into());
                    }
                    playing = true;
                    pi.send(json!({"type":"session.playing", "sessionId":sid}))
                        .await?;
                    if !intermediate {
                        event(
                            hub,
                            sid,
                            json!({"type":"speech.started", "requestId":request}),
                        );
                    }
                    timing(hub, sid, "firstPlaybackMs", ms);
                }
                match status["state"].as_str() {
                    Some("completed") => {
                        if !playing {
                            return Err("Pi completed speech without playback evidence".into());
                        }
                        end_receive
                            .wait_for(|ended| *ended)
                            .await
                            .map_err(|_| "Synthesis did not finish")?;
                        *active.lock().await = None;
                        return Ok(());
                    }
                    Some("failed" | "cancelled") => {
                        return Err(format!("Pi playback stopped: {}", status["error"]));
                    }
                    Some("queued" | "playing") => {}
                    _ => return Err("Unknown Pi speech state".into()),
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| "Pi playback timed out")?
    };
    tokio::try_join!(synthesize, playback)?;
    Ok(())
}

async fn upload_chunks(
    mut receive: mpsc::Receiver<Option<Chunk>>,
    gateway: &Gateway,
    active: &ActiveRun,
    sid: &str,
    request: u64,
    hub: &Hub,
    run_send: watch::Sender<Option<u64>>,
) -> Result<(u64, u64), String> {
    let mut buffer = StartupBuffer::default();
    let mut held = Vec::new();
    let mut released = false;
    let mut sequence = 0u64;
    let mut run = None;
    loop {
        let item = receive
            .recv()
            .await
            .ok_or("Speech producer closed without an end marker")?;
        let ended = item.is_none();
        let mut ready = ended;
        if let Some(chunk) = item {
            if !released {
                ready = buffer.add(&chunk);
            }
            held.push(chunk);
        }
        if released || ready {
            if !released {
                eprintln!(
                    "{}",
                    json!({"event":"speech.buffer_ready", "request_id":request, "mode":if ended {"complete"} else {"stream"},
                    "audio_ms":buffer.audio*1000., "generation_ms":buffer.generation*1000.})
                );
                released = true;
            }
            for chunk in held.drain(..) {
                let path = match run {
                    None => "/api/v2/speech/stream".into(),
                    Some(run) => format!("/api/v2/speech/{run}/chunks/{sequence}"),
                };
                let started = Instant::now();
                let accepted = gateway
                    .upload(&path, &chunk.pcm, &format!("voice:{sid}"))
                    .await?;
                if run.is_none() {
                    let id = accepted["run_id"]
                        .as_u64()
                        .ok_or("Pi returned no speech run ID")?;
                    *active.lock().await = Some(id);
                    run = Some(id);
                    let _ = run_send.send(run);
                }
                eprintln!(
                    "{}",
                    json!({"event":"speech.chunk", "request_id":request, "run_id":run, "sequence":sequence,
                    "audio_ms":chunk.pcm.len() as f64/48., "generation_ms":chunk.generation_ms, "upload_ms":started.elapsed().as_secs_f64()*1000.})
                );
                if sequence == 0 {
                    timing(hub, sid, "firstChunkMs", chunk.synthesis_ms);
                }
                sequence += 1;
            }
        }
        if ended {
            break;
        }
    }
    Ok((run.ok_or("Synthesis returned no audio")?, sequence))
}
