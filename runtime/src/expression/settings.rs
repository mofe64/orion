use serde::Deserialize;
use std::{io::Read, path::Path};

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub(crate) struct ActSettings {
    pub mind_idle_rhythm: bool,
    pub listen_lean_in: bool,
    pub thinking_beats: bool,
    pub wake_sleep_acts: bool,
    pub posture_drift: bool,
    pub act_sounds: bool,
}
impl Default for ActSettings {
    fn default() -> Self {
        Self {
            mind_idle_rhythm: true,
            listen_lean_in: true,
            thinking_beats: true,
            wake_sleep_acts: true,
            posture_drift: false,
            act_sounds: true,
        }
    }
}
impl ActSettings {
    pub fn load(path: &Path) -> Self {
        (|| {
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .ok()?
                .take(8193)
                .read_to_end(&mut bytes)
                .ok()?;
            (bytes.len() <= 8192)
                .then(|| serde_json::from_slice(&bytes).ok())
                .flatten()
        })()
        .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_preferences_preserve_defaults() {
        let value: ActSettings = serde_json::from_str(
            r#"{"posture_drift":true,"act_sounds":false,"object_glances":false}"#,
        )
        .unwrap();
        assert!(
            value.mind_idle_rhythm
                && value.listen_lean_in
                && value.thinking_beats
                && value.wake_sleep_acts
                && value.posture_drift
        );
        assert!(!value.act_sounds);
    }
}
