use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
};

const END: &str = "<!-- memoryEntryEnd -->";
const MAX_FILE: u64 = 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub created: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<MemorySource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemorySource {
    pub conversation_id: String,
    pub conversation_started_at: String,
    pub turn_ids: Vec<String>,
    pub candidate_id: String,
}

pub(crate) fn read(path: &Path) -> Result<Vec<Entry>, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.to_string()),
    };
    let mut text = String::new();
    file.take(MAX_FILE + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() as u64 > MAX_FILE {
        return Err("Memory file exceeds 1 MiB".into());
    }
    let mut entries = vec![];
    let mut rest = text.trim();
    while !rest.is_empty() {
        let (header, body) = rest.split_once('\n').ok_or("Invalid memory header")?;
        let meta = header
            .strip_prefix("<!-- memoryEntry ")
            .and_then(|s| s.strip_suffix(" -->"))
            .ok_or("Invalid memory delimiter")?;
        let value: serde_json::Value =
            serde_json::from_str(meta).map_err(|_| "Invalid memory metadata")?;
        let id = value["id"].as_str().ok_or("Missing memory ID")?.to_owned();
        let created = value["created"]
            .as_str()
            .ok_or("Missing memory date")?
            .to_owned();
        let (text, tail) = body.split_once(END).ok_or("Incomplete memory entry")?;
        validate(text.trim())?;
        if entries.iter().any(|entry: &Entry| entry.id == id) {
            return Err("Duplicate memory ID".into());
        }
        entries.push(Entry {
            id,
            created,
            text: text.trim().into(),
            source: value
                .get("source")
                .filter(|v| !v.is_null())
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| "Invalid memory source")?,
        });
        rest = tail.trim();
    }
    Ok(entries)
}
fn validate(text: &str) -> Result<(), String> {
    if text.trim().is_empty()
        || text.len() > 2000
        || text.contains("<!--")
        || text.contains("-->")
        || text.contains('\0')
    {
        return Err("Memory must contain 1–2000 bytes and no reserved markers".into());
    }
    Ok(())
}

#[derive(Default, Serialize, Deserialize)]
struct AutomaticLedger {
    blocked_before: Option<String>,
    receipts: std::collections::BTreeMap<String, String>,
}
fn ledger_path(path: &Path) -> std::path::PathBuf {
    path.with_extension("automatic.json")
}
fn ledger(path: &Path) -> Result<AutomaticLedger, String> {
    match std::fs::read(ledger_path(path)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AutomaticLedger::default()),
        Err(e) => Err(e.to_string()),
    }
}
/// Caller holds the memory lock. Explicit editing/forgetting prevents retired
/// transcripts predating the edit from putting old facts back into memory.
fn block_automatic(path: &Path) -> Result<(), String> {
    let mut ledger = ledger(path)?;
    ledger.blocked_before = Some(chrono::Utc::now().to_rfc3339());
    orion_history::atomic_write(&ledger_path(path), &ledger)
}

