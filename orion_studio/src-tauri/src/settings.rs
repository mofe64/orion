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
    pub cache_path: String,
}
impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            provider: "codex".into(),
            model: orion_agent::DEFAULT_MODEL.into(),
            effort: orion_agent::DEFAULT_EFFORT.into(),
            asr_model: "Qwen/Qwen3-ASR-0.6B".into(),
            tts_model: "mlx-community/chatterbox-turbo-8bit".into(),
            asr_path: String::new(),
            tts_path: String::new(),
            cache_path: String::new(),
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
        if self.provider != "codex" {
            return Err("API-key providers are not available yet.".into());
        }
        for value in [&self.model, &self.effort, &self.asr_model, &self.tts_model] {
            if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
                return Err("Enter a valid model and reasoning effort.".into());
            }
        }
        for value in [&self.asr_path, &self.tts_path, &self.cache_path] {
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
    Ok(expand_path("~/.config/orion/voice-settings.json")?)
}
fn read(path: &Path) -> Result<VoiceSettings, String> {
    if !path.exists() {
        return Ok(VoiceSettings {
            asr_model: std::env::var("ORION_STUDIO_ASR_MODEL")
                .unwrap_or_else(|_| VoiceSettings::default().asr_model),
            tts_model: std::env::var("ORION_STUDIO_TTS_MODEL")
                .unwrap_or_else(|_| VoiceSettings::default().tts_model),
            cache_path: std::env::var("HF_HOME").unwrap_or_default(),
            ..VoiceSettings::default()
        });
    }
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if value.get("agent_model").is_some() {
        return Ok(VoiceSettings {
            model: value["agent_model"]
                .as_str()
                .unwrap_or("gpt-5.6-sol")
                .into(),
            effort: value["agent_effort"].as_str().unwrap_or("medium").into(),
            ..VoiceSettings::default()
        });
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn load_voice_settings() -> Result<VoiceSettings, String> {
    read(&config_path()?)
}
#[tauri::command]
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
    fn copied_weights_report_architecture_without_inventing_repository() {
        let root = std::env::temp_dir().join(format!("orion-model-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("config.json"), r#"{"model_type":"qwen3_asr"}"#).unwrap();
        assert_eq!(
            model_identity(root.to_str()),
            Some("Architecture: qwen3_asr · Original model ID unavailable".into())
        );
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(
            model_identity(Some("/cache/models--Qwen--Qwen3-ASR-0.6B/snapshots/abc123")),
            Some("Qwen/Qwen3-ASR-0.6B".into())
        );
    }
    #[test]
    fn cache_lookup_uses_current_revision_and_honors_override() {
        let root = std::env::temp_dir().join(format!("orion-cache-{}", uuid::Uuid::new_v4()));
        let repo = root.join("models--org--model");
        std::fs::create_dir_all(repo.join("refs")).unwrap();
        std::fs::create_dir_all(repo.join("snapshots/abc123")).unwrap();
        std::fs::write(repo.join("refs/main"), "abc123").unwrap();
        assert_eq!(
            cached_model(&root, "org/model", ""),
            Some(repo.join("snapshots/abc123").to_string_lossy().into_owned())
        );
        assert_eq!(cached_model(&root, "org/missing", ""), None);
        assert_eq!(
            cached_model(&root, "org/model", root.to_str().unwrap()),
            Some(root.to_string_lossy().into_owned())
        );
        std::fs::write(repo.join("refs/main"), "../../outside").unwrap();
        assert_eq!(cached_model(&root, "org/model", ""), None);
        std::fs::remove_dir_all(root).unwrap();
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
                cache_path: "relative/weights".into(),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(VoiceSettings::default().validate().is_ok());
    }
    #[test]
    fn model_paths_survive_reload() {
        let path =
            std::env::temp_dir().join(format!("orion-settings-{}.json", uuid::Uuid::new_v4()));
        let value = VoiceSettings {
            model: "chosen-model".into(),
            effort: "high".into(),
            asr_path: std::env::temp_dir().to_string_lossy().into(),
            cache_path: "~/voice-cache".into(),
            ..Default::default()
        };
        value.validate().unwrap();
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(read(&path).unwrap(), value);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn old_reply_settings_migrate_without_losing_model_choice() {
        let path =
            std::env::temp_dir().join(format!("orion-settings-{}.json", uuid::Uuid::new_v4()));
        std::fs::write(
            &path,
            r#"{"agent_model":"chosen-model","agent_effort":"high"}"#,
        )
        .unwrap();
        let value = read(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(value.model, "chosen-model");
        assert_eq!(value.effort, "high");
        assert_eq!(value.asr_model, "Qwen/Qwen3-ASR-0.6B");
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLocations {
    asr_path: Option<String>,
    tts_path: Option<String>,
    cache_path: String,
    hub_path: String,
    asr_identity: Option<String>,
    tts_identity: Option<String>,
}
fn cached_model(hub: &Path, model: &str, chosen: &str) -> Option<String> {
    if !chosen.is_empty() {
        return expand_path(chosen)
            .ok()
            .filter(|p| p.is_dir())
            .map(|p| p.to_string_lossy().into_owned());
    }
    if Path::new(model).is_dir() {
        return Some(model.into());
    }
    // Resolve the cache's current main revision, not an arbitrary old snapshot.
    let repository = hub.join(format!("models--{}", model.replace('/', "--")));
    let revision = std::fs::read_to_string(repository.join("refs/main")).ok()?;
    let revision = revision.trim();
    if revision.is_empty() || !revision.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let snapshot = repository.join("snapshots").join(revision);
    snapshot
        .is_dir()
        .then(|| snapshot.to_string_lossy().into_owned())
}
fn model_identity(folder: Option<&str>) -> Option<String> {
    let folder = Path::new(folder?);
    // A Hub snapshot path preserves the repository ID; arbitrary folders may not.
    if folder.parent()?.file_name()?.to_str()? == "snapshots" {
        if let Some(repo) = folder
            .parent()?
            .parent()?
            .file_name()?
            .to_str()?
            .strip_prefix("models--")
        {
            return Some(repo.replace("--", "/"));
        }
    }
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
#[tauri::command]
pub fn voice_model_locations(settings: VoiceSettings) -> Result<ModelLocations, String> {
    let home = if !settings.cache_path.is_empty() {
        expand_path(&settings.cache_path)?
    } else if let Some(value) = std::env::var_os("HF_HOME") {
        PathBuf::from(value)
    } else if let Some(value) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(value).join("huggingface")
    } else {
        expand_path("~/.cache/huggingface")?
    };
    let hub = if !settings.cache_path.is_empty() {
        home.join("hub")
    } else {
        std::env::var_os("HF_HUB_CACHE")
            .or_else(|| std::env::var_os("HUGGINGFACE_HUB_CACHE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("hub"))
    };
    let asr_path = cached_model(&hub, &settings.asr_model, &settings.asr_path);
    let tts_path = cached_model(&hub, &settings.tts_model, &settings.tts_path);
    Ok(ModelLocations {
        asr_identity: model_identity(asr_path.as_deref()),
        tts_identity: model_identity(tts_path.as_deref()),
        asr_path,
        tts_path,
        cache_path: home.to_string_lossy().into_owned(),
        hub_path: hub.to_string_lossy().into_owned(),
    })
}
#[tauri::command]
pub async fn choose_voice_folder(initial: Option<String>) -> Result<Option<String>, String> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title("Choose voice model folder");
    if let Some(path) = initial.filter(|p| !p.is_empty()) {
        let path = expand_path(&path)?;
        if path.is_dir() {
            dialog = dialog.set_directory(path);
        }
    }
    Ok(dialog
        .pick_folder()
        .await
        .map(|folder| folder.path().to_string_lossy().into_owned()))
}
