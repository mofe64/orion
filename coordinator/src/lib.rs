//! Voice orchestration with no Tauri or native model dependency.
mod buffer;
mod gateway;
mod hub;
mod pipeline;
mod service;
mod session;
mod speech;

pub use service::{Coordinator, CoordinatorConfig, CoordinatorConnection};
pub use speech::SpeechConfig;
