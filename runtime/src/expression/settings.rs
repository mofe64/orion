use serde::Deserialize;
use std::{io::Read, path::Path};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DanceMode {
    Off,
    #[default]
    Sometimes,
    Always,
}
impl<'de> Deserialize<'de> for DanceMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Bool(true) => Ok(Self::Sometimes),
            serde_json::Value::Bool(false) => Ok(Self::Off),
            serde_json::Value::String(mode) => match mode.as_str() {
                "off" => Ok(Self::Off),
                "sometimes" => Ok(Self::Sometimes),
                "always" => Ok(Self::Always),
                _ => Err(serde::de::Error::custom("Choose off, sometimes or always")),
            },
            _ => Err(serde::de::Error::custom("Invalid dance setting")),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub(crate) struct ActSettings {
    pub mind_idle_rhythm: bool,
    pub listen_lean_in: bool,
    pub thinking_beats: bool,
    pub wake_sleep_acts: bool,
    pub posture_drift: bool,
    pub act_sounds: bool,
    pub move_to_music: DanceMode,
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
            move_to_music: DanceMode::Sometimes,
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
        assert_eq!(value.move_to_music, DanceMode::Sometimes);
    }
    #[test]
    fn music_preference_restores_from_the_shared_behaviour_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("behaviours.json");
        assert_eq!(ActSettings::load(&path).move_to_music, DanceMode::Sometimes);
        std::fs::write(&path, r#"{"move_to_music":false,"act_sounds":true}"#).unwrap();
        let settings = ActSettings::load(&path);
        assert_eq!(settings.move_to_music, DanceMode::Off);
        assert!(settings.act_sounds);
    }
}
