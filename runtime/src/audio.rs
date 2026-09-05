use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread;
use std::time::Duration;

use crate::{Error, Result};

pub const ORION_AUDIO_CARD: &str = "seeed2micvoicec";
pub const ORION_AUDIO_PCM_DEVICE: &str = "plughw:CARD=seeed2micvoicec,DEV=0";
pub const ORION_AMIXER_PATH: &str = "/usr/bin/amixer";
pub const ORION_APLAY_PATH: &str = "/usr/bin/aplay";

pub trait AudioDevice {
    fn play(&mut self, cue: &str) -> Result<()>;
    fn play_file(&mut self, label: &str, path: &Path) -> Result<()>;
    fn start_pcm(&mut self, _label: &str) -> Result<()> {
        Err(Error::InvalidState(
            "Streaming audio is unavailable.".into(),
        ))
    }
    fn queue_pcm(&mut self, _pcm: &[u8]) -> Result<bool> {
        Err(Error::InvalidState(
            "Streaming audio is unavailable.".into(),
        ))
    }
    fn finish_pcm(&mut self) {}
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
        &["sset", "PCM", "0dB"],
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
    active_label: Option<String>,
    pcm_sender: Option<SyncSender<Vec<u8>>>,
    pcm_writer: Option<thread::JoinHandle<std::io::Result<()>>>,
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
            active_label: None,
            pcm_sender: None,
            pcm_writer: None,
        })
    }

    pub fn active_label(&self) -> Option<&str> {
        self.active_label.as_deref()
    }

    fn start_playback(&mut self, label: &str, path: &Path) -> Result<()> {
        self.update()?;
        if let Some(active) = self.active_label() {
            return Err(Error::InvalidState(format!(
                "Audio source '{active}' is still playing."
            )));
        }
        if label.trim().is_empty() {
            return Err(Error::InvalidArgument(
                "Audio playback label cannot be empty.".into(),
            ));
        }
        validate_wav_header(path)?;
        let child = Command::new(&self.player_path)
            .args(["-q", "-D", &self.pcm_device])
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                Error::Runtime(format!(
                    "Could not start audio source '{label}' with '{}': {error}",
                    self.player_path.display()
                ))
            })?;
        self.child = Some(child);
        self.active_label = Some(label.to_owned());
        Ok(())
    }
}

impl AudioDevice for AlsaAudioDevice {
    fn play(&mut self, cue: &str) -> Result<()> {
        let path = self.cues.cue(cue)?.to_owned();
        self.start_playback(cue, &path)
    }

    fn play_file(&mut self, label: &str, path: &Path) -> Result<()> {
        self.start_playback(label, path)
    }

    fn start_pcm(&mut self, label: &str) -> Result<()> {
        self.update()?;
        if self.child.is_some() {
            return Err(Error::InvalidState("Audio already playing.".into()));
        }
        let mut child = Command::new(&self.player_path)
            .args([
                "-q",
                "-D",
                &self.pcm_device,
                "-t",
                "raw",
                "-f",
                "S16_LE",
                "-r",
                "24000",
                "-c",
                "1",
                "--buffer-time=100000",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("piped audio input");
        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(4);
        self.pcm_writer = Some(thread::spawn(move || {
            loop {
                match receiver.recv_timeout(Duration::from_secs(3)) {
                    Ok(pcm) => stdin.write_all(&pcm)?,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "Speech stream stalled",
                        ));
                    }
                }
            }
        }));
        self.pcm_sender = Some(sender);
        self.child = Some(child);
        self.active_label = Some(label.into());
        Ok(())
    }

    fn queue_pcm(&mut self, pcm: &[u8]) -> Result<bool> {
        let sender = self
            .pcm_sender
            .as_ref()
            .ok_or_else(|| Error::InvalidState("No open audio stream.".into()))?;
        match sender.try_send(pcm.to_vec()) {
            Ok(()) => Ok(true),
            Err(TrySendError::Full(_)) => Ok(false),
            Err(TrySendError::Disconnected(_)) => {
                Err(Error::Runtime("Audio stream writer stopped.".into()))
            }
        }
    }

    fn finish_pcm(&mut self) {
        self.pcm_sender = None;
    }

    fn update(&mut self) -> Result<()> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        let Some(status) = child.try_wait()? else {
            return Ok(());
        };
        self.pcm_sender = None;
        self.child = None;
        let label = self.active_label.take().unwrap_or_else(|| "unknown".into());
        if let Some(writer) = self.pcm_writer.take() {
            writer
                .join()
                .map_err(|_| Error::Runtime("Audio writer panicked.".into()))??;
        }
        if !status.success() {
            return Err(Error::Runtime(format!(
                "Audio source '{label}' failed with {status}."
            )));
        }
        Ok(())
    }

    fn is_playing(&self) -> bool {
        self.child.is_some()
    }

    fn stop(&mut self) -> Result<()> {
        self.pcm_sender = None;
        let Some(mut child) = self.child.take() else {
            self.active_label = None;
            return Ok(());
        };
        if child.try_wait()?.is_none() {
            child.kill()?;
        }
        child.wait()?;
        if let Some(writer) = self.pcm_writer.take() {
            let _ = writer.join();
        }
        self.active_label = None;
        Ok(())
    }
}

