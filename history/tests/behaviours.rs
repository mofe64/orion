use orion_history::{BEHAVIOUR_PAGE, BehaviourEvent, BehaviourWriter, Store};

fn event(id: usize) -> BehaviourEvent {
    BehaviourEvent {
        id: format!("boot-{id:04}"),
        at: "2026-10-12T11:00:00+00:00".into(),
        runtime_time_seconds: id as f64,
        category: if id.is_multiple_of(2) {
            "idle"
        } else {
            "attention"
        }
        .into(),
        behaviour: "object_glance".into(),
        phase: "completed".into(),
        target: Some("object:book:1".into()),
        target_category: Some("book".into()),
        reason: None,
        action_id: Some("action".into()),
        duration_seconds: Some(2.),
        session_id: None,
    }
}

#[test]
fn partial_crash_tail_is_ignored_and_pagination_handles_equal_times() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("behaviours")).unwrap();
    std::fs::write(
        directory.path().join("behaviours/2026-10-12.jsonl"),
        "{broken",
    )
    .unwrap();
    let writer = BehaviourWriter::start(directory.path().into()).unwrap();
    let sender = writer.sender();
    // Deliberately write in reverse ID order; the cursor must follow the read order.
    for id in (0..BEHAVIOUR_PAGE + 5).rev() {
        sender.send(event(id)).unwrap();
    }
    drop(writer); // A live producer clone must not deadlock shutdown.
    let store = Store::new(directory.path().into());
    let first = store.behaviours(None, None).unwrap();
    let second = store
        .behaviours(first["nextCursor"].as_str(), None)
        .unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(), BEHAVIOUR_PAGE);
    assert_eq!(second["items"].as_array().unwrap().len(), 5);
    assert_eq!(
        first["items"][0]["id"],
        format!("boot-{:04}", BEHAVIOUR_PAGE + 4)
    );
    assert_eq!(second["items"][4]["id"], "boot-0000");
    assert_eq!(
        store.behaviours(None, Some("attention")).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        BEHAVIOUR_PAGE / 2 + 2
    );
    assert!(sender.try_send(event(999)).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(directory.path().join("behaviours/2026-10-12.jsonl"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn another_process_adds_remark_words_that_read_back_with_the_log() {
    let directory = tempfile::tempdir().unwrap();
    orion_history::append_behaviour(
        directory.path(),
        "remark_words",
        "observed",
        Some("Is that a new mug?"),
        Some("session-1"),
    )
    .unwrap();
    let page = orion_history::Store::new(directory.path().into())
        .behaviours(None, None)
        .unwrap();
    assert_eq!(page["items"][0]["behaviour"], "remark_words");
    assert_eq!(page["items"][0]["reason"], "Is that a new mug?");
}
