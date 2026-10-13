use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct VoiceSettings {
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub asr_model: String,
    pub tts_model: String,
    pub asr_path: String,
    pub tts_path: String,
}
impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            provider: "codex".into(),
            model: orion_agent::DEFAULT_MODEL.into(),
            effort: orion_agent::DEFAULT_EFFORT.into(),
            asr_model: "Qwen/Qwen3-ASR-0.6B".into(),
            tts_model: "piper-alba-medium".into(),
            asr_path: std::env::var("ORION_ASR_MODEL_DIR").unwrap_or_default(),
            tts_path: String::new(),
        }
    }
}
pub fn expand_path(value: &str) -> Result<PathBuf, String> {
    let path = if value == "~" || value.starts_with("~/") {
        PathBuf::from(std::env::var_os("HOME").ok_or("Home directory is unavailable")?)
            .join(value.trim_start_matches('~').trim_start_matches('/'))
    } else {
        PathBuf::from(value)
    };
    if !path.is_absolute() {
        return Err("Use an absolute folder path or a path starting with ~/".into());
    }
    Ok(path)
}
impl VoiceSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.tts_model.starts_with("pocket-") {
            return Err("That voice model is no longer supported.".into());
        }
        if self.tts_model != "piper-alba-medium" {
            return Err("Orion's Pi supports Piper Alba Medium for speech output.".into());
        }
        if self.provider != "codex" {
            return Err("API-key providers are not available yet.".into());
        }
        for value in [&self.model, &self.effort, &self.asr_model, &self.tts_model] {
            if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
                return Err("Enter a valid model and reasoning effort.".into());
            }
        }
        for value in [&self.asr_path, &self.tts_path] {
            if !value.is_empty() {
                if value.len() > 4096 || value.chars().any(char::is_control) {
                    return Err("Invalid model folder path.".into());
                }
                expand_path(value)?;
            }
        }
        for value in [&self.asr_path, &self.tts_path] {
            if !value.is_empty() && !expand_path(value)?.is_dir() {
                return Err(format!("Model folder does not exist: {value}"));
            }
        }
        Ok(())
    }
}
fn config_path() -> Result<PathBuf, String> {
    expand_path("~/.config/orion/voice-settings.json")
}
fn read(path: &Path) -> Result<VoiceSettings, String> {
    if !path.exists() {
        let settings = VoiceSettings {
            asr_model: std::env::var("ORION_STUDIO_ASR_MODEL")
                .unwrap_or_else(|_| VoiceSettings::default().asr_model),
            ..VoiceSettings::default()
        };
        return Ok(settings);
    }
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    // Discard retired fields while preserving the user's other saved choices.
    if let Some(object) = value.as_object_mut() {
        object.remove("ttsVoice");
        object.remove("cachePath");
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}
pub fn load_voice_settings() -> Result<VoiceSettings, String> {
    read(&config_path()?)
}
pub fn save_voice_settings(settings: VoiceSettings) -> Result<VoiceSettings, String> {
    settings.validate()?;
    let path = config_path()?;
    std::fs::create_dir_all(path.parent().ok_or("Settings folder is unavailable")?)
        .map_err(|e| e.to_string())?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, &path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_defaults_use_pi_speech_and_reject_other_voices() {
        let settings = VoiceSettings::default();
        assert_eq!(settings.tts_model, "piper-alba-medium");
        assert_eq!(
            settings.asr_path,
            std::env::var("ORION_ASR_MODEL_DIR").unwrap_or_default()
        );
        assert!(
            VoiceSettings {
                tts_model: "unsupported-voice".into(),
                ..settings
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn copied_weights_report_architecture_without_inventing_repository() {
        let root = std::env::temp_dir().join(format!("orion-model-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("config.json"), r#"{"model_type":"qwen3_asr"}"#).unwrap();
        assert_eq!(
            model_identity(root.to_str()),
            Some("Architecture: qwen3_asr · Original model ID unavailable".into())
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn model_locations_use_local_folders_and_saved_paths_take_priority() {
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("installed");
        let chosen = root.path().join("chosen voice");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::create_dir_all(&chosen).unwrap();
        assert_eq!(
            local_model("", "piper-alba-medium", installed.to_str()).unwrap(),
            Some(installed.to_string_lossy().into_owned())
        );
        assert_eq!(
            local_model(
                chosen.to_str().unwrap(),
                "piper-alba-medium",
                installed.to_str()
            )
            .unwrap(),
            Some(chosen.to_string_lossy().into_owned())
        );
        assert_eq!(local_model("", "Qwen/Qwen3-ASR-0.6B", None).unwrap(), None);
        let locations = serde_json::to_value(
            voice_model_locations(VoiceSettings {
                asr_path: installed.to_string_lossy().into_owned(),
                tts_path: chosen.to_string_lossy().into_owned(),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            locations,
            serde_json::json!({
                "asrPath": installed, "ttsPath": chosen, "asrIdentity": null, "ttsIdentity": null
            })
        );
        std::fs::remove_dir_all(&chosen).unwrap();
        assert_eq!(
            local_model(
                chosen.to_str().unwrap(),
                "piper-alba-medium",
                installed.to_str()
            )
            .unwrap(),
            None
        );
    }
    #[test]
    fn unsupported_providers_and_relative_paths_are_rejected() {
        assert!(
            VoiceSettings {
                provider: "api_key".into(),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            VoiceSettings {
                asr_path: "relative/weights".into(),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(VoiceSettings::default().validate().is_ok());
        assert!(
            VoiceSettings {
                tts_model: "pocket-int8".into(),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn old_settings_drop_retired_fields_and_preserve_saved_choices() {
        let path =
            std::env::temp_dir().join(format!("orion-settings-{}.json", uuid::Uuid::new_v4()));
        let value = VoiceSettings {
            model: "chosen-model".into(),
            effort: "high".into(),
            asr_path: std::env::temp_dir().to_string_lossy().into(),
            ..Default::default()
        };
        value.validate().unwrap();
        let mut old = serde_json::to_value(&value).unwrap();
        old["cachePath"] = serde_json::json!("~/unused-cache");
        old["ttsVoice"] = serde_json::json!("old-preset");
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let loaded = read(&path).unwrap();
        assert_eq!(loaded, value);
        let cleaned = serde_json::to_value(&loaded).unwrap();
        assert!(cleaned.get("cachePath").is_none());
        assert!(cleaned.get("ttsVoice").is_none());
        old["unknownSetting"] = serde_json::json!(true);
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(read(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLocations {
    asr_path: Option<String>,
    tts_path: Option<String>,
    asr_identity: Option<String>,
    tts_identity: Option<String>,
}
fn local_model(chosen: &str, model: &str, default: Option<&str>) -> Result<Option<String>, String> {
    let value = if !chosen.is_empty() {
        chosen
    } else if Path::new(model).is_absolute() || model.starts_with("~/") {
        model
    } else {
        default.unwrap_or_default()
    };
    if value.is_empty() {
        return Ok(None);
    }
    let path = expand_path(value)?;
    Ok(path.is_dir().then(|| path.to_string_lossy().into_owned()))
}
fn model_identity(folder: Option<&str>) -> Option<String> {
    let folder = Path::new(folder?);
    let config = folder.join("config.json");
    if std::fs::metadata(&config).ok()?.len() > 1024 * 1024 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(config).ok()?).ok()?;
    let architecture = value
        .get("model_type")
        .or_else(|| value.get("architecture"))
        .and_then(|v| v.as_str())?;
    Some(format!(
        "Architecture: {architecture} · Original model ID unavailable"
    ))
}
pub fn voice_model_locations(settings: VoiceSettings) -> Result<ModelLocations, String> {
    let asr_path = local_model(&settings.asr_path, &settings.asr_model, None)?;
    let piper = std::env::var("ORION_PIPER_MODEL_DIR").ok();
    let tts_path = local_model(&settings.tts_path, &settings.tts_model, piper.as_deref())?;
    Ok(ModelLocations {
        asr_identity: model_identity(asr_path.as_deref()),
        tts_identity: model_identity(tts_path.as_deref()),
        asr_path,
        tts_path,
    })
}
