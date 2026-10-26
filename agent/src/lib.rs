mod reaction;
pub use reaction::ReactionCue;
mod config;
mod context;
mod extraction;
mod prompt;
mod providers;
mod runtime;
mod types;

pub use config::{AgentConfig, DEFAULT_EFFORT, DEFAULT_MODEL};
pub use prompt::ORION_INSTRUCTIONS;
pub use runtime::{AgentHandle, AgentReply, AgentService, RemarkMemory};
pub use types::{AgentInfo, ModelInfo};

pub mod home_memory;
pub mod lists;
mod memory;
mod reminders;
pub mod tools;
pub mod weather;
pub use tools::AgentEvent;

mod personality;
pub mod profile;
