use std::path::PathBuf;

pub const DEFAULT_MODEL: &str = "gpt-5.6-sol";
pub const DEFAULT_EFFORT: &str = "medium";
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConfig {
    pub model: String,
    pub effort: String,
    pub codex_bin: Option<PathBuf>,
    pub soul_path: Option<PathBuf>,
    pub memory_path: Option<PathBuf>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            soul_path: std::env::var_os("ORION_SOUL_PATH")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(|home| PathBuf::from(home).join(".local/share/orion/SOUL.md"))
                }),
            memory_path: std::env::var_os("ORION_MEMORY_PATH")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(|home| PathBuf::from(home).join(".local/share/orion/MEMORY.md"))
                }),
            model: DEFAULT_MODEL.into(),
            effort: DEFAULT_EFFORT.into(),
            codex_bin: std::env::var_os("ORION_STUDIO_CODEX_BIN")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
        }
    }
}
