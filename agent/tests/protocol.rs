use orion_agent::{AgentConfig, AgentService};
use std::{path::PathBuf, time::Duration};
fn service() -> AgentService {
    AgentService::start(AgentConfig {
        soul_path: None,
        memory_path: None,
        model: "test-model".into(),
        effort: "high".into(),
        codex_bin: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex.py")),
    })
    .unwrap()
}
#[tokio::test]
async fn streams_only_explicit_final_sentences_from_the_matching_turn() {
    let service = service();
    let agent = service.handle();
    let (send, mut receive) = tokio::sync::mpsc::channel(8);
    let response = agent.respond_with_events("stream-fixture", Some(send));
    let collect = async {
        let mut pieces = Vec::new();
        while let Some(event) = receive.recv().await {
            if let orion_agent::AgentEvent::FinalSpeech(text) = event {
                pieces.push(text);
            }
        }
        pieces
    };
    let (response, pieces) = tokio::join!(response, collect);
    assert_eq!(response.unwrap(), "Let us begin. Take a breath.");
    assert_eq!(pieces, ["Let us begin."]);
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
    for input in ["start-rejected", "fail", "empty"] {
        let before = agent.info().await.unwrap().conversation_id;
        assert!(agent.respond(input).await.is_err());
        assert_eq!(agent.info().await.unwrap().conversation_id, before);
        assert!(
            agent
                .respond("Still here")
                .await
                .unwrap()
                .contains("Still here")
        );
    }
    for input in [
        "approval",
        "crash",
        "unreadable",
        "stale-tool",
        "invalid-tool",
        "multiple-finals",
        "multiple-completed-finals",
        "multiple-terminal-finals",
        "changed-speech",
        "empty-after-stream",
        "tool-after-speech",
    ] {
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
async fn direct_sleep_without_a_robot_coordinator_fails_closed() {
    let service = service();
    let agent = service.handle();
    let before = agent.info().await.unwrap().conversation_id;
    assert_eq!(
        agent.respond("Go to sleep.").await.unwrap_err(),
        "No robot coordinator is attached"
    );
    assert_eq!(agent.info().await.unwrap().conversation_id, before);
}
#[tokio::test]
async fn incompatible_model_is_not_substituted() {
    let service = AgentService::start(AgentConfig {
        soul_path: None,
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
        soul_path: None,
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
    let Some(orion_agent::AgentEvent::ToolCall {
        name,
        arguments,
        result,
        success,
        duration_ms,
    }) = receive.recv().await
    else {
        panic!("Expected the completed tool call for voice history")
    };
    assert_eq!(name, "set_lighting");
    assert_eq!(arguments["brightness"], 35);
    assert_eq!(result["error"], "Pi unavailable");
    assert!(!success);
    assert!(duration_ms >= 0.);
}

#[tokio::test]
#[ignore = "Invokes the installed signed-in Codex model with a temporary memory store; no hardware"]
async fn installed_runtime_memory_tool_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("MEMORY.md");
    let service = AgentService::start(AgentConfig {
        soul_path: None,
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
        soul_path: None,
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

#[tokio::test]
async fn profile_edits_reset_context_but_reads_and_conflicts_preserve_it() {
    use orion_agent::profile::{Personality, ProfileChange};
    let dir = tempfile::tempdir().unwrap();
    let memory = dir.path().join("MEMORY.md");
    let soul = dir.path().join("SOUL.md");
    let service = AgentService::start(AgentConfig {
        model: "test-model".into(),
        effort: "high".into(),
        codex_bin: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex.py")),
        memory_path: Some(memory.clone()),
        soul_path: Some(soul.clone()),
    })
    .unwrap();
    let agent = service.handle();
    // Profile editing is available before any Codex process is started.
    let profile = agent.profile(None).await.unwrap();
    assert!(profile.memories.is_empty());
    let profile = agent
        .profile(Some(ProfileChange::AddMemory {
            text: "Prefers Celsius".into(),
        }))
        .await
        .unwrap();
    let old = agent.info().await.unwrap().conversation_id;
    agent.profile(None).await.unwrap();
    assert_eq!(agent.info().await.unwrap().conversation_id, old);
    let entry = profile.memories[0].clone();
    let updated = agent
        .profile(Some(ProfileChange::EditMemory {
            expected: entry.clone(),
            text: "Prefers Fahrenheit".into(),
        }))
        .await
        .unwrap();
    assert_eq!(updated.memories[0].id, entry.id);
    let next = agent.info().await.unwrap().conversation_id;
    assert_ne!(next, old);
    assert!(
        agent
            .profile(Some(ProfileChange::DeleteMemory { expected: entry }))
            .await
            .is_err()
    );
    assert_eq!(agent.info().await.unwrap().conversation_id, next);
    let changed = agent
        .profile(Some(ProfileChange::Personality {
            expected_revision: profile.soul.revision,
            personality: Personality {
                traits: vec!["direct".into()],
                behaviors: vec![],
            },
        }))
        .await
        .unwrap();
    assert!(
        std::fs::read_to_string(&soul)
            .unwrap()
            .contains("Lead with the answer")
    );
    assert_ne!(agent.info().await.unwrap().conversation_id, next);
    assert!(
        agent
            .profile(Some(ProfileChange::Personality {
                expected_revision: "default".into(),
                personality: Personality::default()
            }))
            .await
            .is_err()
    );
    agent
        .profile(Some(ProfileChange::ClearMemories {
            expected: changed.memories,
        }))
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(memory).unwrap(), "");
}

#[tokio::test]
async fn exactly_800_character_streamed_prefix_keeps_its_final_tail_and_conversation() {
    let service = service();
    let agent = service.handle();
    let before = agent.info().await.unwrap().conversation_id;
    let (send, mut receive) = tokio::sync::mpsc::channel(8);
    let response = agent
        .respond_with_events("800-prefix-fixture", Some(send))
        .await
        .unwrap();
    let Some(orion_agent::AgentEvent::FinalSpeech(prefix)) = receive.recv().await else {
        panic!("Missing speech")
    };
    assert_eq!(prefix.chars().count(), 800);
    assert_eq!(response, format!("{prefix} This ending survives."));
    assert_eq!(agent.info().await.unwrap().conversation_id, before);
}

#[tokio::test]
async fn rejected_duplicate_and_excessive_tools_complete_the_turn_without_reset_or_repeat_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("MEMORY.md");
    let service = AgentService::start(AgentConfig {
        soul_path: None,
        memory_path: Some(path.clone()),
        model: "test-model".into(),
        effort: "high".into(),
        codex_bin: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex.py")),
    })
    .unwrap();
    let agent = service.handle();
    let before = agent.info().await.unwrap().conversation_id;
    let answer = agent.respond("tool-budget-fixture").await.unwrap();
    assert_eq!(
        answer.matches("Duplicate or excessive tool calls").count(),
        2
    );
    assert_eq!(answer.matches("\"success\": true").count(), 16, "{answer}");
    assert_eq!(
        std::fs::read_to_string(path)
            .unwrap()
            .matches("<!-- memoryEntry ")
            .count(),
        16
    );
    assert_eq!(agent.info().await.unwrap().conversation_id, before);
    assert_eq!(
        agent.respond("Continue").await.unwrap(),
        "Reply 2: Continue"
    );
}

#[tokio::test]
async fn every_turn_includes_local_time_offset_and_iana_zone() {
    let service = service();
    let text = service.handle().respond("clock-fixture").await.unwrap();
    let local = text
        .split("Current local date/time: ")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    chrono::DateTime::parse_from_rfc3339(local).unwrap();
    let zone = text.split("IANA timezone: ").nth(1).unwrap();
    assert!(zone == "UTC" || zone.contains('/'));
}

#[test]
fn prompt_leaves_brevity_to_personality_and_specifies_listening_format() {
    let instructions = orion_agent::ORION_INSTRUCTIONS;
    assert!(instructions.contains("one short sentence"));
    assert!(instructions.contains("no length limit"));
    assert!(instructions.contains("no markdown, bullet symbols, tables or headings"));
    assert!(instructions.contains("Speak English by default"));
    assert!(instructions.contains("Switch languages only when the user explicitly asks"));
    assert!(!instructions.contains("18 words") && !instructions.contains("35 words"));
    let off = orion_agent::profile::Personality {
        traits: vec![],
        behaviors: vec![],
    }
    .instructions()
    .unwrap();
    assert!(!off.contains("Prefer short answers"));
    assert!(
        orion_agent::profile::Personality::default()
            .instructions()
            .unwrap()
            .contains("Prefer short answers")
    );
}

#[tokio::test]
async fn phase_less_messages_are_fallbacks_and_do_not_conflict_with_explicit_finals() {
    let service = service();
    let agent = service.handle();
    let id = agent.info().await.unwrap().conversation_id;
    assert_eq!(agent.respond("legacy-only").await.unwrap(), "Legacy 1.");
    assert_eq!(
        agent.respond("repeated-terminal-final").await.unwrap(),
        "One answer."
    );
    let (send, mut receive) = tokio::sync::mpsc::channel(8);
    let response = agent.respond_with_events("legacy-before-final", Some(send));
    let collect = async {
        let mut pieces = Vec::new();
        while let Some(event) = receive.recv().await {
            if let orion_agent::AgentEvent::FinalSpeech(text) = event {
                pieces.push(text);
            }
        }
        pieces
    };
    let (answer, pieces) = tokio::join!(response, collect);
    assert_eq!(answer.unwrap(), "Only the final.");
    assert_eq!(pieces, ["Only the final."]);
    assert_eq!(agent.info().await.unwrap().conversation_id, id);
}

#[tokio::test]
async fn delivery_limit_failure_is_visible_to_the_next_model_turn_once() {
    let service = service();
    let agent = service.handle();
    agent.report_delivery_failure("Speech stream exceeds the 30-minute audio sanity limit; playback cannot complete this answer.");
    let answer = agent.respond("What happened?").await.unwrap();
    assert!(answer.contains("Previous speech delivery failed:"));
    assert!(answer.contains("30-minute audio sanity limit"));
    assert_eq!(
        agent.respond("Try a shorter answer").await.unwrap(),
        "Reply 2: Try a shorter answer"
    );
}

#[tokio::test]
async fn interruption_notice_is_delivered_once_and_preserves_direct_sleep_detection() {
    let service = service();
    let agent = service.handle();
    let conversation = agent.info().await.unwrap().conversation_id;
    agent.report_interruption();
    let answer = agent.respond("What were you saying?").await.unwrap();
    assert!(answer.contains("The previous reply was interrupted before it finished."));
    assert!(answer.contains("User request: What were you saying?"));
    assert_eq!(
        agent.respond("Continue").await.unwrap(),
        "Reply 2: Continue"
    );
    assert_eq!(agent.info().await.unwrap().conversation_id, conversation);
    agent.report_interruption();
    // A notice must not hide the explicit sleep request from the local classifier.
    assert_eq!(
        agent.respond("go to sleep").await.unwrap_err(),
        "No robot coordinator is attached"
    );
}
