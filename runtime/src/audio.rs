use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::{Error, Result};

pub const ORION_AUDIO_CARD: &str = "seeed2micvoicec";
pub const ORION_AUDIO_PCM_DEVICE: &str = "plughw:CARD=seeed2micvoicec,DEV=0";
pub const ORION_AMIXER_PATH: &str = "/usr/bin/amixer";
pub const ORION_APLAY_PATH: &str = "/usr/bin/aplay";

pub trait AudioDevice {
    fn play(&mut self, cue: &str) -> Result<()>;
    fn update(&mut self) -> Result<()> {
        Ok(())
    }
    fn is_playing(&self) -> bool {
        false
    }
    fn stop(&mut self) -> Result<()>;
}

#[derive(Clone, Debug)]
pub struct CueLibrary {
    cues: BTreeMap<String, PathBuf>,
}

impl CueLibrary {
    pub fn load(directory: impl AsRef<Path>) -> Result<Self> {
        let directory = directory.as_ref();
        if !directory.is_dir() {
            return Err(Error::Runtime(format!(
                "Audio cue library is not a directory: {}",
                directory.display()
            )));
        }

        let mut paths = fs::read_dir(directory)
            .map_err(|error| {
                Error::Runtime(format!(
                    "Could not read audio cue library '{}': {error}",
                    directory.display()
                ))
            })?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();

        let mut cues = BTreeMap::new();
        for path in paths {
            if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("wav") {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    Error::Runtime(format!(
                        "Audio cue filename is not valid UTF-8: {}",
                        path.display()
                    ))
                })?;
            validate_cue_name(name)?;
            validate_wav_header(&path)?;
            if cues.insert(name.to_owned(), path.clone()).is_some() {
                return Err(Error::Runtime(format!(
                    "Duplicate Orion audio cue name: {name}"
                )));
            }
        }

        if cues.is_empty() {
            return Err(Error::Runtime(format!(
                "Audio cue library contains no WAV files: {}",
                directory.display()
            )));
        }
        Ok(Self { cues })
    }

    pub fn cue(&self, name: &str) -> Result<&Path> {
        validate_cue_name(name)?;
        self.cues
            .get(name)
            .map(PathBuf::as_path)
            .ok_or_else(|| Error::InvalidArgument(format!("Unknown Orion audio cue: {name}")))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.cues.contains_key(name)
    }

    pub fn names(&self) -> Vec<String> {
        self.cues.keys().cloned().collect()
    }
}

fn validate_cue_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(Error::InvalidArgument(format!(
            "Invalid Orion audio cue name '{name}'; use letters, digits, '-' or '_'."
        )));
    }
    Ok(())
}

fn validate_wav_header(path: &Path) -> Result<()> {
    let mut file = fs::File::open(path)?;
    let mut header = [0_u8; 12];
    file.read_exact(&mut header).map_err(|error| {
        Error::Runtime(format!(
            "Could not read WAV header '{}': {error}",
            path.display()
        ))
    })?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err(Error::Runtime(format!(
            "Audio cue is not a RIFF/WAVE file: {}",
            path.display()
        )));
    }
    Ok(())
}

pub fn configure_respeaker_v2_mixer(card_name: &str) -> Result<()> {
    if card_name.trim().is_empty() {
        return Err(Error::InvalidArgument(
            "ALSA audio card name cannot be empty.".into(),
        ));
    }

    let settings: &[&[&str]] = &[
        &["sset", "PCM", "-20dB"],
        &["sset", "Right DAC Mux", "DAC_R1"],
        &["sset", "Right Line Mixer DACR1", "on"],
        &["sset", "Line DAC", "0dB"],
        &["sset", "Line", "0dB", "unmute"],
    ];
    for setting in settings {
        let output = Command::new(ORION_AMIXER_PATH)
            .args(["-q", "-c", card_name, "--"])
            .args(*setting)
            .output()
            .map_err(|error| {
                Error::Runtime(format!(
                    "Could not run ReSpeaker mixer command '{}': {error}",
                    ORION_AMIXER_PATH
                ))
            })?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(Error::Runtime(format!(
                "ReSpeaker mixer configuration failed for '{}': {}",
                setting[1],
                if detail.is_empty() {
                    output.status.to_string()
                } else {
                    detail
                }
            )));
        }
    }
    Ok(())
}

pub struct AlsaAudioDevice {
    cues: CueLibrary,
    pcm_device: String,
    player_path: PathBuf,
    child: Option<Child>,
    active_cue: Option<String>,
}

impl AlsaAudioDevice {
    pub fn new(
        cues: CueLibrary,
        pcm_device: impl Into<String>,
        player_path: impl Into<PathBuf>,
    ) -> Result<Self> {
        let pcm_device = pcm_device.into();
        if pcm_device.trim().is_empty() {
            return Err(Error::InvalidArgument(
                "ALSA PCM playback device cannot be empty.".into(),
            ));
        }
        let player_path = player_path.into();
        if !player_path.is_file() {
            return Err(Error::Runtime(format!(
                "WAV player executable does not exist: {}",
                player_path.display()
            )));
        }
        Ok(Self {
            cues,
            pcm_device,
            player_path,
            child: None,
            active_cue: None,
        })
    }

