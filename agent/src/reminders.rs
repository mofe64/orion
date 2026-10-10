//! Next-conversation notices use agent storage, never the runtime clock scheduler.
use crate::{
    AgentConfig,
    memory::{self, Entry},
};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
fn path(config: &AgentConfig) -> Result<PathBuf, String> {
    Ok(config
        .memory_path
        .as_deref()
        .ok_or("Reminder storage is disabled")?
        .with_file_name("reminders.json"))
}
pub(crate) fn pending(config: &AgentConfig) -> Result<Vec<Entry>, String> {
    if config.memory_path.is_none() {
        return Ok(vec![]);
    }
    read(&path(config)?)
}
fn read(path: &Path) -> Result<Vec<Entry>, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = vec![];
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Reminder file is too large".into());
    }
    let entries: Vec<Entry> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if entries.len() > 200 || entries.iter().any(|entry| validate(&entry.text).is_err()) {
        return Err("Invalid reminder file".into());
    }
    let mut ids = std::collections::HashSet::new();
    if entries
        .iter()
        .any(|e| e.id.is_empty() || !ids.insert(&e.id))
    {
        return Err("Invalid reminder IDs".into());
    }
    Ok(entries)
}
fn write(path: &Path, entries: &[Entry]) -> Result<(), String> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().ok_or("Invalid path")?)
        .map_err(|e| e.to_string())?;
    temp.write_all(&serde_json::to_vec(entries).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
pub(crate) fn validate(text: &str) -> Result<(), String> {
    if !(1..=200).contains(&text.trim().chars().count()) || text.chars().any(char::is_control) {
        return Err("Use 1–200 printable characters for a reminder".into());
    }
    Ok(())
}
pub(crate) fn add(config: &AgentConfig, text: &str) -> Result<Entry, String> {
    validate(text)?;
    let path = path(config)?;
    let _lock = memory::edit_lock(&path)?;
    let mut entries = read(&path)?;
    if entries.len() >= 200 {
        return Err("At most 200 next-conversation reminders".into());
    }
    let entry = Entry {
        id: uuid::Uuid::new_v4().simple().to_string(),
        created: chrono::Utc::now().to_rfc3339(),
        text: text.trim().into(),
    };
    entries.push(entry.clone());
    write(&path, &entries)?;
    Ok(entry)
}
pub(crate) fn delivered(config: &AgentConfig, expected: &[Entry]) -> Result<(), String> {
    if expected.is_empty() {
        return Ok(());
    }
    let path = path(config)?;
    let _lock = memory::edit_lock(&path)?;
    let mut entries = read(&path)?;
    // Remove only the exact snapshot included in the reply with completed playback. A newly
    // appended reminder remains for a later conversation.
    entries.retain(|entry| !expected.contains(entry));
    write(&path, &entries)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delivery_preserves_a_concurrent_append() {
        let dir = tempfile::tempdir().unwrap();
        let config = AgentConfig {
            memory_path: Some(dir.path().join("MEMORY.md")),
            ..Default::default()
        };
        let first = add(&config, "call Mum").unwrap();
        let snapshot = pending(&config).unwrap();
        let second = add(&config, "bins").unwrap();
        delivered(&config, &snapshot).unwrap();
        assert_eq!(pending(&config).unwrap(), [second]);
        assert_ne!(
            first.id,
            snapshot.get(1).map(|e| &e.id).cloned().unwrap_or_default()
        );
    }
}
