use crate::{Error, Result};

pub trait AudioDevice {
    fn play(&mut self, cue: &str) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
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
}
