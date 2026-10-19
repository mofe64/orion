use serde_json::{Value, json};

pub(super) fn schemas() -> Vec<Value> {
    let mut schemas=["play_music", "pause_music", "resume_music", "skip_song", "previous_song", "whats_playing"].into_iter().map(|name| json!({
        "type":"function", "name":name,
        "description": match name {
            "play_music" => "Search Spotify and play a requested song, artist, album or playlist on Orion. Choose the matching kind; use track for a song. Requires Spotify Premium and sign-in in Studio. Report errors plainly; confirm only after success.",
            "pause_music" => "Pause Spotify on Orion when asked.", "resume_music" => "Resume Spotify on Orion when asked.",
            "skip_song" => "Skip to the next Spotify song on Orion when asked.", "previous_song" => "Play the previous Spotify song on Orion when asked.",
            _ => "Read the Spotify title and artist currently playing on Orion. Do not invent a song when nothing is playing."
        },
        "inputSchema": if name == "play_music" {json!({"type":"object","additionalProperties":false,"required":["query","kind"],"properties":{"query":{"type":"string","minLength":1,"maxLength":500},"kind":{"type":"string","enum":["track","artist","album","playlist"]}}})}
            else {json!({"type":"object","additionalProperties":false,"properties":{},"required":[]})}
    })).collect::<Vec<_>>();
    for name in ["dance", "stop_dancing"] {
        schemas.push(json!({"type":"function","name":name,"description":if name=="dance" {"Enable Orion's gentle movement while music plays. It stays still in silence and yields to listening and other activities. Confirm only after success."}else{"Turn off Orion's movement to music. Confirm only after success."},"inputSchema":{"type":"object","additionalProperties":false,"properties":{},"required":[]}}));
    }
    schemas.push(json!({"type":"function","name":"set_dance_mode","description":"Choose how often Orion dances to music: off, sometimes or always. Confirm only after success.","inputSchema":{"type":"object","additionalProperties":false,"required":["mode"],"properties":{"mode":{"type":"string","enum":["off","sometimes","always"]}}}}));
    schemas
}

pub(super) fn resolve(name: &str, arguments: Value) -> Result<Value, String> {
    let args = arguments.as_object().ok_or("Choose a music action.")?;
    if name == "set_dance_mode" {
        let mode = args
            .get("mode")
            .and_then(Value::as_str)
            .filter(|mode| ["off", "sometimes", "always"].contains(mode))
            .ok_or("Choose off, sometimes or always.")?;
        if args.len() != 1 {
            return Err("Invalid dance setting".into());
        }
        return Ok(json!({"operation":"music_movement","mode":mode}));
    }
    if matches!(name, "dance" | "stop_dancing") {
        if !args.is_empty() {
            return Err("Invalid music movement request.".into());
        }
        return Ok(json!({"operation":"music_movement","enabled":name=="dance"}));
    }
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
        for (name, enabled) in [("dance", true), ("stop_dancing", false)] {
            assert_eq!(
                resolve(name, json!({})).unwrap(),
                json!({"operation":"music_movement","enabled":enabled})
            );
            assert!(resolve(name, json!({"force":true})).is_err());
        }
        for mode in ["off", "sometimes", "always"] {
            assert_eq!(
                resolve("set_dance_mode", json!({"mode":mode})).unwrap(),
                json!({"operation":"music_movement","mode":mode})
            );
        }
        assert!(resolve("set_dance_mode", json!({"mode":"invalid"})).is_err());
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
