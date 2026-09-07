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
            serde_json::json!({"id":value.id,"created":value.created}),
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
    write(path, &[])
}
fn edit_lock(path: &Path) -> Result<std::fs::File, String> {
    let parent = path.parent().ok_or("Invalid memory path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("md.lock"))
        .map_err(|e| e.to_string())?;
    file.try_lock()
        .map_err(|_| "Memory is being edited; retry shortly")?;
    Ok(file)
}

pub(crate) fn search(path: &Path, query: &str) -> Result<Vec<Entry>, String> {
    if query.trim().is_empty() || query.len() > 500 {
        return Err("Search requires 1–500 bytes".into());
    }
    let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut matches: Vec<_> = read(path)?
        .into_iter()
        .filter_map(|entry| {
            let text = entry.text.to_lowercase();
            let score = terms
                .iter()
                .filter(|term| text.contains(term.as_str()))
                .count();
            (score > 0).then_some((score, entry))
        })
        .collect();
    matches.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    Ok(matches
        .into_iter()
        .take(8)
        .map(|(_, entry)| entry)
        .collect())
}
#[cfg(test)]
mod tests {
    use super::*;
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
}
