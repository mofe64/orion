use crate::{
    CoordinatorConfig,
    buffer::StartupBuffer,
    gateway::{ActiveRun, Gateway},
    hub::Hub,
    session::{Phase, Session, after_wake, useful_followup},
    speech::{Chunk, SpeechRuntime},
};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use orion_agent::AgentHandle;
use orion_agent::ReactionCue;
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
#[cfg(test)]
#[path = "microphone_tests.rs"]
mod microphone_tests;
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
    let mut hello =
        json!({"type":"hello", "protocol":2, "token":config.pi_token, "wakePrefix":true});
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
    let (pi, mut input, muted) = tokio::time::timeout(Duration::from_secs(10), async {
        let (pi, mut input, mut status) = connect(config, true).await?;
        if let Some(muted) = muted {
            pi.send(json!({"type":"microphone.mute", "muted":muted}))
                .await?;
            status = decode(pi.receive(&mut input).await?)?;
        }
        let muted = status["muted"]
            .as_bool()
            .ok_or("Invalid Pi microphone status")?;
        Ok::<_, String>((pi, input, muted))
    })
    .await
    .map_err(|_| "Pi microphone control timed out")??;
    // Dropping the split socket without a Close frame makes every status poll
    // look like a failed connection on the Pi. Cleanup has its own deadline so
    // it can't invalidate a mute acknowledged near the request deadline.
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        pi.0.lock().await.send(Message::Close(None)).await?;
        while let Some(message) = input.next().await {
            if matches!(message?, Message::Close(_)) {
                break;
            }
        }
        Ok::<(), tokio_tungstenite::tungstenite::Error>(())
    })
    .await;
    Ok(json!({"type":"microphone.status", "muted":muted}))
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

/// What Orion noticed, in plain words for the reply model. The camera
/// label for an object can be wrong; the picture, when present, decides.
fn remark_prompt(
    message: &Value,
    recent: &std::collections::VecDeque<String>,
) -> Result<String, String> {
    let detail = message["detail"].as_str().unwrap_or("something");
    let event = match message["event"].as_str().ok_or("Remark needs an event")? {
        "person_arrived" => "Someone just came in after the room was empty for a while.".to_owned(),
        "person_looking" => "The person has been looking straight at Orion for a few seconds.".to_owned(),
        "new_object" => format!("Something new was put on the desk. The camera thinks it is: {detail}."),
        "hand_raised" => "The person raised a hand toward Orion. Orion now follows the hand with its head. Invite them to play.".to_owned(),
        "person_left" => "The person just left after spending time at the desk. Say a short goodbye.".to_owned(),
        _ => return Err("Unknown remark event".into()),
    };
    let demo = message["demo"] == true;
    let mut prompt = format!(
        "Nobody spoke to Orion. Orion noticed this on its own: {event}\n\
         Write what Orion says out loud now, in character, to the person: one or two short sentences. \
         {}Never mention cameras, pictures, detection or sensors. Ask at most one question. \
         If there is nothing natural to say, reply with exactly SILENT.",
        if demo {
            "Be lively and playful. A picture of the scene is attached when available; use specific details from it, such as colours or what the person is doing, and stay SILENT if it does not show what was noticed. "
        } else {
            "Keep it light and easy to ignore. "
        }
    );
    if !recent.is_empty() {
        prompt.push_str("\nOrion said these recently; do not repeat them: ");
        prompt.push_str(&recent.iter().map(|line| format!("\"{line}\"")).collect::<Vec<_>>().join(" "));
    }
    Ok(prompt)
}

/// SILENT, or no words at all, means Orion says nothing.
fn remark_text(text: &str) -> Option<String> {
    let text = text.trim();
    let words = response_metadata(text).0;
    let bare: String = words.chars().filter(|c| c.is_alphanumeric()).collect();
    (!bare.is_empty() && !bare.eq_ignore_ascii_case("silent")).then(|| text.to_owned())
}

pub const MORNING_BRIEFING: &str = "Give the user a short morning briefing: greeting and time, weather for today, today's reminders and alarms, and how many items are on the to-do list.";

async fn acknowledge_announcement(gateway: &Gateway, hub: &Hub, sid: &str, id: u64) {
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(100 * attempt)).await;
        }
        match gateway
            .robot_operation(
                json!({"operation":"routines","request":{"action":"announced","id":id}}),
                sid,
            )
            .await
        {
            Ok(_) => return,
            Err(reason) => {
                eprintln!(
                    "{}",
                    json!({"event":"voice.announcement_ack_failed","session_id":sid,"id":id,"attempt":attempt + 1,"reason":reason})
                );
                if attempt == 2 {
                    event(hub, sid, error("announcement_ack_failed", &reason, true));
                }
            }
        }
    }
}

