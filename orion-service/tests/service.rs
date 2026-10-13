use futures_util::{SinkExt, StreamExt};
use orion_service::{Request, StartOptions, rpc};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};
use tokio_tungstenite::{WebSocketStream, accept_async, connect_async, tungstenite::Message};

struct Process {
    child: Child,
    root: tempfile::TempDir,
    directory: PathBuf,
    release: PathBuf,
}

fn install_release(root: &Path, revision: &str) -> PathBuf {
    let executable = root.join("orion-service/target/release/orion-service");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_orion-service"), &executable).unwrap();
    std::fs::write(
        root.join("release.json"),
        json!({"revision": revision}).to_string(),
    )
    .unwrap();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    assert!(
        Command::new("python3")
            .arg(repository.join("coordinator/tests/fixtures/install_speech.py"))
            .arg(root.join("speech/.venv"))
            .arg(revision)
            .status()
            .unwrap()
            .success()
    );
    executable
}

impl Process {
    async fn start() -> Self {
        Self::start_with_voice_url(None).await
    }
    async fn start_with_voice_url(voice_url: Option<&str>) -> Self {
        Self::start_at("installed/releases/test", "test-release", voice_url).await
    }
    async fn start_at(install_path: &str, revision: &str, voice_url: Option<&str>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let release = root.path().join(install_path);
        let executable = install_release(&release, revision);
        Self::launch(root, release, executable, voice_url).await
    }
    async fn launch(
        root: tempfile::TempDir,
        release: PathBuf,
        executable: PathBuf,
        voice_url: Option<&str>,
    ) -> Self {
        let directory = root.path().join(".local/share/orion/studio-service");
        let child = Self::spawn(root.path(), &executable, voice_url);
        let mut result = Self {
            child,
            root,
            directory,
            release,
        };
        result.wait_ready().await;
        result
    }
    fn spawn(root: &Path, executable: &Path, voice_url: Option<&str>) -> Child {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let token = root.join(".config/orion/studio-token");
        std::fs::create_dir_all(token.parent().unwrap()).unwrap();
        std::fs::write(token, "a".repeat(32)).unwrap();
        Command::new(executable)
            .args(["serve", "--no-autostart"])
            .current_dir(root)
            // Keep the developer's real ~/.config/orion out of the test.
            .env("HOME", root)
            // Old startup values must not override the installed local speech defaults.
            .env("HF_HOME", root.join("unused cache"))
            .env("HF_HUB_OFFLINE", "0")
            .env("ORION_STUDIO_TTS_MODEL", "unsupported-old-default")
            .env_remove("ORION_PI_VOICE_URL")
            .envs(voice_url.map(|url| ("ORION_PI_VOICE_URL", url)))
            // An inherited Python path must not supply speech from the checkout.
            .env("PYTHONPATH", repository.join("coordinator/tests/fixtures"))
            .env(
                "ORION_STUDIO_CODEX_BIN",
                repository.join("agent/tests/fixtures/codex.py"),
            )
            .env("ORION_MEMORY_PATH", root.join("MEMORY.md"))
            .env_remove("ORION_VOICE_HISTORY_DIR")
            .env("ORION_SOUL_PATH", root.join("SOUL.md"))
            .stdout(Stdio::null())
            .spawn()
            .unwrap()
    }
    async fn wait_ready(&mut self) {
        // Starting a copied executable and checking its venv can contend with parallel tests.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline {
            if self.directory.join("connection.json").exists() {
                return;
            }
            assert!(self.child.try_wait().unwrap().is_none());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("Service startup timed out");
    }
    async fn call(&self, request: Request) -> Value {
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || rpc::call(&directory, &request))
            .await
            .unwrap()
            .unwrap()
    }
    async fn terminate(&mut self) {
        Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap();
        for _ in 0..600 {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                assert!(!self.directory.join("connection.json").exists());
                assert!(rpc::Owner::acquire(&self.directory).is_ok());
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("Service shutdown timed out");
    }
}

