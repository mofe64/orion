use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use serde::{Deserialize, Serialize};

use crate::{AudioDevice, Error, Result};

pub const DEFAULT_TTS_SOCKET_PATH: &str = "/tmp/orion-tts.sock";
pub const MAX_SPEECH_TEXT_BYTES: usize = 2_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechPhase {
    Synthesizing,
    Playing,
    Completed,
    Failed,
    Cancelled,
}

impl SpeechPhase {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SpeechStatus {
    pub run_id: u64,
    pub state: SpeechPhase,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

enum SynthesisOutcome {
    Ready(PathBuf),
    Failed(String),
}

struct ActiveSpeech {
    status: SpeechStatus,
    result: Receiver<SynthesisOutcome>,
    wav_path: Option<PathBuf>,
}

pub struct SpeechCoordinator {
    socket_path: PathBuf,
    next_run_id: u64,
    active: Option<ActiveSpeech>,
    last: Option<SpeechStatus>,
}

impl SpeechCoordinator {
    pub fn new(socket_path: impl Into<PathBuf>) -> Result<Self> {
        let socket_path = socket_path.into();
        if socket_path.as_os_str().is_empty() {
            return Err(Error::InvalidArgument(
                "TTS worker socket path cannot be empty.".into(),
            ));
        }
        Ok(Self {
            socket_path,
            next_run_id: 1,
            active: None,
            last: None,
        })
    }

    pub fn start(&mut self, text: &str) -> Result<SpeechStatus> {
        if let Some(active) = &self.active {
            return Err(Error::InvalidState(format!(
                "Speech run {} is already active.",
                active.status.run_id
            )));
        }
        let text = validate_speech_text(text)?;
        let run_id = self.next_run_id;
        self.next_run_id = self
            .next_run_id
            .checked_add(1)
            .ok_or_else(|| Error::Runtime("Speech run ID overflowed.".into()))?;

        let socket_path = self.socket_path.clone();
        let worker_text = text.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let outcome = match request_synthesis(&socket_path, run_id, &worker_text) {
                Ok(path) => SynthesisOutcome::Ready(path),
                Err(error) => SynthesisOutcome::Failed(error.to_string()),
            };
            if let Err(error) = sender.send(outcome) {
                if let SynthesisOutcome::Ready(path) = error.0 {
                    let _ = fs::remove_file(path);
                }
            }
        });

        let status = SpeechStatus {
            run_id,
            state: SpeechPhase::Synthesizing,
            text,
            error: None,
        };
        self.active = Some(ActiveSpeech {
            status: status.clone(),
            result: receiver,
            wav_path: None,
        });
        Ok(status)
    }

    pub fn tick<A: AudioDevice + ?Sized>(&mut self, audio: &mut A) {
        let Some(active) = self.active.as_mut() else {
            return;
        };

        if active.status.state == SpeechPhase::Synthesizing {
            match active.result.try_recv() {
                Ok(SynthesisOutcome::Ready(path)) => {
                    let label = format!("speech-{}", active.status.run_id);
                    match audio.play_file(&label, &path) {
                        Ok(()) => {
                            active.wav_path = Some(path);
                            active.status.state = SpeechPhase::Playing;
                        }
                        Err(error) => {
                            let _ = fs::remove_file(path);
                            active.status.state = SpeechPhase::Failed;
                            active.status.error = Some(error.to_string());
                        }
                    }
                }
                Ok(SynthesisOutcome::Failed(error)) => {
                    active.status.state = SpeechPhase::Failed;
                    active.status.error = Some(error);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    active.status.state = SpeechPhase::Failed;
                    active.status.error = Some("TTS worker request ended without a result.".into());
                }
            }
        } else if active.status.state == SpeechPhase::Playing {
            match audio.update() {
                Ok(()) if !audio.is_playing() => {
                    active.status.state = SpeechPhase::Completed;
                }
                Ok(()) => {}
                Err(error) => {
                    active.status.state = SpeechPhase::Failed;
                    active.status.error = Some(error.to_string());
                }
            }
        }

        if self
            .active
            .as_ref()
            .is_some_and(|speech| speech.status.state.is_terminal())
        {
            self.finish_active();
        }
    }

    pub fn cancel<A: AudioDevice + ?Sized>(&mut self, audio: &mut A) -> Result<SpeechStatus> {
        let Some(active) = self.active.as_mut() else {
            return Err(Error::InvalidState("No speech run is active.".into()));
        };
        if active.status.state == SpeechPhase::Playing {
            audio.stop()?;
        }
        active.status.state = SpeechPhase::Cancelled;
        self.finish_active();
        Ok(self
            .last
            .clone()
            .expect("cancelled speech status must exist"))
    }

    pub fn active_status(&self) -> Option<&SpeechStatus> {
        self.active.as_ref().map(|speech| &speech.status)
    }