enum Completed {
    Remark {
        text: Option<String>,
    },
    AnnouncementPlayed {
        id: u64,
        attempt: u64,
        reply: Option<orion_agent::AgentReply>,
    },
    Announcement {
        id: u64,
    },
    Transcript {
        sid: String,
        purpose: String,
        transcript: Value,
        elapsed: f64,
    },
    Response {
        sleep: bool,
        reply: orion_agent::AgentReply,
    },
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
        || ready["protocol"] != 2
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
    let mut announcements = std::collections::VecDeque::<Value>::new();
    let mut announced = std::collections::HashMap::new();
    let mut announcement_ready: Option<(String, tokio::sync::oneshot::Sender<Result<(), String>>)> =
        None;
    let mut active_announcement: Option<u64> = None;
    let mut active_remark: Option<u64> = None;
    let mut recent_remarks = std::collections::VecDeque::<String>::new();
    let mut session: Option<Session> = None;
    let mut followup: Option<String> = None;
    let mut jobs = JoinSet::<Result<Completed, String>>::new();
    // Interrupted responses finish their agent calls outside the current voice
    // session. Their completion must never finish/cancel a newer session.
    let mut retiring = JoinSet::<Result<(), String>>::new();
    let mut response_stop: Option<watch::Sender<bool>> = None;
    let mut active: ActiveRun = Arc::new(Mutex::new(None));
    // Explicit cancellation retires the owned turn and any rejected incoming ID.
    // Drain cancelled jobs before accepting the next turn on the same socket.
    macro_rules! cancel_turn {
        ($reason:expr, $incoming:expr) => {{
            let identity = session.as_ref().map(|s| s.id.clone()).or_else(|| followup.clone());
            if let Some(interrupt) = response_stop.take() { let _ = interrupt.send(true); }
            jobs.abort_all(); while jobs.join_next().await.is_some() {}
            gateway.cancel(&active).await;
            active = Arc::new(Mutex::new(None));
            announcement_ready = None;
            active_remark = None;
            if let Some(id) = active_announcement.take() {
                let _ = gateway.robot_operation(json!({"operation":"routines","request":{"action":"announce_failed","id":id}}), $incoming).await;
            }
            if let Some(ref identity) = identity {
                pi.send(json!({"type":"session.cancel","sessionId":identity})).await?;
            }
            if identity.as_deref() != Some($incoming) {
                pi.send(json!({"type":"session.cancel","sessionId":$incoming})).await?;
            }
            eprintln!("{}", json!({"event":"voice.turn_cancelled","session_id":identity,"reason":$reason}));
            session = None; followup = None;
        }};
    }
    let result = async {
        loop {
            if session.is_none() && jobs.is_empty() && retiring.is_empty()
                && announcements.front().is_some_and(|message| message["purpose"] == "remark") {
                    let message = announcements.pop_front().unwrap();
                    let sid = message["sessionId"].as_str().ok_or("Remark needs a session")?.to_owned();
                    let id = message["id"].as_u64().ok_or("Remark needs an ID")?;
                    let prompt = remark_prompt(&message, &recent_remarks)?;
                    let demo = message["demo"] == true;
                    active_remark = Some(id);
                    session = Some(Session::new(&sid, Phase::Responding)?); followup = None;
                    *next_request += 1;
                    let (ready_send, ready_receive) = tokio::sync::oneshot::channel();
                    announcement_ready = Some((sid.clone(), ready_send));
                    pi.send(json!({"type":"announcement.start","sessionId":sid})).await?;
                    let agent = agent.clone();
                    let speech = speech.clone(); let gateway = gateway.clone(); let pi = pi.clone();
                    let hub = hub.clone(); let active = active.clone(); let request = *next_request;
                    jobs.spawn(async move {
                        tokio::time::timeout(Duration::from_secs(5), ready_receive).await.map_err(|_| "Remark session did not open")?.map_err(|_| "Remark session was cancelled")??;
                        event(&hub, &sid, json!({"type":"agent.started", "requestId":request, "remark":true}));
                        let started = Instant::now();
                        // Demo mode lets the model see what Orion noticed. The
                        // gateway shows the camera light while it takes the picture.
                        let image = if demo { gateway.camera_snapshot().await.ok() } else { None };
                        let text = agent.respond_remark(&prompt, image).await?;
                        let text = remark_text(&text);
                        event(&hub, &sid, json!({"type":"agent.response", "requestId":request, "remark":true,
                            "text":text.as_deref().map(|text| response_metadata(text).0), "durationMs":started.elapsed().as_secs_f64()*1000.}));
                        if let Some(text) = &text {
                            speak(&speech,&gateway,&pi,&hub,&sid,request,text.clone(),active,false,true).await?;
                        }
                        Ok(Completed::Remark { text })
                    });
            }
            if session.is_none() && jobs.is_empty() && retiring.is_empty()
                && let Some(message) = announcements.pop_front() {
                    let sid = message["sessionId"].as_str().ok_or("Announcement needs a session")?.to_owned();
                    let id = message["id"].as_u64().ok_or("Announcement needs an alert ID")?;
                    let attempt = message["attempt"].as_u64().ok_or("Announcement needs an attempt")?;
                    let briefing = message["purpose"] == "briefing";
                    let text = message["text"].as_str().ok_or("Announcement needs text")?.to_owned();
                    active_announcement = Some(id);
                    session = Some(Session::new(&sid, Phase::Responding)?); followup = None;
                    *next_request += 1;
                    let (ready_send, ready_receive) = tokio::sync::oneshot::channel();
                    announcement_ready = Some((sid.clone(), ready_send));
                    pi.send(json!({"type":"announcement.start","sessionId":sid})).await?;
                    let agent = agent.clone();
                    let speech = speech.clone(); let gateway = gateway.clone(); let pi = pi.clone();
                    let hub = hub.clone(); let active = active.clone(); let request = *next_request;
                    jobs.spawn(async move {
                        // The listener acknowledges runtime ownership after the cue.
                        tokio::time::timeout(Duration::from_secs(5), ready_receive).await.map_err(|_| "Announcement session did not open")?.map_err(|_| "Announcement session was cancelled")??;
                        let mut delivered_reply = None;
                        if briefing {
                            if !text.ends_with(": ") {
                                speak(&speech,&gateway,&pi,&hub,&sid,request,text,active.clone(),true,true).await?;
                                pi.send(json!({"type":"session.processing","sessionId":sid})).await?;
                            }
                            let (_keep, interrupted) = watch::channel(false);
                            let (_, reply) = response(&agent,&speech,&gateway,&pi,&hub,&sid,request,MORNING_BRIEFING,active,tool_feedback,interrupted,true).await?;
                            delivered_reply = Some(reply);
                        } else {
                            speak(&speech,&gateway,&pi,&hub,&sid,request,text,active,false,true).await?;
                        }
                        Ok(Completed::AnnouncementPlayed { id, attempt, reply: delivered_reply })
                    });
            }
            tokio::select! {
                biased;
                _ = stop.changed() => return Ok(()),
                Some(result) = retiring.join_next(), if !retiring.is_empty() => {
                    if let Err(message) = result.map_err(|e| e.to_string()).and_then(|r| r) {
                        hub.publish(error("interrupted_agent_failed", &message, true));
                    }
                },
                Some(result) = jobs.join_next(), if !jobs.is_empty() => {
                    let current = session.as_mut().ok_or("A job completed without an active session")?;
                    let sid = current.id.clone();
                    match result.map_err(|e| e.to_string())? {
                        Err(message) => {
                            if let Some(id) = active_announcement.take()
                                && let Err(reason) = gateway.robot_operation(json!({"operation":"routines","request":{"action":"announce_failed","id":id}}), &sid).await {
                                event(hub, &sid, error("announcement_failure_report_failed", &reason, true));
                            }
                            announcement_ready = None;
                            gateway.cancel(&active).await;
                            pi.send(json!({"type":"session.cancel", "sessionId":sid})).await?;
                            event(hub, &sid, error("voice_request_failed", &message, true));
                            session = None; followup = None;
                            response_stop = None;
                        },
                        Ok(Completed::Transcript { sid: job_sid, purpose, transcript, elapsed }) => {
                            let expected = if purpose == "wake_prefix" { Phase::VerifyingWake } else { Phase::Transcribing };
                            if job_sid != sid || current.phase != expected {
                                eprintln!("{}", json!({"event":"voice.stale_transcription_ignored","session_id":job_sid,"current_session_id":sid,"reason":"Stale transcription result"}));
                                continue;
                            }
                            timing(hub, &sid, &format!("transcription_{purpose}"), elapsed);
                            let raw = transcript["text"].as_str().ok_or("Invalid transcript text")?.trim();
                            if purpose == "wake_prefix" {
                                let accepted = after_wake(raw).is_some();
                                eprintln!("{}", json!({"event":"voice.wake_outcome","session_id":sid,"accepted":accepted,"qwen_ms":elapsed}));
                                current.wake_verified = accepted;
                                current.phase = Phase::Wake;
                                if !accepted {
                                    pi.send(json!({"type":"session.cancel", "sessionId":sid,"wakeAccepted":false,"qwenMs":elapsed})).await?;
                                    event(hub, &sid, json!({"type":"wake.rejected"}));
                                    session = None; followup = None;
                                    continue;
                                }
                                pi.send(json!({"type":"wake.verified", "sessionId":sid, "accepted":true,"qwenMs":elapsed})).await?;
                                event(hub, &sid, json!({"type":"wake.confirmed", "hasCommand":false, "early":true}));
                                continue;
                            }
                            let command = if purpose == "wake_and_command" {
                                match after_wake(raw).or_else(|| current.wake_verified.then(|| raw.to_owned())) {
                                    None => {
                                        eprintln!("{}", json!({"event":"voice.wake_outcome","session_id":sid,"accepted":false,"qwen_ms":elapsed}));
                                        pi.send(json!({"type":"session.reject", "sessionId":sid})).await?;
                                        event(hub, &sid, json!({"type":"wake.rejected"}));
                                        session = None;
                                        continue;
                                    },
                                    Some(command) => {
                                        current.phase = if command.is_empty() { Phase::Command } else { Phase::Responding };
                                        pi.send(json!({"type":"wake.confirmed", "sessionId":sid, "followup":command.is_empty()})).await?;
                                        if !current.wake_verified {
                                            eprintln!("{}", json!({"event":"voice.wake_outcome","session_id":sid,"accepted":true,"qwen_ms":elapsed}));
                                            event(hub, &sid, json!({"type":"wake.confirmed", "text":raw, "hasCommand":!command.is_empty()}));
                                        }
                                        if command.is_empty() {
                                            event(hub, &sid, json!({"type":"command.started"})); continue;
                                        }
                                        command
                                    }
                                }
                            } else { raw.to_owned() };
                            if current.continuation && !useful_followup(&command) {
                                pi.send(json!({"type":"session.cancel", "sessionId":sid})).await?;
                                event(hub, &sid, json!({"type":"conversation.window","active":false}));
                                session = None; followup = None; continue;
                            }
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
                            let (interrupt, interrupted) = watch::channel(false);
                            response_stop = Some(interrupt);
                            jobs.spawn(async move { response(&agent, &speech, &gateway, &pi, &hub, &sid, request, &command, active, tool_feedback, interrupted, false).await.map(|(sleep,reply)| Completed::Response {sleep,reply}) });
                        },
                        Ok(Completed::AnnouncementPlayed {id, attempt, reply}) => {
                            // Playback is irreversible. Record it before any fallible
                            // acknowledgement, including if wake cancels the retries.
                            active_announcement = None;
                            announcement_ready = None;
                            announced.insert((id, attempt), sid.clone());
                            if let Some(reply) = &reply { agent.confirm_playback(reply); }
                            let gateway = gateway.clone(); let hub = hub.clone();
                            jobs.spawn(async move {
                                acknowledge_announcement(&gateway, &hub, &sid, id).await;
                                Ok(Completed::Announcement { id })
                            });
                        },
                        Ok(Completed::Remark {text}) => {
                            active_remark = None;
                            announcement_ready = None;
                            session = None; followup = None;
                            if let Some(text) = text {
                                agent.note_remark(&text);
                                recent_remarks.push_back(response_metadata(&text).0);
                                if recent_remarks.len() > 5 { recent_remarks.pop_front(); }
                                // Answering a remark needs no wake word.
                                followup = window.then_some(sid.clone());
                                let mut finish = json!({"type":"session.finish", "sessionId":sid});
                                if window { finish["conversationWindow"] = true.into(); }
                                pi.send(finish).await?;
                                event(hub, &sid, json!({"type":"speech.completed", "remark":true}));
                            } else {
                                // Nothing worth saying: close the silent session.
                                pi.send(json!({"type":"session.cancel","sessionId":sid})).await?;
                            }
                        },
                        Ok(Completed::Announcement {id}) => {
                            active_announcement = None;
                            announcement_ready = None;
                            session = None; followup = None;
                            pi.send(json!({"type":"session.finish","sessionId":sid})).await?;
                            event(hub, &sid, json!({"type":"announcement.completed","id":id}));
                        },
                        Ok(Completed::Response {sleep,reply}) => {
                            response_stop = None;
                            let window = window && !sleep;
                            let request = *next_request;
                            followup = window.then_some(sid.clone()); session = None;
                            let mut finish = json!({"type":"session.finish", "sessionId":sid});
                            if window { finish["conversationWindow"] = true.into(); }
                            pi.send(finish).await?;
                            event(hub, &sid, json!({"type":"speech.completed", "requestId":request}));
                            agent.confirm_playback(&reply);
                        }
                    }
                },
                message = pi.receive(&mut input) => {
                    let message = decode(message?)?;
                    if message["type"] == "announcement.ready" || message["type"] == "announcement.failed" {
                        let failed = message["type"] == "announcement.failed";
                        if announcement_ready.as_ref().is_some_and(|(expected, _)| message["sessionId"].as_str() == Some(expected.as_str()))
                            && (!failed || message["id"].as_u64() == active_announcement
                                || (active_remark.is_some() && message["id"].as_u64() == active_remark)) {
                            let (_, send) = announcement_ready.take().unwrap();
                            let result = if failed { Err(message["reason"].as_str().unwrap_or("Announcement ownership failed").to_owned()) } else { Ok(()) };
                            let _ = send.send(result);
                        } else {
                            eprintln!("{}", json!({"event":if failed {"voice.announcement_failure_ignored"} else {"voice.announcement_ready_ignored"},"session_id":message["sessionId"]}));
                        }
                        continue;
                    }
                    let sid = message["sessionId"].as_str().ok_or("Pi event has no session ID")?;
                    match message["type"].as_str().ok_or("Pi event has no type")? {
                        "announcement" => {
                            let id = message["id"].as_u64().ok_or("Announcement needs an alert ID")?;
                            let attempt = message["attempt"].as_u64().ok_or("Announcement needs an attempt")?;
                            if !matches!(message["purpose"].as_str(),Some("announcement"|"briefing")) || message["text"].as_str().is_none_or(|text| text.trim().is_empty() || text.chars().count()>512) || !crate::session::valid_session(sid) { return Err("Invalid announcement".into()); }
                            if let Some(delivered_sid) = announced.get(&(id,attempt)) {
                                // Release a fresh listener reservation for speech
                                // already delivered, without replaying it.
                                if delivered_sid != sid { pi.send(json!({"type":"session.finish","sessionId":sid})).await?; }
                            } else if !announcements.iter().any(|queued| queued["id"]==id && queued["attempt"]==attempt) && !session.as_ref().is_some_and(|s| s.id==sid) {
                                if announcements.len()>=64 { return Err("Announcement queue is full".into()); }
                                announcements.push_back(message);
                            }
                        },
                        "remark" => {
                            let valid = message["id"].as_u64().is_some()
                                && matches!(message["event"].as_str(), Some("person_arrived"|"person_looking"|"new_object"|"hand_raised"|"person_left"))
                                && message["demo"].is_boolean()
                                && (message["detail"].is_null() || message["detail"].as_str().is_some_and(|detail| !detail.is_empty() && detail.chars().count() <= 32))
                                && crate::session::valid_session(sid);
                            if !valid { return Err("Invalid remark".into()); }
                            if session.is_some() || !jobs.is_empty() || announcements.len() >= 64 {
                                // Remarks never wait behind other speech; release the reservation.
                                pi.send(json!({"type":"session.cancel","sessionId":sid})).await?;
                            } else {
                                let mut message = message.clone();
                                message["purpose"] = "remark".into();
                                announcements.push_front(message);
                            }
                        },
                        "wake.candidate" | "command.candidate" => {
                            if session.is_some() || !jobs.is_empty() {
                                // Retire only the old owner, then admit this candidate.
                                let old_sid = session.as_ref().map(|s| s.id.clone()).or_else(|| followup.clone())
                                    .ok_or("Voice jobs without an active session")?;
                                cancel_turn!("Overlapping Pi voice session", old_sid.as_str());
                            }
                            let continuation = message["type"] == "command.candidate";
                            if continuation && (followup.as_deref() != message["previousSessionId"].as_str() || followup.is_none() || followup.as_deref() == Some(sid)) {
                                cancel_turn!("Unexpected conversation continuation", sid);
                                continue;
                            }
                            session = Some(Session::new(sid, if continuation { Phase::Command } else { Phase::Wake })?);
                            session.as_mut().unwrap().continuation = continuation;
                            followup = None;
                            if continuation { event(hub, sid, json!({"type":"command.started"})); } else { hub.publish(message); }
                        },
                        "wake.verified" => {
                            // The listener's acoustic verifier decided before any ASR ran.
                            let current = session.as_mut().ok_or("Acoustic verdict without wake session")?;
                            let accepted = message["accepted"].as_bool().ok_or("Acoustic verdict needs accepted")?;
                            current.acoustic_verdict(sid, accepted)?;
                            event(hub, sid, if accepted {
                                json!({"type":"wake.confirmed", "text":"", "hasCommand":false, "early":true,
                                       "source":"acoustic", "score":message["score"], "verifierMs":message["verifierMs"]})
                            } else {
                                json!({"type":"wake.rejected", "text":"", "source":"acoustic", "score":message["score"], "verifierMs":message["verifierMs"]})
                            });
                            if !accepted {
                                // No ASR job starts until the acoustic verdict.
                                // Retire this candidate before the next wake arrives.
                                session = None; followup = None;
                                pi.send(json!({"type":"session.cancel", "sessionId":sid})).await?;
                            }
                        },
                        "conversation.ready" | "conversation.closed" => {
                            if session.as_ref().map(|s| s.id.as_str()) != Some(sid) && followup.as_deref() != Some(sid) {
                                eprintln!("{}", json!({"event":"voice.stale_window_ignored","session_id":sid,"type":message["type"],"reason":"Stale conversation window event"}));
                                continue;
                            }
                            let open = message["type"] == "conversation.ready";
                            hub.publish(json!({"type":"conversation.window", "active":open}));
                            if !open { followup = None; }
                        },
                        "utterance" => {
                            let purpose = message["purpose"].as_str().ok_or("Missing utterance purpose")?.to_owned();
                            let size = message["bytes"].as_u64().ok_or("Missing utterance length")?;
                            let admission = session.as_mut().ok_or_else(|| "Utterance without wake session".to_owned())
                                .and_then(|current| current.accept(sid, &purpose, size));
                            let audio = tokio::select! {
                                _ = stop.changed() => return Ok(()),
                                result = tokio::time::timeout(Duration::from_secs(5), pi.receive(&mut input)) => result.map_err(|_| "Pi audio timed out")??,
                            };
                            let Message::Binary(pcm) = audio else { return Err("Expected Pi PCM16 bytes".into()); };
                            if pcm.len() as u64 != size { return Err("Pi PCM16 length mismatch".into()); }
                            if let Err(reason) = admission {
                                cancel_turn!(&reason, sid);
                                continue;
                            }
                            if message["endReason"] == "max_duration" {
                                pi.send(json!({"type":"session.cancel", "sessionId":sid})).await?;
                                event(hub, sid, error("utterance_too_long", "The recording limit was reached. Please repeat a shorter request.", true));
                                session = None; followup = None;
                                continue;
                            }
                            *next_request += 1;
                            event(hub, sid, json!({"type":"transcription.started", "purpose":purpose, "captureMs":message["captureMs"]}));
                            let speech = speech.clone(); let sid = sid.to_owned();
                            let resources = gateway.voice_resources.clone();
                            jobs.spawn(async move {
                                let _resources = resources.lock().await;
                                let started = Instant::now();
                                let transcript = speech.transcribe(pcm.to_vec()).await?;
                                Ok(Completed::Transcript { sid, purpose, transcript, elapsed:started.elapsed().as_secs_f64()*1000. })
                            });
                        },
                        "session.interrupted" => {
                            // Waiting announcements can be cancelled while another reply retires.
                            let queued = announcements.iter().any(|a| a["sessionId"] == sid);
                            announcements.retain(|a| a["sessionId"] != sid);
                            if queued && session.as_ref().map(|s| s.id.as_str()) != Some(sid) { continue; }
                            if session.as_ref().map(|session| session.id.as_str()) != Some(sid) && followup.as_deref() != Some(sid) { continue; }
                            let reason = message["reason"].as_str().ok_or("Interruption needs a reason")?;
                            match reason {
                                "barge_in" => {
                                    let responding = session.as_ref().is_some_and(|s| s.phase == Phase::Responding);
                                    if let Some(interrupt) = response_stop.take() { let _ = interrupt.send(true); }
                                    let mut interrupted_jobs = std::mem::take(&mut jobs);
                                    if !responding { interrupted_jobs.abort_all(); }
                                    retiring.spawn(async move {
                                        while let Some(result) = interrupted_jobs.join_next().await {
                                            result.map_err(|e| e.to_string())??;
                                        }
                                        Ok(())
                                    });
                                    if responding { agent.report_interruption(); }
                                },
                                "alarm" | "announcement" | "announcement_wake" => {
                                    jobs.abort_all(); while jobs.join_next().await.is_some() {}
                                    retiring.abort_all(); while retiring.join_next().await.is_some() {}
                                    response_stop = None;
                                },
                                _ => return Err("Unknown interruption reason".into()),
                            }
                            announcement_ready = None;
                            active_announcement = None;
                            gateway.cancel(&active).await;
                            active = Arc::new(Mutex::new(None));
                            event(hub, sid, json!({"type":"session.interrupted","reason":reason}));
                            session = None; followup = None;
                        },
                        "session.cancel" => {
                            if session.as_ref().is_some_and(|s| s.id == sid) || followup.as_deref() == Some(sid) {
                                cancel_turn!("Listener cancelled turn", sid);
                            }
                        },
                        "session.expired" => {
                            if let Some(index) = announcements.iter().position(|a| a["sessionId"] == sid) {
                                let expired = announcements.remove(index).unwrap();
                                let _ = gateway.robot_operation(json!({"operation":"routines","request":{"action":"announce_failed","id":expired["id"]}}), sid).await;
                                continue;
                            }
                            if session.as_ref().map(|session| session.id.as_str()) != Some(sid) && followup.as_deref() != Some(sid) {
                                eprintln!("{}", json!({"event":"voice.stale_expiry_ignored","session_id":sid,"reason":"Stale Pi expiry"}));
                                continue;
                            }
                            announcement_ready = None;
                            if let Some(id) = active_announcement.take() {
                                let _ = gateway.robot_operation(json!({"operation":"routines","request":{"action":"announce_failed","id":id}}), sid).await;
                            }
                            jobs.abort_all(); while jobs.join_next().await.is_some() {}
                            retiring.abort_all(); while retiring.join_next().await.is_some() {}
                            response_stop = None;
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
    retiring.abort_all();
    while retiring.join_next().await.is_some() {}
    gateway.cancel(&active).await;
    if let Some(session) = session {
        event(
            hub,
            &session.id,
            json!({"type":"session.cancelled","reason":"coordinator stopped or disconnected"}),
        );
        let _ = pi
            .send(json!({"type":"session.cancel", "sessionId":session.id}))
            .await;
    }
    result
}

#[derive(Debug, PartialEq)]
enum SpeechPiece {
    Text(String),
    Cue(ReactionCue),
}
fn speech_pieces(text: &str) -> Vec<SpeechPiece> {
    let mut pieces = Vec::new();
    let mut rest = text;
    let mut offset = 0;
    while let Some(start) = rest[offset..].find('[').map(|i| i + offset) {
        let Some(end) = rest[start..].find(']').map(|i| i + start) else {
            break;
        };
        if let Some(cue) = ReactionCue::parse(&rest[start + 1..end]) {
            if !rest[..start].trim().is_empty() {
                pieces.push(SpeechPiece::Text(rest[..start].trim().into()));
            }
            pieces.push(SpeechPiece::Cue(cue));
            rest = &rest[end + 1..];
            offset = 0;
        } else {
            offset = end + 1;
        }
    }
    if !rest.trim().is_empty() {
        pieces.push(SpeechPiece::Text(rest.trim().into()));
    }
    pieces
}
fn response_metadata(text: &str) -> (String, Vec<&'static str>) {
    let mut words = Vec::new();
    let mut cues = Vec::new();
    for piece in speech_pieces(text) {
        match piece {
            SpeechPiece::Text(text) => words.push(text),
            SpeechPiece::Cue(cue) => cues.push(cue.as_str()),
        }
    }
    (words.join(" "), cues)
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
    interrupted: watch::Receiver<bool>,
    system: bool,
) -> Result<(bool, orion_agent::AgentReply), String> {
    event(
        hub,
        sid,
        json!({"type":"agent.started", "requestId":request}),
    );
    let sleep_requested = std::sync::atomic::AtomicBool::new(false);
    let started = Instant::now();
    let (send, mut receive) = mpsc::channel(8);
    // Text is bounded by the agent's streamed-answer/event guards. Queue it
    // independently so slow inference cannot consume the agent turn deadline.
    let (text_send, mut text_receive) = mpsc::unbounded_channel();
    // response() starts after ASR has completed. An explicit lookup owns this
    // gate until the gateway has completed movement and closed its voice lease.
    // Speech holds it from first synthesis through measured playback completion.
    let vision_gate = &gateway.voice_resources;
    let streamed_text = text_send.clone();
    let activity = async {
        let text_send = streamed_text;
        let mut prefix = String::new();
        while let Some(activity) = receive.recv().await {
            match activity {
                orion_agent::AgentEvent::CameraSnapshot { reply } => {
                    if *interrupted.borrow() || reply.is_closed() {
                        let _ = reply.send(Err("The visual request was interrupted".into()));
                        continue;
                    }
                    let result = gateway.camera_snapshot().await;
                    // A cancellation during HTTP delivery must not attach the
                    // completed frame to an interrupted agent request.
                    let result = if *interrupted.borrow() {
                        Err("The visual request was interrupted".into())
                    } else {
                        result
                    };
                    let _ = reply.send(result);
                }
                orion_agent::AgentEvent::ToolCall {
                    name,
                    arguments,
                    result,
                    success,
                    duration_ms,
                } => {
                    event(
                        hub,
                        sid,
                        json!({"type":"agent.tool", "requestId":request,
                        "name":name, "arguments":arguments, "result":result, "success":success, "durationMs":duration_ms}),
                    );
                }
                orion_agent::AgentEvent::FinalSpeech(text) => {
                    if !prefix.is_empty() {
                        prefix.push(' ');
                    }
                    prefix.push_str(&text);
                    if !*interrupted.borrow() {
                        text_send
                            .send(text)
                            .map_err(|_| "Speech renderer stopped")?;
                    }
                }
                orion_agent::AgentEvent::SearchStarted => {
                    event(
                        hub,
                        sid,
                        json!({"type":"agent.progress","requestId":request,"message":"Searching the web"}),
                    );
                    if tool_feedback
                        && !*interrupted.borrow()
                        && let Ok(_voice_interval) = vision_gate.try_lock()
                    {
                        let mut stopped = interrupted.clone();
                        tokio::select! {
                            biased;
                            _ = stopped.wait_for(|stopped| *stopped) => {},
                            result = speak(
                                speech,
                                gateway,
                                pi,
                                hub,
                                sid,
                                request,
                                "I’ll search for that now.".into(),
                                active.clone(),
                                true,
                                false,
                            ) => result?,
                        }
                        if !*interrupted.borrow() {
                            pi.send(json!({"type":"session.processing","sessionId":sid}))
                                .await?;
                        }
                    }
                }
                orion_agent::AgentEvent::RobotOperation { parameters, reply } => {
                    if *interrupted.borrow() {
                        let _ = reply.send(Err("The previous voice reply was interrupted".into()));
                        continue;
                    }
                    if reply.is_closed() {
                        continue;
                    }
                    let sleep = parameters["operation"] == "sleep";
                    let mode = parameters["request"]["action"] == "set_mode";
                    let result = if parameters["operation"] == "aim_light" {
                        gateway.aim_light(parameters, sid).await
                    } else {
                        gateway.robot_operation(parameters, sid).await
                    };
                    if sleep && result.is_ok() {
                        sleep_requested.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    if mode && result.is_ok() {
                        // A later mode selection supersedes a queued sleep.
                        sleep_requested.store(false, std::sync::atomic::Ordering::Relaxed);
                    }
                    let _ = reply.send(result);
                }
                orion_agent::AgentEvent::SetLighting { parameters, reply } => {
                    if *interrupted.borrow() {
                        let _ = reply.send(Err("The previous voice reply was interrupted".into()));
                        continue;
                    }
                    if reply.is_closed() {
                        continue;
                    }
                    let result = gateway.set_lighting(parameters).await;
                    let _ = reply.send(result);
                }
            }
        }
        Ok::<String, String>(prefix)
    };
    let agent_turn = async {
        let generation = async {
            if system {
                agent.respond_system_for_playback(command, Some(send)).await
            } else {
                agent.respond_for_session(sid, command, Some(send)).await
            }
        };
        let (reply, prefix) = tokio::try_join!(generation, activity)?;
        let text = &reply.text;
        let remaining = text
            .strip_prefix(&prefix)
            .ok_or("Final answer changed streamed speech")?
            .trim();
        if !remaining.is_empty() && !*interrupted.borrow() {
            text_send
                .send(remaining.into())
                .map_err(|_| "Speech renderer stopped")?;
        }
        drop(text_send);
        let (text, cues) = response_metadata(text);
        event(
            hub,
            sid,
            json!({"type":"agent.response", "requestId":request, "text":text, "cues":cues, "durationMs":started.elapsed().as_secs_f64()*1000.}),
        );
        Ok::<orion_agent::AgentReply, String>(reply)
    };
    let spoken = async {
        let mut stopped = interrupted.clone();
        tokio::select! {
            biased;
            _ = stopped.wait_for(|stopped| *stopped) => Ok(()),
            result = async {
                // Do not lock while waiting for the agent's first final text:
                // that agent may be waiting for an aim_light tool result.
                let first = text_receive.recv().await.ok_or("No final speech was produced")?;
                let _voice_interval = vision_gate.lock().await;
                let (send, receive) = mpsc::unbounded_channel();
                send.send(first).map_err(|_| "Speech renderer stopped")?;
                let forward = async {
                    while let Some(text) = text_receive.recv().await {
                        send.send(text).map_err(|_| "Speech renderer stopped")?;
                    }
                    drop(send);
                    Ok::<(), String>(())
                };
                tokio::try_join!(forward, speak_sequence(speech, gateway, pi, hub, sid, request,
                    receive, active.clone(), false)).map(|_| ())
            } => result,
        }
    };
    let result = tokio::try_join!(agent_turn, spoken);
    if let Err(reason) = &result
        && reason.contains("30-minute audio sanity limit")
    {
        agent.report_delivery_failure(reason);
    }
    let (reply, ()) = result?;
    Ok((
        sleep_requested.load(std::sync::atomic::Ordering::Relaxed),
        reply,
    ))
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
    acquire_resources: bool,
) -> Result<(), String> {
    let _resources = if acquire_resources {
        Some(gateway.voice_resources.lock().await)
    } else {
        None
    };
    let (send, receive) = mpsc::unbounded_channel();
    send.send(text).map_err(|_| "Speech renderer stopped")?;
    drop(send);
    speak_sequence(
        speech,
        gateway,
        pi,
        hub,
        sid,
        request,
        receive,
        active,
        intermediate,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn speak_sequence(
    speech: &SpeechRuntime,
    gateway: &Gateway,
    pi: &Pi,
    hub: &Hub,
    sid: &str,
    request: u64,
    mut texts: mpsc::UnboundedReceiver<String>,
    active: ActiveRun,
    intermediate: bool,
) -> Result<(), String> {
    let (run_send, mut run_receive) = watch::channel(None::<u64>);
    let (end_send, mut end_receive) = watch::channel(false);
    let (progress_send, progress_receive) = watch::channel(UploadProgress::default());
    let (synthesis_started, synthesis_ready) = watch::channel(false);
    let synthesize = async {
        let (send, receive) = mpsc::channel(8);
        let produce = async {
            let mut started = None;
            let mut pending_cues = Vec::new();
            while let Some(text) = texts.recv().await {
                if started.is_none() {
                    started = Some(Instant::now());
                    let _ = synthesis_started.send(true);
                    if !intermediate {
                        event(
                            hub,
                            sid,
                            json!({"type":"synthesis.started", "requestId":request}),
                        );
                    }
                }
                for piece in speech_pieces(&text) {
                    let text = match piece {
                        SpeechPiece::Cue(cue) => {
                            pending_cues.push(cue);
                            continue;
                        }
                        SpeechPiece::Text(text) => text,
                    };
                    let (chunk_send, mut chunk_receive) = mpsc::channel(8);
                    let job = speech.synthesize(text, chunk_send);
                    let forward = async {
                        while let Some(Some(mut chunk)) = chunk_receive.recv().await {
                            let cues = std::mem::take(&mut pending_cues);
                            chunk.cues = cues.first().copied().into_iter().collect();
                            for cue in cues.into_iter().skip(1) {
                                eprintln!(
                                    "{}",
                                    json!({"event":"speech.cue_dropped", "cue":cue.as_str(), "reason":"crowded", "request_id":request})
                                );
                            }
                            chunk.synthesis_ms = started.unwrap().elapsed().as_secs_f64() * 1000.;
                            send.send(Some(chunk))
                                .await
                                .map_err(|_| "Speech upload stopped")?;
                        }
                        Ok::<(), String>(())
                    };
                    tokio::try_join!(job, forward)?;
                }
            }
            for cue in pending_cues {
                eprintln!(
                    "{}",
                    json!({"event":"speech.cue_dropped", "cue":cue.as_str(), "reason":"trailing", "request_id":request})
                );
            }
            let elapsed = started
                .ok_or("No final speech was produced")?
                .elapsed()
                .as_secs_f64()
                * 1000.;
            send.send(None).await.map_err(|_| "Speech upload stopped")?;
            Ok::<f64, String>(elapsed)
        };
        let upload = upload_chunks(
            receive,
            gateway,
            &active,
            sid,
            request,
            hub,
            run_send,
            progress_send,
        );
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
        {
            let mut playing = false;
            let mut progress = PlaybackProgress::new(Instant::now());
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
                let uploaded = *progress_receive.borrow();
                let buffered = status["buffered_ms"].as_i64();
                // buffered_ms is a software estimate. Clamp at zero so elapsed
                // wall time cannot disguise a player stuck after its expected end.
                let played_ms =
                    buffered.map(|ms| uploaded.audio_ms.saturating_sub(ms.max(0) as u64));
                let waiting_for_synthesis =
                    !*end_receive.borrow() && buffered.is_some_and(|ms| ms <= 0);
                if progress.stalled(
                    Instant::now(),
                    status["state"].as_str().unwrap_or_default(),
                    uploaded.chunks,
                    played_ms,
                    waiting_for_synthesis,
                ) {
                    return Err("Pi playback stalled for 20 seconds".into());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    };
    let keep_alive = async {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            // A final sequence waiting for agent text must not overwrite the
            // phase of a concurrently spoken search acknowledgement.
            if !*synthesis_ready.borrow() {
                continue;
            }
            pi.send(json!({"type":"session.keepalive", "sessionId":sid}))
                .await?;
        }
        #[allow(unreachable_code)]
        Ok::<(), String>(())
    };
    tokio::select! {
        result = async { tokio::try_join!(synthesize, playback).map(|_| ()) } => result,
        result = keep_alive => result,
    }
}

#[allow(clippy::too_many_arguments)]
async fn upload_chunks(
    mut receive: mpsc::Receiver<Option<Chunk>>,
    gateway: &Gateway,
    active: &ActiveRun,
    sid: &str,
    request: u64,
    hub: &Hub,
    run_send: watch::Sender<Option<u64>>,
    progress_send: watch::Sender<UploadProgress>,
) -> Result<(u64, u64), String> {
    let mut buffer = StartupBuffer::default();
    let mut audio_bytes = 0;
    let mut held = Vec::new();
    let mut released = false;
    let mut sequence = 0u64;
    let mut run = None;
    let mut queued_complete_burst = false;
    loop {
        let item = receive
            .recv()
            .await
            .ok_or("Speech producer closed without an end marker")?;
        let ended = item.is_none();
        let mut ready = ended;
        if let Some(chunk) = item {
            // Check before retaining PCM, including complete-buffered replies
            // that have not created a runtime run yet.
            audio_bytes = checked_audio_total(audio_bytes, chunk.pcm.len())?;
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
                if let Some(id) = run {
                    // Keep PCM ahead of software playback bounded. This is
                    // backpressure, not a duration limit; synthesis queues wait.
                    loop {
                        let status = gateway
                            .request(&format!("/api/v2/speech/{id}"), None)
                            .await?;
                        if matches!(status["state"].as_str(), Some("failed" | "cancelled")) {
                            return Err(format!("Pi playback stopped: {}", status["error"]));
                        }
                        // Finish a complete queued burst even if playback starts
                        // midway: its accumulated lead can take longer than the
                        // upload-idle guard to drain to the pacing threshold.
                        queued_complete_burst |= ended && status["state"] == "queued";
                        if queued_complete_burst
                            || !status["buffered_ms"].as_i64().is_some_and(|ms| ms > 16_000)
                        {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
                let started = Instant::now();
                let accepted = gateway
                    .upload(&path, &chunk.pcm, &format!("voice:{sid}"), &chunk.cues)
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
                    "cues":chunk.cues.iter().map(|c| c.as_str()).collect::<Vec<_>>(), "audio_ms":chunk.pcm.len() as f64/48., "generation_ms":chunk.generation_ms, "upload_ms":started.elapsed().as_secs_f64()*1000.})
                );
                if sequence == 0 {
                    timing(hub, sid, "firstChunkMs", chunk.synthesis_ms);
                }
                sequence += 1;
                progress_send.send_modify(|progress| {
                    progress.chunks = sequence;
                    progress.audio_ms += chunk.pcm.len() as u64 / 48;
                });
            }
        }
        if ended {
            break;
        }
    }
    Ok((run.ok_or("Synthesis returned no audio")?, sequence))
}

const MAX_REPLY_AUDIO_BYTES: u64 = 30 * 60 * 48_000;
const SPEECH_LIMIT_REASON: &str =
    "Speech stream exceeds the 30-minute audio sanity limit; playback cannot complete this answer.";

fn checked_audio_total(received: u64, chunk_bytes: usize) -> Result<u64, String> {
    received
        .checked_add(chunk_bytes as u64)
        .filter(|total| *total <= MAX_REPLY_AUDIO_BYTES)
        .ok_or_else(|| SPEECH_LIMIT_REASON.into())
}

// A reply can play indefinitely while it progresses. Waiting for synthesis has
// its own per-job guard and must not consume the playback stall budget.
const PLAYBACK_STALL_TIMEOUT: Duration = Duration::from_secs(20);
#[derive(Clone, Copy, Default)]
struct UploadProgress {
    chunks: u64,
    audio_ms: u64,
}
struct PlaybackProgress {
    last: Instant,
    observed: Option<(String, u64, Option<u64>)>,
}
impl PlaybackProgress {
    fn new(now: Instant) -> Self {
        Self {
            last: now,
            observed: None,
        }
    }
    fn stalled(
        &mut self,
        now: Instant,
        state: &str,
        chunks: u64,
        played_ms: Option<u64>,
        waiting: bool,
    ) -> bool {
        let observed = (state.to_owned(), chunks, played_ms);
        if waiting || self.observed.as_ref() != Some(&observed) {
            self.last = now;
        }
        self.observed = Some(observed);
        now.duration_since(self.last) >= PLAYBACK_STALL_TIMEOUT
    }
}

#[cfg(test)]
mod playback_tests {
    use super::*;
    #[test]
    fn remarks_describe_the_event_and_silent_means_nothing_is_said() {
        let mut recent = std::collections::VecDeque::new();
        let message = json!({"event":"new_object","detail":"banana","demo":true});
        let prompt = remark_prompt(&message, &recent).unwrap();
        assert!(prompt.contains("banana") && prompt.contains("attached"));
        recent.push_back("Is that lunch?".to_owned());
        let prompt = remark_prompt(&json!({"event":"person_left","demo":false}), &recent).unwrap();
        assert!(prompt.contains("goodbye") && prompt.contains("\"Is that lunch?\"") && !prompt.contains("attached"));
        assert!(remark_prompt(&json!({"event":"other","demo":false}), &recent).is_err());
        for silent in ["", "  SILENT ", "Silent.", "[laugh]"] {
            assert_eq!(remark_text(silent), None, "{silent}");
        }
        assert_eq!(remark_text(" Oh, hello! ").as_deref(), Some("Oh, hello!"));
    }

    #[test]
    fn every_enabled_cue_stays_out_of_spoken_text_and_reply_history() {
        for cue in ReactionCue::enabled() {
            let tagged = format!("[{}] Hello.", cue.as_str());
            assert_eq!(
                speech_pieces(&tagged),
                vec![SpeechPiece::Cue(*cue), SpeechPiece::Text("Hello.".into())]
            );
            assert_eq!(
                response_metadata(&tagged),
                ("Hello.".into(), vec![cue.as_str()])
            );
        }
    }

    #[test]
    fn retained_audio_accepts_exactly_thirty_minutes_and_rejects_more() {
        assert_eq!(
            checked_audio_total(MAX_REPLY_AUDIO_BYTES - 96_000, 96_000).unwrap(),
            MAX_REPLY_AUDIO_BYTES
        );
        assert_eq!(
            checked_audio_total(MAX_REPLY_AUDIO_BYTES, 2).unwrap_err(),
            SPEECH_LIMIT_REASON
        );
        assert_eq!(
            checked_audio_total(u64::MAX, 2).unwrap_err(),
            SPEECH_LIMIT_REASON
        );
    }

    #[test]
    fn long_playback_progress_and_synthesis_waits_do_not_time_out() {
        let start = Instant::now();
        let mut progress = PlaybackProgress::new(start);
        for seconds in 0..400 {
            assert!(!progress.stalled(
                start + Duration::from_secs(seconds),
                "playing",
                100,
                Some(seconds * 1000),
                false
            ));
        }
        for seconds in 400..700 {
            assert!(!progress.stalled(
                start + Duration::from_secs(seconds),
                "playing",
                100,
                Some(399000),
                true
            ));
        }
        assert!(!progress.stalled(
            start + Duration::from_secs(700),
            "playing",
            101,
            Some(399000),
            false
        ));
        assert!(!progress.stalled(
            start + Duration::from_secs(719),
            "playing",
            101,
            Some(399000),
            false
        ));
        assert!(progress.stalled(
            start + Duration::from_secs(720),
            "playing",
            101,
            Some(399000),
            false
        ));
    }
    #[test]
    fn state_changes_or_new_accepted_chunks_reset_the_stall_clock() {
        let start = Instant::now();
        let mut progress = PlaybackProgress::new(start);
        assert!(!progress.stalled(start, "queued", 1, None, false));
        assert!(!progress.stalled(start + Duration::from_secs(19), "playing", 1, None, false));
        assert!(!progress.stalled(start + Duration::from_secs(38), "playing", 2, None, false));
        assert!(progress.stalled(start + Duration::from_secs(58), "playing", 2, None, false));
    }
}