#[tokio::test]
async fn installed_service_works_at_different_paths_and_from_unrelated_working_directory() {
    for (path, revision) in [
        (
            "home/alice/.local/share/orion/voice-stack/releases/one",
            "one",
        ),
        ("opt/custom-orion/releases/two", "two"),
        ("users/Bob Smith/custom install/three", "three"),
    ] {
        let mut process = Process::start_at(path, revision, None).await;
        let status = process.call(Request::Status).await;
        assert_eq!(
            status["project_root"],
            process.release.canonicalize().unwrap().to_str().unwrap()
        );
        assert_eq!(status["revision"], revision);
        let settings = process.call(Request::LoadSettings).await;
        assert_eq!(settings["ttsModel"], "piper-alba-medium");
        assert!(settings.get("cachePath").is_none());
        // Exercise the real gateway bridge against the service's default discovery folder.
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let bridge = Command::new("python3")
            .args(["-I", "-c", "import json, sys; sys.path.insert(0, sys.argv[1]); from gateway import voice_service_request; print(json.dumps(voice_service_request({'method': 'status'})))"])
            .arg(repository.join("orion_studio"))
            .env("HOME", process.root.path())
            .current_dir(process.root.path())
            .output().unwrap();
        assert!(
            bridge.status.success(),
            "{}",
            String::from_utf8_lossy(&bridge.stderr)
        );
        let gateway_status: Value = serde_json::from_slice(&bridge.stdout).unwrap();
        assert_eq!(gateway_status["pid"], process.child.id());
        assert_eq!(gateway_status["revision"], revision);
        let check = Command::new(
            process
                .release
                .join("orion-service/target/release/orion-service"),
        )
        .arg("check")
        .current_dir(process.root.path())
        .env("HOME", process.root.path())
        .output()
        .unwrap();
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stderr)
        );
        process.terminate().await;
    }
}