    pub fn active_cue(&self) -> Option<&str> {
        self.active_cue.as_deref()
    }
}

impl AudioDevice for AlsaAudioDevice {
    fn play(&mut self, cue: &str) -> Result<()> {
        self.update()?;
        if let Some(active) = self.active_cue() {
            return Err(Error::InvalidState(format!(
                "Audio cue '{active}' is still playing."
            )));
        }
        let path = self.cues.cue(cue)?;
        let child = Command::new(&self.player_path)
            .args(["-q", "-D", &self.pcm_device])
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                Error::Runtime(format!(
                    "Could not start audio cue '{cue}' with '{}': {error}",
                    self.player_path.display()
                ))
            })?;
        self.child = Some(child);
        self.active_cue = Some(cue.to_owned());
        Ok(())
    }

    fn update(&mut self) -> Result<()> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        let Some(status) = child.try_wait()? else {
            return Ok(());
        };
        let cue = self.active_cue.take().unwrap_or_else(|| "unknown".into());
        self.child = None;
        if !status.success() {
            return Err(Error::Runtime(format!(
                "Audio cue '{cue}' failed with {status}."
            )));
        }
        Ok(())
    }

    fn is_playing(&self) -> bool {
        self.child.is_some()
    }

    fn stop(&mut self) -> Result<()> {
        let Some(mut child) = self.child.take() else {
            self.active_cue = None;
            return Ok(());
        };
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        child.wait()?;
        self.active_cue = None;
        Ok(())
    }
}

impl Drop for AlsaAudioDevice {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Debug, Default)]
pub struct UnavailableAudioDevice;

impl AudioDevice for UnavailableAudioDevice {
    fn play(&mut self, cue: &str) -> Result<()> {
        Err(Error::InvalidState(format!(
            "Audio cue '{cue}' cannot play because Orion's audio backend is not configured."
        )))
    }

    fn stop(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AudioCommand {
    Play(String),
    Stop,
}

#[derive(Debug)]
pub struct RecordingAudioDevice {
    commands: Vec<AudioCommand>,
    playing: bool,
    auto_complete: bool,
}

impl Default for RecordingAudioDevice {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            playing: false,
            auto_complete: true,
        }
    }
}

impl RecordingAudioDevice {
    pub fn blocking() -> Self {
        Self {
            auto_complete: false,
            ..Self::default()
        }
    }

    pub fn commands(&self) -> &[AudioCommand] {
        &self.commands
    }

    pub fn finish(&mut self) {
        self.playing = false;
    }
}

impl AudioDevice for RecordingAudioDevice {
    fn play(&mut self, cue: &str) -> Result<()> {
        validate_cue_name(cue)?;
        if self.playing {
            return Err(Error::InvalidState(
                "A recorded audio cue is already playing.".into(),
            ));
        }
        self.commands.push(AudioCommand::Play(cue.to_owned()));
        self.playing = !self.auto_complete;
        Ok(())
    }

    fn is_playing(&self) -> bool {
        self.playing
    }

    fn stop(&mut self) -> Result<()> {
        self.playing = false;
        self.commands.push(AudioCommand::Stop);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_minimal_wav(path: &Path) {
        fs::write(path, b"RIFF\x04\x00\x00\x00WAVE").unwrap();
    }

    #[test]
    fn loads_named_wav_cues_and_rejects_path_like_names() {
        let directory = tempdir().unwrap();
        write_minimal_wav(&directory.path().join("acknowledge.wav"));
        fs::write(directory.path().join("notes.txt"), "ignored").unwrap();

        let cues = CueLibrary::load(directory.path()).unwrap();
        assert_eq!(cues.names(), vec!["acknowledge"]);
        assert!(cues.cue("acknowledge").unwrap().ends_with("acknowledge.wav"));
        assert!(cues.cue("../acknowledge").is_err());
        assert!(cues.cue("missing").is_err());
    }

    #[test]
    fn rejects_invalid_or_empty_wav_libraries() {
        let directory = tempdir().unwrap();
        assert!(CueLibrary::load(directory.path()).is_err());
        fs::write(directory.path().join("broken.wav"), b"not a wave").unwrap();
        assert!(CueLibrary::load(directory.path()).is_err());
    }

    #[test]
    fn records_audio_commands_and_models_completion() {
        let mut device = RecordingAudioDevice::blocking();
        device.play("acknowledge").unwrap();
        assert!(device.is_playing());
        assert!(device.play("another").is_err());
        device.finish();
        assert!(!device.is_playing());
        device.stop().unwrap();
        assert_eq!(
            device.commands(),
            &[AudioCommand::Play("acknowledge".into()), AudioCommand::Stop]
        );
        assert!(device.play(" ").is_err());
    }

    #[test]
    fn unavailable_audio_fails_instead_of_claiming_playback() {
        let mut device = UnavailableAudioDevice;
        let error = device.play("acknowledge").unwrap_err().to_string();
        assert!(error.contains("audio backend is not configured"));
        device.stop().unwrap();
    }
}
