use chrono::{Duration, TimeZone, Utc};
use orion_history::Store;
use serde_json::json;

fn start() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 11, 0, 0).unwrap()
}

#[test]
fn only_accepted_nonempty_commands_create_history() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().into());
    for kind in [
        "wake.candidate",
        "wake.rejected",
        "wake.confirmed",
        "worker.error",
        "session.expired",
    ] {
        store
            .record_at(
                &json!({"type":kind,"sessionId":"false-wake","message":"No command"}),
                start(),
            )
            .unwrap();
    }
    store
        .record_at(
            &json!({"type":"transcript.final","sessionId":"empty","text":" "}),
            start(),
        )
        .unwrap();
    assert_eq!(store.read(None, None).unwrap()["items"], json!([]));
    store
        .record_at(
            &json!({"type":"transcript.final","sessionId":"real","text":"Hello"}),
            start(),
        )
        .unwrap();
    store
        .record_at(
            &json!({"type":"worker.error","sessionId":"real","message":"Agent unavailable"}),
            start(),
        )
        .unwrap();
    let detail = store.read(Some("real"), None).unwrap();
    assert_eq!(detail["turns"][0]["error"], "Agent unavailable");
    assert_eq!(
        store.read(None, None).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn interrupted_first_write_cannot_anchor_the_next_conversation() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().into());
    store
        .accept_turn("orphan", "Interrupted first write", start())
        .unwrap();
    // Emulate a crash after metadata persisted but before the turn was saved.
    std::fs::remove_file(
        store
            .conversation_path("orphan")
            .unwrap()
            .join("turns/orphan.json"),
    )
    .unwrap();
    assert_eq!(store.read(None, None).unwrap()["items"], json!([]));
    let accepted_at = start() + Duration::hours(1);
    let real = store
        .accept_turn("real", "First saved command", accepted_at)
        .unwrap();
    assert_eq!(real.conversation_id, "real");
    let detail = store.read(Some("real"), None).unwrap();
    assert_eq!(
        detail["conversation"]["startedAt"],
        accepted_at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
    );
    assert_eq!(
        detail["conversation"]["expiresAt"],
        (accepted_at + Duration::hours(24)).to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
    );
    assert_eq!(
        store.read(None, None).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn fixed_window_restart_boundary_and_late_completion() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().into());
    let first = store.accept_turn("first", "First", start()).unwrap();
    let reopened = Store::new(directory.path().into());
    let last = reopened
        .accept_turn(
            "last",
            "Last",
            start() + Duration::hours(24) - Duration::seconds(1),
        )
        .unwrap();
    assert_eq!(first.conversation_id, last.conversation_id);
    let next = reopened
        .accept_turn("next", "Next", start() + Duration::hours(24))
        .unwrap();
    assert_ne!(first.conversation_id, next.conversation_id);
    reopened
        .record_at(
            &json!({"type":"speech.completed","sessionId":"last"}),
            start() + Duration::hours(25),
        )
        .unwrap();
    assert_eq!(
        reopened.read(Some("first"), None).unwrap()["turns"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        reopened.read(Some("first"), None).unwrap()["turns"][1]["delivery"],
        "completed"
    );
    assert_eq!(
        reopened
            .accept_turn("last", "Last", start() + Duration::hours(30))
            .unwrap()
            .conversation_id,
        "first"
    );
    assert!(reopened.accept_turn("last", "Different", start()).is_err());
}

#[test]
fn retirement_waits_for_pending_turns_and_recovers_interrupted_work() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().into());
    store
        .accept_turn("first", "Important fact", start())
        .unwrap();
    store.retire_expired(start() + Duration::hours(24)).unwrap();
    assert!(store.pending_extractions().unwrap().is_empty());
    store.recover_interrupted().unwrap();
    assert_eq!(store.pending_extractions().unwrap().len(), 1);
    store.extraction_state("first", "complete").unwrap();
    assert!(store.pending_extractions().unwrap().is_empty());
}

#[test]
fn extraction_waits_for_late_generation_and_interruption_is_not_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().into());
    store.accept_turn("first", "Reply please", start()).unwrap();
    store
        .record_at(
            &json!({"type":"session.interrupted","sessionId":"first"}),
            start(),
        )
        .unwrap();
    store.retire_expired(start() + Duration::hours(24)).unwrap();
    assert!(store.pending_extractions().unwrap().is_empty());
    store
        .record_at(
            &json!({"type":"agent.generated","sessionId":"first","text":"Late answer"}),
            start(),
        )
        .unwrap();
    store
        .record_at(
            &json!({"type":"speech.completed","sessionId":"first"}),
            start(),
        )
        .unwrap();
    assert_eq!(store.pending_extractions().unwrap().len(), 1);
    assert_eq!(
        store.read(Some("first"), None).unwrap()["turns"][0]["delivery"],
        "interrupted"
    );
}

#[test]
fn unknown_format_is_never_reset_or_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("format.json"), r#"{"version":99}"#).unwrap();
    assert!(Store::new(directory.path().into()).prepare().is_err());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("format.json")).unwrap(),
        r#"{"version":99}"#
    );
}

#[test]
fn legacy_history_is_removed_once_without_touching_other_storage() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("2026-10-11");
    std::fs::create_dir(&old).unwrap();
    std::fs::write(old.join("false-wake.json"), "{}").unwrap();
    std::fs::write(directory.path().join("MEMORY.md"), "keep").unwrap();
    let store = Store::new(directory.path().into());
    store.prepare().unwrap();
    assert!(!old.exists());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("MEMORY.md")).unwrap(),
        "keep"
    );
    store.accept_turn("new", "Hello", start()).unwrap();
    store.prepare().unwrap();
    assert_eq!(
        store.read(None, None).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn recovery_context_preserves_tools_and_delivery_and_can_be_reset() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().into());
    store.accept_turn("first", "Set a timer", start()).unwrap();
    store.record_at(&json!({"sessionId":"first","type":"agent.tool","name":"set_timer","success":true,"result":{"id":"timer-1"}}), start()).unwrap();
    store
        .record_at(
            &json!({"sessionId":"first","type":"agent.response","text":"Timer set"}),
            start(),
        )
        .unwrap();
    store
        .record_at(
            &json!({"sessionId":"first","type":"session.interrupted"}),
            start(),
        )
        .unwrap();
    store
        .accept_turn("second", "Cancel it", start() + Duration::seconds(1))
        .unwrap();
    let context = store.context("first", "second").unwrap();
    assert!(
        context.contains("timer-1")
            && context.contains("interrupted")
            && !context.contains("Cancel it")
    );
    store.reset_context(start() + Duration::seconds(2)).unwrap();
    assert_eq!(store.context("first", "second").unwrap(), "");
}