    pub fn last_status(&self) -> Option<&SpeechStatus> {
        self.last.as_ref()
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    fn finish_active(&mut self) {
        let Some(active) = self.active.take() else {
            return;
        };
        if let Some(path) = active.wav_path {
            let _ = fs::remove_file(path);
        }
        self.last = Some(active.status);
    }
}

#[derive(Serialize)]
struct WorkerRequest<'a> {
    request_id: u64,
    text: &'a str,
}

#[derive(Deserialize)]
struct WorkerResponse {
    request_id: u64,
    state: String,
    wav_path: Option<PathBuf>,
    error: Option<String>,
}

fn request_synthesis(socket_path: &Path, request_id: u64, text: &str) -> Result<PathBuf> {
    let mut stream = UnixStream::connect(socket_path).map_err(|error| {
        Error::Runtime(format!(
            "Could not connect to TTS worker '{}': {error}",
            socket_path.display()
        ))
    })?;
    serde_json::to_writer(&mut stream, &WorkerRequest { request_id, text })?;
    stream.write_all(b"\n")?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let response: WorkerResponse = serde_json::from_str(response.trim())?;
    if response.request_id != request_id {
        return Err(Error::Runtime(format!(
            "TTS worker returned request {}, expected {request_id}.",
            response.request_id
        )));
    }
    match response.state.as_str() {
        "ready" => {
            let path = response.wav_path.ok_or_else(|| {
                Error::Runtime("TTS worker returned ready without a WAV path.".into())
            })?;
            if !path.is_absolute() {
                return Err(Error::Runtime(
                    "TTS worker returned a non-absolute WAV path.".into(),
                ));
            }
            Ok(path)
        }
        "failed" => Err(Error::Runtime(response.error.unwrap_or_else(|| {
            "TTS worker failed without an error message.".into()
        }))),
        state => Err(Error::Runtime(format!(
            "TTS worker returned unknown state '{state}'."
        ))),
    }
}

fn validate_speech_text(text: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::InvalidArgument(
            "Speech text cannot be empty.".into(),
        ));
    }
    if text.contains('\n') || text.contains('\r') {
        return Err(Error::InvalidArgument(
            "Speech text cannot contain line breaks.".into(),
        ));
    }
    if text.len() > MAX_SPEECH_TEXT_BYTES {
        return Err(Error::InvalidArgument(format!(
            "Speech text cannot exceed {MAX_SPEECH_TEXT_BYTES} UTF-8 bytes."
        )));
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::time::Duration;

    use crate::{AudioCommand, RecordingAudioDevice};

    use super::*;

    fn write_minimal_wav(path: &Path) {
        fs::write(path, b"RIFF\x04\x00\x00\x00WAVE").unwrap();
    }

    #[test]
    fn synthesizes_plays_completes_and_removes_ephemeral_wav() {
        let directory = tempfile::tempdir().unwrap();
        let socket_path = directory.path().join("tts.sock");
        let wav_path = directory.path().join("generated.wav");
        write_minimal_wav(&wav_path);
        let listener = UnixListener::bind(&socket_path).unwrap();
        let server_wav_path = wav_path.clone();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let request: serde_json::Value = serde_json::from_str(request.trim()).unwrap();
            serde_json::to_writer(
                &mut stream,
                &serde_json::json!({
                    "request_id": request["request_id"],
                    "state": "ready",
                    "wav_path": server_wav_path,
                    "error": null,
                }),
            )
            .unwrap();
        });

        let mut speech = SpeechCoordinator::new(&socket_path).unwrap();
        let accepted = speech.start("Hello from Orion.").unwrap();
        assert_eq!(accepted.run_id, 1);
        assert_eq!(accepted.state, SpeechPhase::Synthesizing);

        let mut audio = RecordingAudioDevice::blocking();
        for _ in 0..100 {
            speech.tick(&mut audio);
            if speech
                .active_status()
                .is_some_and(|status| status.state == SpeechPhase::Playing)
            {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(speech.active_status().unwrap().state, SpeechPhase::Playing);
        assert!(matches!(
            audio.commands().last(),
            Some(AudioCommand::PlayFile { label, path })
                if label == "speech-1" && path == &wav_path
        ));

        audio.finish();
        speech.tick(&mut audio);
        assert!(!speech.is_active());
        assert_eq!(speech.last_status().unwrap().state, SpeechPhase::Completed);
        assert!(!wav_path.exists());
        server.join().unwrap();
    }

    #[test]
    fn rejects_empty_multiline_and_oversized_text() {
        let mut speech = SpeechCoordinator::new("/tmp/not-used.sock").unwrap();
        assert!(speech.start("  ").is_err());
        assert!(speech.start("hello\nthere").is_err());
        assert!(
            speech
                .start(&"x".repeat(MAX_SPEECH_TEXT_BYTES + 1))
                .is_err()
        );
    }
}
