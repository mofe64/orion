use serde_json::{Value, json};

pub(super) fn schemas() -> Vec<Value> {
    ["play_music", "pause_music", "resume_music", "skip_song", "previous_song", "whats_playing"].into_iter().map(|name| json!({
        "type":"function", "name":name,
        "description": match name {
            "play_music" => "Search Spotify and play a requested song, artist, album or playlist on Orion. Choose the matching kind; use track for a song. Requires Spotify Premium and sign-in in Studio. Report errors plainly; confirm only after success.",
            "pause_music" => "Pause Spotify on Orion when asked.", "resume_music" => "Resume Spotify on Orion when asked.",
            "skip_song" => "Skip to the next Spotify song on Orion when asked.", "previous_song" => "Play the previous Spotify song on Orion when asked.",
            _ => "Read the Spotify title and artist currently playing on Orion. Do not invent a song when nothing is playing."
        },
        "inputSchema": if name == "play_music" {json!({"type":"object","additionalProperties":false,"required":["query","kind"],"properties":{"query":{"type":"string","minLength":1,"maxLength":500},"kind":{"type":"string","enum":["track","artist","album","playlist"]}}})}
            else {json!({"type":"object","additionalProperties":false,"properties":{},"required":[]})}
    })).collect()
}

pub(super) fn resolve(name: &str, arguments: Value) -> Result<Value, String> {
    let args = arguments.as_object().ok_or("Choose a music action.")?;
    let request = if name == "play_music" {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .filter(|query| !query.trim().is_empty() && query.chars().count() <= 500)
            .ok_or("Choose a song, artist, album or playlist.")?;
        let kind = args
            .get("kind")
            .and_then(Value::as_str)
            .filter(|kind| ["track", "artist", "album", "playlist"].contains(kind))
            .ok_or("Choose a song, artist, album or playlist.")?;
        if args.len() != 2 {
            return Err("Invalid music request.".into());
        }
        json!({"action":"play","query":query.trim(),"kind":kind})
    } else {
        if !args.is_empty() {
            return Err("Invalid music request.".into());
        }
        let action = match name {
            "pause_music" => "pause",
            "resume_music" => "resume",
            "skip_song" => "next",
            "previous_song" => "previous",
            "whats_playing" => "status",
            _ => return Err("Unknown music action.".into()),
        };
        json!({"action":action})
    };
    Ok(json!({"operation":"music","request":request}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playback_tools_are_strict_and_select_the_requested_kind() {
        for kind in ["track", "artist", "album", "playlist"] {
            assert_eq!(
                resolve("play_music", json!({"query":"test","kind":kind})).unwrap()["request"],
                json!({"action":"play","query":"test","kind":kind})
            );
        }
        for (name, action) in [
            ("pause_music", "pause"),
            ("resume_music", "resume"),
            ("skip_song", "next"),
            ("previous_song", "previous"),
            ("whats_playing", "status"),
        ] {
            assert_eq!(
                resolve(name, json!({})).unwrap()["request"]["action"],
                action
            );
            assert!(resolve(name, json!({"query":"extra"})).is_err());
        }
        assert!(resolve("play_music", json!({"query":"","kind":"track"})).is_err());
        assert!(resolve("play_music", json!({"query":"test","kind":"secret"})).is_err());
    }
}
