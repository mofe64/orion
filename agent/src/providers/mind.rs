use chrono::Timelike;
use serde::Deserialize;
use std::{io::Read, path::Path};

#[derive(Deserialize)]
struct State {
    energy: f64,
    mood: f64,
    label: Label,
    attention: Attention,
    updated_at: f64,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Label {
    Sleepy,
    Cheerful,
    Subdued,
    Curious,
    Content,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Attention {
    None,
    Speaker { track_id: u64 },
    Person { track_id: u64 },
    Hands,
    Object { class: String },
}

pub(super) fn context() -> Option<String> {
    let path = std::env::var_os("HOME")
        .map(|home| std::path::PathBuf::from(home).join(".local/share/orion/mind.json"))?;
    let now = chrono::Local::now();
    read(&path, now.timestamp_millis() as f64 / 1000., now.hour())
}

fn read(path: &Path, wall: f64, hour: u32) -> Option<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(4097)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    let state: State = serde_json::from_slice(&bytes).ok()?;
    if !(0. ..=600.).contains(&(wall - state.updated_at))
        || !(0. ..=1.).contains(&state.energy)
        || !(-1. ..=1.).contains(&state.mood)
    {
        return None;
    }
    let attention = match state.attention {
        Attention::None => "resting freely",
        Attention::Speaker { track_id } if track_id > 0 => "on the speaker",
        Attention::Person { track_id } if track_id > 0 => "on a person",
        Attention::Hands => "on the hands",
        Attention::Object { class } if !class.is_empty() => "on an object",
        _ => return None,
    };
    let energy = if state.energy < 0.3 {
        "low"
    } else if state.energy < 0.7 {
        "moderate"
    } else {
        "high"
    };
    let label = match state.label {
        Label::Sleepy => "sleepy",
        Label::Cheerful => "cheerful",
        Label::Subdued => "subdued",
        Label::Curious => "curious",
        Label::Content => "content",
    };
    let period = match hour {
        7..=11 => "morning",
        12..=17 => "afternoon",
        18..=21 => "evening",
        _ => "night",
    };
    Some(format!(
        "Orion's current state: energy {energy} ({period}), mood {label}, attention {attention}."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_state_is_brief_and_stale_missing_or_corrupt_state_is_omitted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mind.json");
        assert!(read(&path, 1000., 20).is_none());
        std::fs::write(&path, r#"{"energy":0.2,"mood":0,"label":"content","attention":{"kind":"speaker","track_id":2},"updated_at":1000}"#).unwrap();
        assert_eq!(
            read(&path, 1600., 20).unwrap(),
            "Orion's current state: energy low (evening), mood content, attention on the speaker."
        );
        assert!(read(&path, 1601., 20).is_none());
        assert!(read(&path, 999., 20).is_none());
        std::fs::write(&path, "broken").unwrap();
        assert!(read(&path, 1000., 20).is_none());
        std::fs::write(&path, r#"{"energy":4,"mood":0,"label":"content","attention":{"kind":"none"},"updated_at":1000}"#).unwrap();
        assert!(read(&path, 1000., 20).is_none());
    }
}
