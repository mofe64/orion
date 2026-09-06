use orion_agent::{AgentConfig, AgentHandle, AgentService};
use std::sync::Mutex;

/// Separate from CoordinatorManager so voice model reloads preserve context.
#[derive(Default)]
pub struct AgentManager(Mutex<Option<(AgentConfig, AgentService)>>);

impl AgentManager {
    pub fn ensure(&self, model: &str, effort: &str) -> Result<AgentHandle, String> {
        let config = AgentConfig {
            model: model.into(),
            effort: effort.into(),
            ..AgentConfig::default()
        };
        let mut slot = self.0.lock().map_err(|e| e.to_string())?;
        if let Some((current, service)) = slot.as_ref()
            && current == &config
        {
            return Ok(service.handle());
        }
        *slot = None;
        let service = AgentService::start(config.clone())?;
        let connection = service.handle();
        *slot = Some((config, service));
        Ok(connection)
    }

    pub fn shutdown(&self) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuses_agent_until_agent_settings_change() {
        let manager = AgentManager::default();
        let first = manager.ensure("model", "medium").unwrap();
        let second = manager.ensure("model", "medium").unwrap();
        assert!(first.same_runtime(&second));
        let changed = manager.ensure("model", "high").unwrap();
        assert!(!first.same_runtime(&changed));
        manager.shutdown();
    }
}
