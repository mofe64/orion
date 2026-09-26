use super::*;
use crate::SpeechConfig;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

async fn check_control_close(request: Option<bool>, acknowledge_close: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = CoordinatorConfig {
        pi_url: format!("ws://{}", listener.local_addr().unwrap()),
        pi_token: "a".repeat(32),
        gateway_url: String::new(),
        speech: SpeechConfig {
            python: "python3".into(),
            root: ".".into(),
            asr_model: String::new(),
            tts_model: String::new(),
            cache_path: String::new(),
        },
    };
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let hello = decode(socket.next().await.unwrap().unwrap()).unwrap();
        assert_eq!(hello["role"], "control");
        assert_eq!(hello["token"], "a".repeat(32));
        socket
            .send(Message::text(
                json!({"type":"microphone.status", "muted":false}).to_string(),
            ))
            .await
            .unwrap();
        if let Some(muted) = request {
            let command = decode(socket.next().await.unwrap().unwrap()).unwrap();
            assert_eq!(command, json!({"type":"microphone.mute", "muted":muted}));
            socket
                .send(Message::text(
                    json!({"type":"microphone.status", "muted":muted}).to_string(),
                ))
                .await
                .unwrap();
        }
        let close = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap();
        assert!(
            matches!(close, Some(Ok(Message::Close(_)))),
            "Expected graceful close, got {close:?}"
        );
        if acknowledge_close {
            // Reading Close queues the peer's reply; flush completes it.
            socket.flush().await.unwrap();
        } else {
            // Keep the transport open without replying to the close handshake.
            std::future::pending::<()>().await;
        }
    });
    let status = tokio::time::timeout(Duration::from_secs(3), microphone(&config, request))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        status,
        json!({"type":"microphone.status", "muted":request.unwrap_or(false)})
    );
    if acknowledge_close {
        peer.await.unwrap();
    } else {
        assert!(!peer.is_finished());
        peer.abort();
        assert!(peer.await.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn status_poll_closes_control_socket_gracefully() {
    check_control_close(None, true).await;
}

#[tokio::test]
async fn mute_closes_control_socket_gracefully() {
    check_control_close(Some(true), true).await;
}

#[tokio::test]
async fn acknowledged_mute_survives_unresponsive_close_peer() {
    check_control_close(Some(true), false).await;
}