impl Drop for AlsaAudioDevice {
    fn drop(&mut self) {
        let _ = self.stop();
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

    fn play_file(&mut self, label: &str, _path: &Path) -> Result<()> {
        Err(Error::InvalidState(format!(
            "Audio source '{label}' cannot play because Orion's audio backend is not configured."
        )))
    }

    fn stop(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AudioCommand {
    Play(String),
    PlayFile { label: String, path: PathBuf },
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

    fn play_file(&mut self, label: &str, path: &Path) -> Result<()> {
        if label.trim().is_empty() {
            return Err(Error::InvalidArgument(
                "Audio playback label cannot be empty.".into(),
            ));
        }
        validate_wav_header(path)?;
        if self.playing {
            return Err(Error::InvalidState(
                "A recorded audio source is already playing.".into(),
            ));
        }
        self.commands.push(AudioCommand::PlayFile {
            label: label.to_owned(),
            path: path.to_owned(),
        });
        self.playing = !self.auto_complete;
        Ok(())
    }

    fn start_pcm(&mut self, label: &str) -> Result<()> {
        self.playing = true;
        self.commands.push(AudioCommand::Play(label.into()));
        Ok(())
    }
    fn queue_pcm(&mut self, _pcm: &[u8]) -> Result<bool> {
        Ok(true)
    }
    fn finish_pcm(&mut self) {
        if self.auto_complete {
            self.playing = false;
        }
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

    #[cfg(unix)]
    #[test]
    fn streamed_pcm_reaches_one_process_in_order_and_eof_completes() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempdir().unwrap();
        write_minimal_wav(&directory.path().join("cue.wav"));
        let player = directory.path().join("player");
        fs::write(&player, "#!/bin/sh\ncat > \"$0.pcm\"\n").unwrap();
        fs::set_permissions(&player, fs::Permissions::from_mode(0o700)).unwrap();
        let mut device =
            AlsaAudioDevice::new(CueLibrary::load(directory.path()).unwrap(), "test", &player)
                .unwrap();
        device.start_pcm("speech").unwrap();
        assert!(device.queue_pcm(&[1, 0, 2, 0]).unwrap());
        assert!(device.queue_pcm(&[3, 0]).unwrap());
        device.finish_pcm();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while device.is_playing() {
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
            device.update().unwrap();
        }
        assert_eq!(
            fs::read(directory.path().join("player.pcm")).unwrap(),
            vec![1, 0, 2, 0, 3, 0]
        );
    }

    #[test]
    fn loads_named_wav_cues_and_rejects_path_like_names() {
        let directory = tempdir().unwrap();
        write_minimal_wav(&directory.path().join("acknowledge.wav"));
        fs::write(directory.path().join("notes.txt"), "ignored").unwrap();

        let cues = CueLibrary::load(directory.path()).unwrap();
        assert_eq!(cues.names(), vec!["acknowledge"]);
        assert!(
            cues.cue("acknowledge")
                .unwrap()
                .ends_with("acknowledge.wav")
        );
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
