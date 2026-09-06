mod codex;
mod service;

use serde::{Deserialize, Serialize};
pub use service::{AgentHandle, AgentService};
use std::path::PathBuf;

pub const DEFAULT_MODEL: &str = "gpt-5.6-sol";
pub const DEFAULT_EFFORT: &str = "medium";
pub const MAX_RESPONSE_CHARACTERS: usize = 800;
pub const ORION_INSTRUCTIONS: &str = "You are Orion, a conversational desk-lamp companion.\n\
Answer the user's spoken request in at most two concise sentences suitable for speech.\n\
Return only the words Orion should say. Do not use tools, inspect files, modify anything,\n\
or claim that a physical action happened.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConfig {
    pub model: String,
    pub effort: String,
    pub codex_bin: Option<PathBuf>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.into(),
            effort: DEFAULT_EFFORT.into(),
            codex_bin: std::env::var_os("ORION_STUDIO_CODEX_BIN")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub model: String,
    pub name: String,
    pub efforts: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentInfo {
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub runtime: String,
    pub models: Vec<ModelInfo>,
    pub conversation_id: String,
}

fn spoken_response(text: &str) -> Result<String, String> {
    let response = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if response.is_empty() {
        return Err("Codex returned no spoken response.".into());
    }
    if response.chars().count() <= MAX_RESPONSE_CHARACTERS {
        return Ok(response);
    }
    // Count Unicode characters, never slice a UTF-8 code point in half.
    let truncated: String = response.chars().take(MAX_RESPONSE_CHARACTERS).collect();
    let prefix = truncated
        .rsplit_once(' ')
        .map_or(truncated.as_str(), |(head, _)| head);
    Ok(format!("{}…", prefix.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_bounds_unicode_speech() {
        assert_eq!(
            spoken_response("  Hello,\n Orion. ").unwrap(),
            "Hello, Orion."
        );
        assert!(spoken_response(" \n").is_err());
        let reply = spoken_response(&"灯 ".repeat(900)).unwrap();
        assert!(reply.chars().count() <= MAX_RESPONSE_CHARACTERS + 1);
        assert!(reply.ends_with('…'));
    }
}