pub(crate) fn append_extracted(
    path: &Path,
    text: &str,
    source: MemorySource,
    expected: &[Entry],
) -> Result<Option<Entry>, String> {
    validate(text)?;
    let _lock = edit_lock(path)?;
    let mut entries = read(path)?;
    let mut ledger = ledger(path)?;
    // A receipt is authoritative even if its memory was subsequently deleted.
    if ledger.receipts.contains_key(&source.candidate_id) {
        return Ok(None);
    }
    if let Some(entry) = entries.iter().find(|e| e.id == source.candidate_id) {
        let result = entry.clone();
        ledger.receipts.insert(source.candidate_id, "saved".into());
        orion_history::atomic_write(&ledger_path(path), &ledger)?;
        return Ok(Some(result));
    }
    if entries != expected {
        return Err("Memories changed during extraction; review again".into());
    }
    let blocked = match ledger.blocked_before.as_ref() {
        Some(at) => {
            chrono::DateTime::parse_from_rfc3339(&source.conversation_started_at)
                .map_err(|e| e.to_string())?
                <= chrono::DateTime::parse_from_rfc3339(at).map_err(|e| e.to_string())?
        }
        None => false,
    };
    if blocked {
        ledger
            .receipts
            .insert(source.candidate_id, "suppressed".into());
        orion_history::atomic_write(&ledger_path(path), &ledger)?;
        return Ok(None);
    }
    let canonical = |value: &str| {
        value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    if entries
        .iter()
        .any(|e| canonical(&e.text) == canonical(text))
    {
        ledger
            .receipts
            .insert(source.candidate_id, "duplicate".into());
        orion_history::atomic_write(&ledger_path(path), &ledger)?;
        return Ok(None);
    }
    let entry = Entry {
        id: source.candidate_id.clone(),
        created: chrono::Utc::now().to_rfc3339(),
        text: text.trim().into(),
        source: Some(source.clone()),
    };
    entries.push(entry.clone());
    write(path, &entries)?;
    ledger.receipts.insert(source.candidate_id, "saved".into());
    orion_history::atomic_write(&ledger_path(path), &ledger)?;
    Ok(Some(entry))
}
pub(crate) fn append(path: &Path, text: &str) -> Result<Entry, String> {
    validate(text)?;
    let _lock = edit_lock(path)?;
    let mut entries = read(path)?;
    if let Some(index) = entries.iter().position(|entry| entry.text == text.trim()) {
        return Ok(entries.remove(index));
    }
    let entry = Entry {
        id: uuid::Uuid::new_v4().simple().to_string(),
        created: chrono::Utc::now().to_rfc3339(),
        text: text.trim().into(),
        source: None,
    };
    entries.push(entry.clone());
    write(path, &entries)?;
    Ok(entry)
}
fn write(path: &Path, entries: &[Entry]) -> Result<(), String> {
    let mut content = String::new();
    for value in entries {
        content.push_str(&format!(
            "<!-- memoryEntry {} -->\n{}\n{END}\n\n",
            serde_json::json!({"id":value.id,"created":value.created,"source":value.source}),
            value.text
        ));
    }
    if content.len() as u64 > MAX_FILE {
        return Err("Memory file is full".into());
    }
    let parent = path.parent().ok_or("Invalid memory path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    // All writers hold the same file lock. Atomic replacement also
    // protects existing entries if the process exits during a save.
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(content.as_bytes())
        .map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn edit(path: &Path, expected: &Entry, replacement: Option<&str>) -> Result<(), String> {
    if let Some(text) = replacement {
        validate(text)?;
    }
    let _lock = edit_lock(path)?;
    let mut entries = read(path)?;
    let index = entries
        .iter()
        .position(|entry| entry.id == expected.id)
        .ok_or("Memory no longer exists. Refresh the list.")?;
    if entries[index] != *expected {
        return Err("This memory changed. Refresh before editing it.".into());
    }
    block_automatic(path)?;
    if let Some(text) = replacement {
        entries[index].text = text.trim().into();
    } else {
        entries.remove(index);
    }
    write(path, &entries)
}
pub(crate) fn clear(path: &Path, expected: &[Entry]) -> Result<(), String> {
    let _lock = edit_lock(path)?;
    if read(path)? != expected {
        return Err("Memories changed. Refresh before clearing them.".into());
    }
    block_automatic(path)?;
    write(path, &[])
}
pub(crate) struct EditLock(std::fs::File);
impl Drop for EditLock {
    fn drop(&mut self) {
        // A concurrently spawned process can briefly inherit this descriptor
        // before exec closes it. Release the lock explicitly rather than wait
        // for every copy of the open file description to close.
        let _ = self.0.unlock();
    }
}

pub(crate) fn edit_lock(path: &Path) -> Result<EditLock, String> {
    let parent = path.parent().ok_or("Invalid memory path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension(format!(
            "{}.lock",
            path.extension().and_then(|s| s.to_str()).unwrap_or("json")
        )))
        .map_err(|e| e.to_string())?;
    file.try_lock()
        .map_err(|_| "Memory is being edited; retry shortly")?;
    Ok(EditLock(file))
}

pub(crate) fn search(path: &Path, query: &str) -> Result<Vec<Entry>, String> {
    if query.trim().is_empty() || query.len() > 500 {
        return Err("Search requires 1–500 bytes".into());
    }
    let terms = words(query);
    let mut matches: Vec<_> = read(path)?
        .into_iter()
        .filter_map(|entry| {
            let text = words(&entry.text);
            let score = terms.iter().filter(|term| text.contains(*term)).count();
            (score > 0).then_some((score, entry))
        })
        .collect();
    matches.sort_by(|(score_a, a), (score_b, b)| {
        score_b.cmp(score_a).then_with(|| b.created.cmp(&a.created))
    });
    Ok(matches
        .into_iter()
        .take(8)
        .map(|(_, entry)| entry)
        .collect())
}
// Normalize whole words; a trailing s joins simple singular/plural pairs.
fn words(text: &str) -> std::collections::HashSet<String> {
    const STOP: &[&str] = &[
        "the", "a", "an", "is", "are", "am", "do", "does", "did", "to", "of", "in", "on", "at",
        "and", "or", "for", "me", "my", "where", "what", "who", "how",
    ];
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() > 1 && !STOP.contains(word))
        .map(|word| {
            if word.len() > 3 && word.ends_with('s') && !word.ends_with("ss") {
                word[..word.len() - 1].to_owned()
            } else {
                word.to_owned()
            }
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whole_words_rescue_the_ninth_result_and_sort_ties_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MEMORY.md");
        let mut entries = (0..20)
            .map(|i| Entry {
                id: i.to_string(),
                created: format!("2026-01-{:02}T00:00:00Z", i + 1),
                text: "I do enjoy dominoes and films".into(),
                source: None,
            })
            .collect::<Vec<_>>();
        entries[8].text = "You live in Woking with two cats".into();
        write(&path, &entries).unwrap();
        assert_eq!(search(&path, "where do I live").unwrap()[0].id, "8");
        assert_eq!(search(&path, "cat").unwrap()[0].id, "8");
        assert!(search(&path, "dom").unwrap().is_empty());
        assert_eq!(search(&path, "films").unwrap()[0].id, "19");
        assert!(search(&path, "I do").unwrap().is_empty());
    }
    #[test]
    fn finishing_an_edit_unlocks_even_with_an_inherited_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MEMORY.md");
        let lock = edit_lock(&path).unwrap();
        let inherited = lock.0.try_clone().unwrap();
        assert!(
            edit_lock(&path).is_err(),
            "active writers still exclude each other"
        );
        drop(lock);
        let next = edit_lock(&path)
            .expect("an inherited descriptor must not retain the completed edit lock");
        drop(next);
        drop(inherited);
    }

    #[test]
    fn stores_individual_entries_and_rejects_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MEMORY.md");
        let first = append(&path, "Prefers Celsius").unwrap();
        assert_eq!(append(&path, "Prefers Celsius").unwrap().id, first.id);
        append(&path, "Lives in Woking").unwrap();
        assert_eq!(search(&path, "celsius").unwrap()[0].id, first.id);
        assert_eq!(read(&path).unwrap().len(), 2);
        assert!(append(&path, END).is_err());
        std::fs::write(&path, "<!-- memoryEntry broken").unwrap();
        assert!(append(&path, "Must not overwrite").is_err());
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "<!-- memoryEntry broken"
        );
    }
}

