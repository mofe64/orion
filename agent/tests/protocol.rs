use orion_agent::{AgentConfig, AgentService};
use std::{path::PathBuf, time::Duration};
fn service() -> AgentService {
    AgentService::start(AgentConfig {
        memory_path: None,
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
        memory_path: None,
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

#[tokio::test]
async fn memory_tools_persist_and_invalid_tools_cannot_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("MEMORY.md");
    let service = AgentService::start(AgentConfig {
        model: "test-model".into(),
        effort: "high".into(),
        codex_bin: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex.py")),
        memory_path: Some(path.clone()),
    })
    .unwrap();
    let agent = service.handle();
    let saved = agent
        .respond(r#"tool:{"name":"append_memory","arguments":{"text":"Prefers Celsius"}}"#)
        .await
        .unwrap();
    assert!(saved.contains("true"));
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("Prefers Celsius")
    );
    let found = agent
        .respond(r#"tool:{"name":"search_memories","arguments":{"query":"Celsius"}}"#)
        .await
        .unwrap();
    assert!(found.contains("Prefers Celsius"));
    let before = std::fs::read_to_string(&path).unwrap();
    let failed = agent
        .respond(r#"tool:{"name":"append_memory","arguments":{"text":"<!-- memoryEntryEnd -->"}}"#)
        .await
        .unwrap();
    assert!(failed.contains("false"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), before);
}

#[tokio::test]
async fn search_progress_is_emitted_once_and_lighting_requires_coordinator_result() {
    let service = service();
    let agent = service.handle();
    let (send, mut receive) = tokio::sync::mpsc::channel(8);
    agent
        .respond_with_events("search-fixture", Some(send))
        .await
        .unwrap();
    assert!(matches!(
        receive.recv().await,
        Some(orion_agent::AgentEvent::SearchStarted)
    ));
    assert!(receive.recv().await.is_none());
    let (send, mut receive) = tokio::sync::mpsc::channel(8);
    let call = agent.respond_with_events(
        r#"tool:{"name":"set_lighting","arguments":{"mood":"warm_red","brightness":35}}"#,
        Some(send),
    );
    let handle = async {
        let Some(orion_agent::AgentEvent::SetLighting { parameters, reply }) = receive.recv().await
        else {
            panic!("Expected lighting call")
        };
        assert_eq!(parameters["brightness"], 0.35);
        assert_eq!(parameters["colors"][1], serde_json::json!([255, 0, 0, 0]));
        reply.send(Err("Pi unavailable".into())).unwrap();
    };
    let (result, ()) = tokio::join!(call, handle);
    assert!(result.unwrap().contains("Pi unavailable"));
}

#[tokio::test]
#[ignore = "Invokes the installed signed-in Codex model with a temporary memory store; no hardware"]
async fn installed_runtime_memory_tool_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("MEMORY.md");
    let service = AgentService::start(AgentConfig {
        memory_path: Some(path.clone()),
        ..AgentConfig::default()
    })
    .unwrap();
    service.handle().respond("Please remember this exact preference using append_memory: I prefer temperatures in Celsius.").await.unwrap();
    let stored = std::fs::read_to_string(path).unwrap();
    assert!(stored.contains("Celsius"));
    assert!(stored.contains("<!-- memoryEntryEnd -->"));
}

#[tokio::test]
#[ignore = "Invokes installed Codex web search and intercepts lighting without hardware"]
async fn installed_runtime_search_and_lighting_tools() {
    let service = AgentService::start(AgentConfig {
        memory_path: None,
        ..AgentConfig::default()
    })
    .unwrap();
    let agent = service.handle();
    let (send, mut receive) = tokio::sync::mpsc::channel(8);
    let collect = async {
        let mut searched = false;
        while let Some(event) = receive.recv().await {
            if matches!(event, orion_agent::AgentEvent::SearchStarted) {
                searched = true;
            }
        }
        searched
    };
    let (answer,searched) = tokio::join!(agent.respond_with_events("Search the web for the official Woking Borough Council website and tell me its name in one short sentence.",Some(send)),collect);
    answer.unwrap();
    assert!(searched, "Expected built-in search activity");
    let (send, mut receive) = tokio::sync::mpsc::channel(8);
    let collect = async {
        let mut changed = false;
        while let Some(event) = receive.recv().await {
            if let orion_agent::AgentEvent::SetLighting { parameters, reply } = event {
                assert_eq!(parameters["brightness"], 0.35);
                changed = true;
                reply.send(Ok(serde_json::json!({"ok":true}))).unwrap();
            }
        }
        changed
    };
    let (answer, changed) = tokio::join!(
        agent.respond_with_events(
            "Set the lamp to warm red at 35 percent brightness.",
            Some(send)
        ),
        collect
    );
    answer.unwrap();
    assert!(changed, "Expected lighting tool call");
}