#[test]
fn incomplete_release_reports_missing_environment_or_package_without_source_fallback() {
    let base = tempfile::tempdir().unwrap();
    let release = base.path().join("release");
    let executable = install_release(&release, "incomplete");
    let python = release.join("speech/.venv/bin/python");
    let result = Command::new(&python)
        .args([
            "-I",
            "-c",
            "import sysconfig; print(sysconfig.get_path('purelib'))",
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    let package =
        PathBuf::from(String::from_utf8(result.stdout).unwrap().trim()).join("orion_speech_worker");
    std::fs::remove_dir_all(&package).unwrap();
    // A bundled source copy or inherited Python path must not hide an incomplete venv.
    let source = release.join("speech/orion_speech_worker");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("worker.py"), "").unwrap();
    let check = || {
        Command::new(&executable)
            .arg("check")
            .env("HOME", base.path())
            .env("PYTHONPATH", release.join("speech"))
            .current_dir(&release)
            .output()
            .unwrap()
    };
    let missing_package = check();
    assert!(!missing_package.status.success());
    assert!(
        String::from_utf8_lossy(&missing_package.stderr).contains("Invalid speech environment")
    );
    std::fs::remove_file(python).unwrap();
    let missing_environment = check();
    assert!(!missing_environment.status.success());
    assert!(
        String::from_utf8_lossy(&missing_environment.stderr)
            .contains("Missing speech Python environment")
    );
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn service_reads_grouped_conversations_and_filtered_behaviour_files() {
    let mut process = Process::start().await;
    let store = orion_history::Store::new(
        process
            .root
            .path()
            .join(".local/share/orion/voice-stack/history"),
    );
    let now = "2026-10-12T11:00:00Z".parse().unwrap();
    store.accept_turn("first", "Hello", now).unwrap();
    store.accept_turn("second", "Follow-up", now).unwrap();
    let groups = process
        .call(Request::History {
            conversation_id: None,
            before: None,
        })
        .await;
    assert_eq!(groups["items"].as_array().unwrap().len(), 1);
    assert_eq!(groups["items"][0]["turnCount"], 2);
    let detail = process
        .call(Request::History {
            conversation_id: Some("first".into()),
            before: None,
        })
        .await;
    assert_eq!(detail["turns"].as_array().unwrap().len(), 2);
    let writer = orion_history::BehaviourWriter::start(store.root().into()).unwrap();
    writer
        .sender()
        .try_send(orion_history::BehaviourEvent {
            id: "rest-event".into(),
            at: now.to_rfc3339(),
            runtime_time_seconds: 1.,
            category: "rest".into(),
            behaviour: "sleep".into(),
            phase: "started".into(),
            target: None,
            target_category: None,
            reason: Some("requested".into()),
            action_id: Some("rest-event".into()),
            duration_seconds: None,
            session_id: None,
        })
        .unwrap();
    drop(writer);
    let events = process
        .call(Request::Behaviours {
            before: None,
            category: Some("rest".into()),
        })
        .await;
    assert_eq!(events["items"][0]["behaviour"], "sleep");
    let filtered = process
        .call(Request::Behaviours {
            before: None,
            category: Some("attention".into()),
        })
        .await;
    assert_eq!(filtered["items"], json!([]));
    process.terminate().await;
}

#[tokio::test]
async fn service_authentication_single_owner_and_clean_shutdown() {
    let mut process = Process::start().await;
    let status = process.call(Request::Status).await;
    assert_eq!(
        process.call(Request::Status).await["pid"],
        process.child.id()
    );
    assert_eq!(status["pid"], process.child.id());
    assert_eq!(status["onboard"], true);
    assert_eq!(status["coordinator_running"], false);
    assert!(rpc::Owner::acquire(&process.directory).is_err());
    let mut second = Command::new(
        process
            .release
            .join("orion-service/target/release/orion-service"),
    )
    .args(["serve", "--no-autostart"])
    .env("HOME", process.root.path())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    assert!(!second.wait().unwrap().success());
    let connection: rpc::Connection =
        serde_json::from_slice(&std::fs::read(process.directory.join("connection.json")).unwrap())
            .unwrap();
    let mut stream = TcpStream::connect(connection.address).await.unwrap();
    stream
        .write_all(b"{\"protocol\":1,\"token\":\"wrong\",\"request\":{\"method\":\"status\"}}\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&response).unwrap()["ok"],
        false
    );
    assert!(!response.contains(&connection.token));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(process.directory.join("connection.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&process.directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    use orion_agent::lists::{ListChange, ListName};
    let items = process
        .call(Request::Lists {
            list: ListName::Shopping,
            change: Some(ListChange::Add {
                text: "Milk".into(),
            }),
        })
        .await;
    assert_eq!(items[0]["text"], "Milk");
    assert_eq!(
        process
            .call(Request::Lists {
                list: ListName::Shopping,
                change: None
            })
            .await,
        items
    );
    let expected = serde_json::from_value(items).unwrap();
    assert_eq!(
        process
            .call(Request::Lists {
                list: ListName::Shopping,
                change: Some(ListChange::Clear {
                    expected,
                    confirmed: true
                })
            })
            .await,
        json!([])
    );
    process.terminate().await;
    assert!(
        rpc::call(&process.directory, &Request::Status)
            .unwrap_err()
            .contains("stopped")
    );
    assert!(rpc::Owner::acquire(&process.directory).is_ok());
}

async fn send<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    value: Value,
) {
    socket.send(Message::text(value.to_string())).await.unwrap();
}
async fn next<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(8), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(raw) = message {
            return serde_json::from_str(&raw).unwrap();
        }
    }
}
async fn until<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    kind: &str,
) -> Value {
    loop {
        let value = next(socket).await;
        if value["type"] == kind {
            return value;
        }
        assert_ne!(value["type"], "worker.error", "{value}");
    }
}

