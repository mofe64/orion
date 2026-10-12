//! Accepted turns and conversation history share the agent's file store.
pub use orion_history::{Store as History, directory};
use serde_json::Value;
use std::path::Path;

pub fn read(
    root: &Path,
    conversation_id: Option<&str>,
    before: Option<&str>,
) -> Result<Value, String> {
    History::new(root.into()).read(conversation_id, before)
}
