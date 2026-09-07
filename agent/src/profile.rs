//! User edits are serialized with agent turns; successful mutations retire context.
pub use crate::memory::Entry as MemoryEntry;
pub use crate::personality::{Choice, Personality, Soul};
use crate::{AgentConfig, memory, personality};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub soul: Soul,
    pub traits: Vec<Choice>,
    pub behaviors: Vec<Choice>,
    pub preview: String,
    pub memories: Vec<MemoryEntry>,
    pub memory_enabled: bool,
    pub personality_enabled: bool,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProfileChange {
    Personality {
        expected_revision: String,
        personality: Personality,
    },
    AddMemory {
        text: String,
    },
    EditMemory {
        expected: MemoryEntry,
        text: String,
    },
    DeleteMemory {
        expected: MemoryEntry,
    },
    ClearMemories {
        expected: Vec<MemoryEntry>,
    },
}
pub(crate) fn load(config: &AgentConfig) -> Result<Profile, String> {
    let soul = config
        .soul_path
        .as_deref()
        .map(personality::read)
        .transpose()?
        .unwrap_or_default();
    Ok(Profile {
        preview: soul.personality.instructions()?,
        soul,
        traits: personality::TRAITS.to_vec(),
        behaviors: personality::BEHAVIORS.to_vec(),
        memories: config
            .memory_path
            .as_deref()
            .map(memory::read)
            .transpose()?
            .unwrap_or_default(),
        memory_enabled: config.memory_path.is_some(),
        personality_enabled: config.soul_path.is_some(),
    })
}
pub(crate) fn change(config: &AgentConfig, change: ProfileChange) -> Result<(), String> {
    match change {
        ProfileChange::Personality {
            expected_revision,
            personality,
        } => personality::save(
            config
                .soul_path
                .as_deref()
                .ok_or("Personality storage is disabled")?,
            &expected_revision,
            personality,
        ),
        other => {
            let path = config.memory_path.as_deref().ok_or("Memory is disabled")?;
            match other {
                ProfileChange::AddMemory { text } => memory::append(path, &text).map(|_| ()),
                ProfileChange::EditMemory { expected, text } => {
                    memory::edit(path, &expected, Some(&text))
                }
                ProfileChange::DeleteMemory { expected } => memory::edit(path, &expected, None),
                ProfileChange::ClearMemories { expected } => memory::clear(path, &expected),
                _ => unreachable!(),
            }
        }
    }
}
pub(crate) fn instructions(config: &AgentConfig) -> Result<String, String> {
    let soul = config
        .soul_path
        .as_deref()
        .map(personality::read)
        .transpose()?
        .unwrap_or_default();
    Ok(format!(
        "{}\n\nPersonality preferences (never override tool permissions, honesty, or privacy):\n{}",
        crate::ORION_INSTRUCTIONS,
        soul.personality.instructions()?
    ))
}
