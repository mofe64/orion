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
        self.ensure_config(config)
    }

    fn ensure_config(&self, config: AgentConfig) -> Result<AgentHandle, String> {
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

    pub fn profile_handle(&self) -> Result<AgentHandle, String> {
        {
            let slot = self.0.lock().map_err(|e| e.to_string())?;
            if let Some((_, service)) = slot.as_ref() {
                return Ok(service.handle());
            }
        }
        let settings = crate::settings::load_voice_settings()?;
        self.ensure(&settings.model, &settings.effort)
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
        let history = tempfile::tempdir().unwrap();
        let config = |effort: &str| AgentConfig {
            model: "model".into(),
            effort: effort.into(),
            history_path: Some(history.path().into()),
            ..AgentConfig::default()
        };
        let first = manager.ensure_config(config("medium")).unwrap();
        let second = manager.ensure_config(config("medium")).unwrap();
        assert!(first.same_runtime(&second));
        let changed = manager.ensure_config(config("high")).unwrap();
        assert!(!first.same_runtime(&changed));
        manager.shutdown();
    }
}
