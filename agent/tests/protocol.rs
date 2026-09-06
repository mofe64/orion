use orion_agent::{AgentConfig, AgentService};
use std::{path::PathBuf, time::Duration};
fn service() -> AgentService {
    AgentService::start(AgentConfig {
        model: "test-model".into(),
        effort: "high".into(),
        codex_bin: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex.py")),
    })
    .unwrap()
}
#[tokio::test]
async fn caller_reconnections_keep_conversation_and_only_speak_matching_final_turn() {
    let service = service();
    let first = service.handle();
    let id = first.info().await.unwrap().conversation_id;
    assert_eq!(
        first.respond("First command").await.unwrap(),
        "Reply 1: First command"
    );
    drop(first);
    let second = service.handle();
    assert_eq!(second.info().await.unwrap().conversation_id, id);
    assert_eq!(
        second.respond("Follow-up").await.unwrap(),
        "Reply 2: Follow-up"
    );
}
#[tokio::test]
async fn failures_reset_uncertain_session_and_next_request_recovers() {
    let service = service();
    let agent = service.handle();
    for input in ["fail", "approval", "empty", "crash"] {
        let before = agent.info().await.unwrap().conversation_id;
        assert!(agent.respond(input).await.is_err());
        assert_eq!(
            agent.respond("Recovered").await.unwrap(),
            "Reply 1: Recovered"
        );
        assert_ne!(agent.info().await.unwrap().conversation_id, before);
    }
}
#[tokio::test]
async fn dropping_active_call_cancels_without_affecting_later_reply() {
    let service = service();
    let agent = service.handle();
    let before = agent.info().await.unwrap().conversation_id;
    assert!(
        tokio::time::timeout(Duration::from_millis(150), agent.respond("hang"))
            .await
            .is_err()
    );
    assert_eq!(
        agent.respond("After cancellation").await.unwrap(),
        "Reply 1: After cancellation"
    );
    assert_ne!(agent.info().await.unwrap().conversation_id, before);
}
#[tokio::test]
async fn invalid_input_does_not_reset_conversation() {
    let service = service();
    let agent = service.handle();
    let before = agent.info().await.unwrap().conversation_id;
    assert!(agent.respond(" ").await.is_err());
    assert_eq!(agent.info().await.unwrap().conversation_id, before);
}
#[tokio::test]
async fn incompatible_model_is_not_substituted() {
    let service = AgentService::start(AgentConfig {
        model: "missing-model".into(),
        effort: "high".into(),
        codex_bin: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex.py")),
    })
    .unwrap();
    assert!(
        service
            .handle()
            .info()
            .await
            .unwrap_err()
            .contains("does not advertise missing-model with high effort")
    );
}
