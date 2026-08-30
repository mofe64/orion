use crate::{Error, Result};

pub trait AudioDevice {
    fn play(&mut self, cue: &str) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
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

#[derive(Debug, Default)]
pub struct RecordingAudioDevice {
    commands: Vec<AudioCommand>,
}

impl RecordingAudioDevice {
    pub fn commands(&self) -> &[AudioCommand] {
        &self.commands
    }
}

impl AudioDevice for RecordingAudioDevice {
    fn play(&mut self, cue: &str) -> Result<()> {
        if cue.trim().is_empty() {
            return Err(Error::InvalidArgument(
                "Audio cue name cannot be empty.".into(),
            ));
        }
        self.commands.push(AudioCommand::Play(cue.to_owned()));
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.commands.push(AudioCommand::Stop);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_audio_commands_in_order() {
        let mut device = RecordingAudioDevice::default();
        device.play("acknowledge").unwrap();
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
