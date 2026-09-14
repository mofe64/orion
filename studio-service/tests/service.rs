use futures_util::{SinkExt, StreamExt};
use orion_studio_service::{Request, StartOptions, rpc};
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
}
impl Process {
    async fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("service");
        let fixture = root.path().join("speech/orion_speech_worker");
        std::fs::create_dir_all(&fixture).unwrap();
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        std::fs::copy(
            repository.join("coordinator/tests/fixtures/orion_speech_worker/worker.py"),
            fixture.join("worker.py"),
        )
        .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_orion-studio-headless"))
            .args(["serve", "--no-autostart"])
            .env("ORION_STUDIO_SERVICE_HOME", &directory)
            .env("ORION_PROJECT_ROOT", root.path())
            .env("ORION_STUDIO_VOICE_PYTHON", "python3")
            .env(
                "ORION_STUDIO_CODEX_BIN",
                repository.join("agent/tests/fixtures/codex.py"),
            )
            .env("ORION_MEMORY_PATH", root.path().join("MEMORY.md"))
            .env("ORION_SOUL_PATH", root.path().join("SOUL.md"))
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let mut result = Self {
            child,
            root,
            directory,
        };
        for _ in 0..100 {
            if result.directory.join("connection.json").exists() {
                return result;
            }
            assert!(result.child.try_wait().unwrap().is_none());
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
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn service_authentication_single_owner_and_clean_shutdown() {
    let mut process = Process::start().await;
    let client = orion_studio_service::Backend::at(process.directory.clone());
    let status = client.request(Request::Status).await.unwrap();
    client.shutdown();
    drop(client);
    assert_eq!(
        process.call(Request::Status).await["pid"],
        process.child.id()
    );
    assert_eq!(status["pid"], process.child.id());
    assert_eq!(status["coordinator_running"], false);
    assert!(rpc::Owner::acquire(&process.directory).is_err());
    let mut second = Command::new(env!("CARGO_BIN_EXE_orion-studio-headless"))
        .args(["serve", "--no-autostart"])
        .env("ORION_STUDIO_SERVICE_HOME", &process.directory)
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
    // An installed but stopped owner must not silently start an embedded replacement.
    std::fs::write(process.directory.join("installed"), "").unwrap();
    process.terminate().await;
    let client = orion_studio_service::Backend::at(process.directory.clone());
    assert!(
        client
            .request(Request::Status)
            .await
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
        tts_model: Some("fixture".into()),
        asr_path: Some(String::new()),
        tts_path: Some(String::new()),
        cache_path: Some(String::new()),
    };
    let connection = process.call(Request::Start(options.clone())).await;
    let mut pi = tokio::time::timeout(Duration::from_secs(8), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    send(&mut pi, json!({"type":"ready","protocol":1,"sampleRate":16000,"channels":1,"encoding":"pcm_s16le","muted":false,
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