#[cfg(test)]
mod editing_tests {
    use super::*;
    #[test]
    fn edits_and_clear_do_not_overwrite_concurrent_memories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MEMORY.md");
        let first = append(&path, "First fact").unwrap();
        let snapshot = read(&path).unwrap();
        let second = append(&path, "Second fact").unwrap();
        assert!(clear(&path, &snapshot).is_err());
        edit(&path, &first, Some("Updated fact")).unwrap();
        assert!(edit(&path, &first, None).is_err());
        assert_eq!(read(&path).unwrap()[1], second);
        edit(&path, &second, None).unwrap();
        assert_eq!(read(&path).unwrap().len(), 1);
    }
    fn source(id: &str) -> MemorySource {
        MemorySource {
            conversation_id: "conversation-1".into(),
            conversation_started_at: "2020-01-01T00:00:00Z".into(),
            turn_ids: vec!["turn-1".into()],
            candidate_id: id.into(),
        }
    }
    #[test]
    fn extraction_receipts_survive_retry_and_explicit_forgetting() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("MEMORY.md");
        let entry = append_extracted(
            &path,
            "An important enduring project",
            source("candidate-1"),
            &[],
        )
        .unwrap()
        .unwrap();
        assert!(
            append_extracted(&path, &entry.text, source("candidate-1"), &[])
                .unwrap()
                .is_none()
        );
        // Simulate a crash after MEMORY.md was saved and before its receipt was written.
        std::fs::remove_file(ledger_path(&path)).unwrap();
        assert_eq!(
            append_extracted(&path, &entry.text, source("candidate-1"), &[]).unwrap(),
            Some(entry.clone())
        );
        edit(&path, &entry, None).unwrap();
        assert!(
            append_extracted(&path, &entry.text, source("candidate-1"), &[])
                .unwrap()
                .is_none()
        );
        assert!(
            append_extracted(&path, "Rephrased old project", source("candidate-2"), &[])
                .unwrap()
                .is_none()
        );
        assert!(read(&path).unwrap().is_empty());
    }
    #[test]
    fn extraction_checks_the_exact_reviewed_snapshot_before_saving() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("MEMORY.md");
        append(&path, "A newer explicit fact").unwrap();
        assert!(append_extracted(&path, "Old extracted fact", source("candidate"), &[]).is_err());
        assert_eq!(read(&path).unwrap().len(), 1);
    }
}