#[tokio::test]
async fn saved_start_connects_to_listener_with_pi_speech_defaults() {
    let mut listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut pi_url = format!("ws://{}", listener.local_addr().unwrap());
    let mut process = Process::start_with_voice_url(Some(&pi_url)).await;
    let settings = orion_service::settings::VoiceSettings {
        model: "test-model".into(),
        effort: "high".into(),
        ..Default::default()
    };
    std::fs::write(
        process
            .root
            .path()
            .join(".config/orion/voice-settings.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    for revision in ["test-release", "updated-release"] {
        if revision == "updated-release" {
            let release = process.root.path().join("installed/releases/update");
            let executable = install_release(&release, revision);
            // Preparing a new release does not change the running service's identity.
            assert_eq!(
                process.call(Request::Status).await["revision"],
                "test-release"
            );
            process.terminate().await;
            // Discard unaccepted control connections left by the stopped fake peer.
            listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            pi_url = format!("ws://{}", listener.local_addr().unwrap());
            process.child = Process::spawn(process.root.path(), &executable, Some(&pi_url));
            process.release = release;
            process.wait_ready().await;
        }
        let connection = process.call(Request::StartSaved).await;
        // The coordinator also polls microphone status on a separate control connection.
        let mut pi = loop {
            let (stream, _) = tokio::time::timeout(Duration::from_secs(8), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let hello = next(&mut socket).await;
            assert_eq!(hello["token"], "a".repeat(32));
            if hello["role"] == "control" {
                send(
                    &mut socket,
                    json!({"type":"microphone.status","muted":false}),
                )
                .await;
                socket.close(None).await.unwrap();
            } else {
                break socket;
            }
        };
        send(&mut pi, json!({"type":"ready","protocol":2,"sampleRate":16000,"channels":1,"encoding":"pcm_s16le","muted":false,
            "conversationWindow":true,"toolFeedback":true,"wake":{"provider":"rustpotter","model":"fixture","threshold":0.4}})).await;
        let (mut observer, _) = connect_async(connection["url"].as_str().unwrap())
            .await
            .unwrap();
        send(
            &mut observer,
            json!({"type":"hello","protocol":7,"token":connection["token"]}),
        )
        .await;
        let ready = until(&mut observer, "ready").await;
        assert_eq!(ready["tts"]["provider"], "piper-tts");
        assert_eq!(ready["tts"]["model"], "piper-alba-medium");
        assert_eq!(ready["asr"]["model"], revision);
        assert_eq!(process.call(Request::Status).await["revision"], revision);
        assert_eq!(process.call(Request::Status).await["onboard"], true);
    }
    process.terminate().await;
}

#[tokio::test]
async fn headless_process_runs_voice_without_ui_and_reuses_owner_for_observers() {
    let mut process = Process::start().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pi_url = format!("ws://{}", listener.local_addr().unwrap());
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gateway_url = format!("http://{}", http.local_addr().unwrap());
    let uploads = Arc::new(AtomicUsize::new(0));
    let cancellations = Arc::new(AtomicUsize::new(0));
    let complete = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let mut tasks = JoinSet::new();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
    tasks.spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let sender = sender.clone();
            tokio::spawn(async move {
                let mut socket = accept_async(stream).await.unwrap();
                let hello = next(&mut socket).await;
                assert_eq!(hello["token"], "a".repeat(32));
                if hello["role"] == "control" {
                    send(
                        &mut socket,
                        json!({"type":"microphone.status","muted":false}),
                    )
                    .await;
                    if let Some(Ok(Message::Text(raw))) = socket.next().await {
                        let value: Value = serde_json::from_str(&raw).unwrap();
                        send(
                            &mut socket,
                            json!({"type":"microphone.status","muted":value["muted"]}),
                        )
                        .await;
                    }
                } else {
                    sender.send(socket).await.unwrap();
                }
            });
        }
    });
    let count = uploads.clone();
    let cancelled = cancellations.clone();
    let completed = complete.clone();
    tasks.spawn(async move {
        loop {
            let (stream, _) = http.accept().await.unwrap();
            tokio::spawn(gateway(
                stream,
                count.clone(),
                cancelled.clone(),
                completed.clone(),
            ));
        }
    });
    let options = StartOptions {
        pi_url,
        pi_token: "a".repeat(32),
        gateway_url,
        agent_model: Some("test-model".into()),
        agent_effort: Some("high".into()),
        asr_model: Some("fixture".into()),
        tts_model: Some("piper-alba-medium".into()),
        asr_path: Some(String::new()),
        tts_path: Some(String::new()),
    };
    let connection = process.call(Request::Start(options.clone())).await;
    let mut pi = tokio::time::timeout(Duration::from_secs(8), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    send(&mut pi, json!({"type":"ready","protocol":2,"sampleRate":16000,"channels":1,"encoding":"pcm_s16le","muted":false,
        "conversationWindow":true,"toolFeedback":true,"wake":{"provider":"rustpotter","model":"fixture","threshold":0.4}})).await;
    assert_eq!(connection, process.call(Request::Start(options)).await);
    assert!(receiver.try_recv().is_err());
    let (mut observer, _) = connect_async(connection["url"].as_str().unwrap())
        .await
        .unwrap();
    send(
        &mut observer,
        json!({"type":"hello","protocol":7,"token":connection["token"]}),
    )
    .await;
    until(&mut observer, "ready").await;
    observer.close(None).await.unwrap();
    let id = "b".repeat(32);
    wake(&mut pi, &id, "Hey Orion, hello").await;
    until(&mut pi, "session.finish").await;
    assert!(uploads.load(Ordering::SeqCst) > 0);
    assert_eq!(
        process.call(Request::Microphone { muted: true }).await["muted"],
        true
    );
    assert_eq!(
        process.call(Request::Status).await["coordinator_running"],
        true
    );
    complete.store(false, Ordering::SeqCst);
    wake(&mut pi, &"c".repeat(32), "Hey Orion, another reply").await;
    until(&mut pi, "session.playing").await;
    process.terminate().await;
    assert_eq!(cancellations.load(Ordering::SeqCst), 1);
    assert!(process.root.path().exists());
    tasks.abort_all();
}
async fn wake(pi: &mut WebSocketStream<TcpStream>, id: &str, text: &str) {
    send(
        pi,
        json!({"type":"wake.candidate","sessionId":id,"name":"hey_orion","score":0.8}),
    )
    .await;
    let mut pcm = text.as_bytes().to_vec();
    if !pcm.len().is_multiple_of(2) {
        pcm.push(0);
    }
    send(
        pi,
        json!({"type":"utterance","sessionId":id,"purpose":"wake_and_command","bytes":pcm.len()}),
    )
    .await;
    pi.send(Message::Binary(pcm.into())).await.unwrap();
}
async fn gateway(
    mut stream: TcpStream,
    uploads: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
    complete: Arc<std::sync::atomic::AtomicBool>,
) {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        if stream.read_exact(&mut byte).await.is_err() {
            return;
        }
        header.push(byte[0]);
    }
    let header = String::from_utf8(header).unwrap();
    assert!(
        header
            .to_lowercase()
            .contains(&format!("authorization: bearer {}", "a".repeat(32)))
    );
    let path = header
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let length = header
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .map(|(_, v)| v.trim().parse().unwrap())
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await.unwrap();
    let value = if path == "/api/v2/operations" {
        cancelled.fetch_add(1, Ordering::SeqCst);
        json!({"ok":true})
    } else if path.ends_with("/stream") || path.contains("/chunks/") {
        assert!(body.starts_with(b"RIFF"));
        uploads.fetch_add(1, Ordering::SeqCst);
        json!({"run_id":1})
    } else if path.ends_with("/end") {
        json!({"ok":true})
    } else {
        json!({"state":if complete.load(Ordering::SeqCst) {"completed"} else {"playing"},"first_playback_ms":25})
    };
    let body = value.to_string();
    let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await;
}
