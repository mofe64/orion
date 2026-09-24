use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub at: String,
    pub event: Value,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub session_id: String,
    pub started_at: String,
    pub updated_at: String,
    pub events: Vec<Entry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub session_id: String,
    pub started_at: String,
    pub updated_at: String,
    pub command: Option<String>,
    pub reply: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub items: Vec<Summary>,
    pub next_cursor: Option<String>,
}

pub fn directory() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("ORION_VOICE_HISTORY_DIR") {
        return Ok(PathBuf::from(path));
    }
    if std::env::var("ORION_ONBOARD").as_deref() != Ok("1") {
        return Err("Voice history is stored only on Orion".into());
    }
    Ok(
        PathBuf::from(std::env::var_os("HOME").ok_or("Home is unavailable")?)
            .join(".local/share/orion/voice-stack/history"),
    )
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn write(path: &Path, turn: &Turn) -> Result<(), String> {
    fs::create_dir_all(path.parent().ok_or("History folder is unavailable")?)
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path.parent().unwrap().parent().unwrap(),
            fs::Permissions::from_mode(0o700),
        )
        .map_err(|e| e.to_string())?;
        fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(turn).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub struct History {
    root: PathBuf,
    active: HashMap<String, PathBuf>,
}
impl History {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            active: HashMap::new(),
        }
    }

    pub fn record(&mut self, event: &Value) -> Result<(), String> {
        if event["type"] == "speech.chunk" || event["type"] == "speech.audio" {
            return Ok(());
        }
        let Some(id) = event["sessionId"].as_str().filter(|id| safe_id(id)) else {
            return Ok(());
        };
        let now = Utc::now().to_rfc3339();
        let path = self
            .active
            .entry(id.to_owned())
            .or_insert_with(|| self.root.join(&now[..10]).join(format!("{id}.json")))
            .clone();
        let mut turn: Turn = if path.exists() {
            serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        } else {
            Turn {
                session_id: id.into(),
                started_at: now.clone(),
                updated_at: now.clone(),
                events: Vec::new(),
            }
        };
        if turn.events.len() >= 512 {
            return Ok(());
        }
        turn.updated_at = now.clone();
        turn.events.push(Entry {
            at: now,
            event: event.clone(),
        });
        write(&path, &turn)
    }
}

fn files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let days = match fs::read_dir(root) {
        Ok(days) => days,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut paths = Vec::new();
    for day in days {
        let day = day.map_err(|e| e.to_string())?;
        if !day.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        for file in fs::read_dir(day.path()).map_err(|e| e.to_string())? {
            let file = file.map_err(|e| e.to_string())?;
            if file.path().extension().is_some_and(|ext| ext == "json") {
                paths.push(file.path());
            }
        }
    }
    Ok(paths)
}

pub fn read(root: &Path, session_id: Option<&str>, before: Option<&str>) -> Result<Value, String> {
    if session_id.is_some_and(|id| !safe_id(id)) {
        return Err("Invalid voice turn ID".into());
    }
    if before.is_some_and(|cursor| cursor.len() > 128) {
        return Err("Invalid history page".into());
    }
    let mut turns = Vec::new();
    for path in files(root)? {
        if session_id.is_some_and(|id| path.file_stem().and_then(|name| name.to_str()) != Some(id))
        {
            continue;
        }
        let turn: Turn = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if session_id.is_some() {
            return serde_json::to_value(turn).map_err(|e| e.to_string());
        }
        turns.push(turn);
    }
    if session_id.is_some() {
        return Err("Voice turn was not found".into());
    }
    turns.sort_by(|a, b| {
        b.started_at
            .cmp(&a.started_at)
            .then_with(|| b.session_id.cmp(&a.session_id))
    });
    let mut filtered = turns.into_iter().filter(|turn| {
        before.is_none_or(|cursor| {
            format!("{}|{}", turn.started_at, turn.session_id).as_str() < cursor
        })
    });
    let page: Vec<_> = filtered.by_ref().take(200).collect();
    let has_more = filtered.next().is_some();
    let next_cursor = if has_more {
        page.last()
            .map(|turn| format!("{}|{}", turn.started_at, turn.session_id))
    } else {
        None
    };
    let summaries: Vec<Summary> = page
        .into_iter()
        .map(|turn| {
            let find = |kind: &str, key: &str| {
                turn.events
                    .iter()
                    .rev()
                    .find(|entry| entry.event["type"] == kind)
                    .and_then(|entry| entry.event[key].as_str())
                    .map(str::to_owned)
            };
            Summary {
                session_id: turn.session_id,
                started_at: turn.started_at,
                updated_at: turn.updated_at,
                command: find("transcript.final", "text"),
                reply: find("agent.response", "text"),
                error: find("worker.error", "message"),
            }
        })
        .collect();
    serde_json::to_value(Page {
        items: summaries,
        next_cursor,
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn persists_turns_and_reads_newest_first() {
        let root = std::env::temp_dir().join(format!("orion-history-{}", uuid::Uuid::new_v4()));
        let mut history = History::new(root.clone());
        history
            .record(&json!({"sessionId":"first","type":"transcript.final","text":"set a timer"}))
            .unwrap();
        history
            .record(
                &json!({"sessionId":"first","type":"agent.tool","name":"set_timer","success":true}),
            )
            .unwrap();
        let turn = read(&root, Some("first"), None).unwrap();
        assert_eq!(turn["events"].as_array().unwrap().len(), 2);
        assert_eq!(
            read(&root, None, None).unwrap()["items"][0]["command"],
            "set a timer"
        );
        assert!(read(&root, Some("../first"), None).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn older_turns_remain_accessible_after_first_page() {
        let root = std::env::temp_dir().join(format!("orion-history-{}", uuid::Uuid::new_v4()));
        for number in 0..201 {
            let id = format!("turn{number:03}");
            write(
                &root.join("2026-09-22").join(format!("{id}.json")),
                &Turn {
                    session_id: id,
                    started_at: "2026-09-22T12:00:00+00:00".into(),
                    updated_at: "2026-09-22T12:00:00+00:00".into(),
                    events: vec![],
                },
            )
            .unwrap();
        }
        let first = read(&root, None, None).unwrap();
        assert_eq!(first["items"].as_array().unwrap().len(), 200);
        let cursor = first["nextCursor"].as_str().unwrap();
        let second = read(&root, None, Some(cursor)).unwrap();
        assert_eq!(second["items"].as_array().unwrap().len(), 1);
        assert_eq!(second["items"][0]["sessionId"], "turn000");
        assert!(second["nextCursor"].is_null());
        fs::remove_dir_all(root).unwrap();
    }
}
