use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use serde::{Deserialize, Serialize};

use crate::{AudioDevice, Error, Result};

pub const DEFAULT_TTS_SOCKET_PATH: &str = "/tmp/orion-tts.sock";
pub const DEFAULT_SPEECH_SPOOL_PATH: &str = "/tmp/orion-speech-spool";
pub const MAX_SPEECH_TEXT_BYTES: usize = 2_000;
pub const MAX_SPEECH_WAV_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_SPEECH_SECONDS: f64 = 120.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechPhase {
    Queued,
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

#[derive(Clone, Debug, PartialEq)]
pub struct SpeechAnalysis {
    pub rms_20ms: Vec<f64>,
    pub quiet_regions: Vec<(usize, usize)>,
    pub phrase_peaks: Vec<usize>,
    pub duration_seconds: f64,
}

enum SynthesisOutcome {
    Ready(PathBuf),
    Failed(String),
}

struct ActiveSpeech {
    status: SpeechStatus,
    result: Option<Receiver<SynthesisOutcome>>,
    wav_path: Option<PathBuf>,
    analysis: Option<SpeechAnalysis>,
    energy_frame: usize,
}

pub struct SpeechCoordinator {
    socket_path: PathBuf,
    spool_path: PathBuf,
    next_run_id: u64,
    active: Option<ActiveSpeech>,
    last: Option<SpeechStatus>,
}

impl SpeechCoordinator {
    pub fn new(socket_path: impl Into<PathBuf>) -> Result<Self> {
        Self::with_spool(socket_path, DEFAULT_SPEECH_SPOOL_PATH)
    }

    pub fn with_spool(
        socket_path: impl Into<PathBuf>,
        spool_path: impl Into<PathBuf>,
    ) -> Result<Self> {
        let socket_path = socket_path.into();
        let spool_path = spool_path.into();
        if socket_path.as_os_str().is_empty() {
            return Err(Error::InvalidArgument(
                "TTS worker socket path cannot be empty.".into(),
            ));
        }
        Ok(Self {
            socket_path,
            spool_path,
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
            result: Some(receiver),
            wav_path: None,
            analysis: None,
            energy_frame: 0,
        });
        Ok(status)
    }

    pub fn start_spooled(&mut self, identifier: &str) -> Result<SpeechStatus> {
        if self.active.is_some() {
            return Err(Error::InvalidState(
                "A speech run is already active.".into(),
            ));
        }
        if identifier.is_empty()
            || identifier.len() > 80
            || !identifier
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(Error::InvalidArgument(
                "Speech spool identifier is invalid.".into(),
            ));
        }
        let path = self.spool_path.join(format!("{identifier}.wav"));
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            Error::Runtime(format!("Speech spool item is unavailable: {error}"))
        })?;
        if !metadata.file_type().is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_SPEECH_WAV_BYTES
        {
            return Err(Error::InvalidArgument(
                "Speech spool item must be a regular WAV no larger than 8 MiB.".into(),
            ));
        }
        let analysis = analyze_pcm16_mono_wav(&path, Some(24_000))?;
        let run_id = self.next_run_id;
        self.next_run_id = self
            .next_run_id
            .checked_add(1)
            .ok_or_else(|| Error::Runtime("Speech run ID overflowed.".into()))?;
        let status = SpeechStatus {
            run_id,
            state: SpeechPhase::Queued,
            text: identifier.to_owned(),
            error: None,
        };
        self.active = Some(ActiveSpeech {
            status: status.clone(),
            result: None,
            wav_path: Some(path),
            analysis: Some(analysis),
            energy_frame: 0,
        });
        Ok(status)
    }

    pub fn tick<A: AudioDevice + ?Sized>(&mut self, audio: &mut A) {
        let Some(active) = self.active.as_mut() else {
            return;
        };

        if active.status.state == SpeechPhase::Queued {
            let path = active
                .wav_path
                .as_ref()
                .expect("queued speech has a spool path");
            let label = format!("speech-{}", active.status.run_id);
            match audio.play_file(&label, path) {
                Ok(()) => active.status.state = SpeechPhase::Playing,
                Err(error) => {
                    active.status.state = SpeechPhase::Failed;
                    active.status.error = Some(error.to_string());
                }
            }
        } else if active.status.state == SpeechPhase::Synthesizing {
            match active
                .result
                .as_ref()
                .expect("synthesizing speech has a worker receiver")
                .try_recv()
            {
                Ok(SynthesisOutcome::Ready(path)) => {
                    let label = format!("speech-{}", active.status.run_id);
                    match analyze_pcm16_mono_wav(&path, None)
                        .and_then(|analysis| audio.play_file(&label, &path).map(|()| analysis))
                    {
                        Ok(analysis) => {
                            active.analysis = Some(analysis);
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
                Ok(()) => active.energy_frame = active.energy_frame.saturating_add(1),
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

    pub fn active_analysis(&self) -> Option<&SpeechAnalysis> {
        self.active
            .as_ref()
            .and_then(|speech| speech.analysis.as_ref())
    }

    pub fn active_energy_frame(&self) -> Option<usize> {
        self.active
            .as_ref()
            .filter(|speech| speech.status.state == SpeechPhase::Playing)
            .map(|speech| speech.energy_frame)
    }

    pub fn active_energy(&self) -> Option<f64> {
        let speech = self.active.as_ref()?;
        let analysis = speech.analysis.as_ref()?;
        analysis
            .rms_20ms
            .get(speech.energy_frame)
            .copied()
            .or_else(|| analysis.rms_20ms.last().copied())
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

fn analyze_pcm16_mono_wav(
    path: &Path,
    required_sample_rate: Option<u32>,
) -> Result<SpeechAnalysis> {
    let bytes = fs::read(path)?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(Error::InvalidArgument(
            "Speech upload must be a RIFF/WAV file.".into(),
        ));
    }
    let mut cursor = 12usize;
    let mut format = None;
    let mut pcm = None;
    while cursor + 8 <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let size = u32::from_le_bytes(
            bytes[cursor + 4..cursor + 8]
                .try_into()
                .expect("four-byte chunk size"),
        ) as usize;
        let start = cursor + 8;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| {
                Error::InvalidArgument("Speech WAV contains a truncated chunk.".into())
            })?;
        if id == b"fmt " && size >= 16 {
            format = Some((
                u16::from_le_bytes(bytes[start..start + 2].try_into().unwrap()),
                u16::from_le_bytes(bytes[start + 2..start + 4].try_into().unwrap()),
                u32::from_le_bytes(bytes[start + 4..start + 8].try_into().unwrap()),
                u16::from_le_bytes(bytes[start + 14..start + 16].try_into().unwrap()),
            ));
        } else if id == b"data" {
            pcm = Some(&bytes[start..end]);
        }
        cursor = end + (size % 2);
    }
    let Some((encoding, channels, sample_rate, bits_per_sample)) = format else {
        return Err(Error::InvalidArgument(
            "Speech WAV must contain a PCM format chunk.".into(),
        ));
    };
    if encoding != 1
        || channels != 1
        || bits_per_sample != 16
        || sample_rate == 0
        || required_sample_rate.is_some_and(|required| sample_rate != required)
    {
        return Err(Error::InvalidArgument(
            if required_sample_rate.is_some() {
                "Speech WAV must be PCM16, mono, 24 kHz."
            } else {
                "Local speech WAV must be mono PCM16 with a valid sample rate."
            }
            .into(),
        ));
    }
    let pcm =
        pcm.ok_or_else(|| Error::InvalidArgument("Speech WAV contains no data chunk.".into()))?;
    if pcm.is_empty() || pcm.len() % 2 != 0 {
        return Err(Error::InvalidArgument(
            "Speech WAV PCM data is empty or misaligned.".into(),
        ));
    }
    let duration_seconds = pcm.len() as f64 / 2.0 / sample_rate as f64;
    if duration_seconds > MAX_SPEECH_SECONDS {
        return Err(Error::InvalidArgument(
            "Speech WAV cannot exceed 120 seconds.".into(),
        ));
    }
    let samples: Vec<f64> = pcm
        .chunks_exact(2)
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as f64 / 32768.0)
        .collect();
    let mut rms_20ms = Vec::new();
    let mut smoothed = 0.0;
    let frame_samples = (sample_rate as usize / 50).max(1);
    for frame in samples.chunks(frame_samples) {
        let rms =
            (frame.iter().map(|sample| sample * sample).sum::<f64>() / frame.len() as f64).sqrt();
        smoothed = 0.65 * smoothed + 0.35 * rms;
        rms_20ms.push(smoothed);
    }
    let maximum = rms_20ms.iter().copied().fold(0.0, f64::max);
    let quiet_threshold = (maximum * 0.12).max(0.004);
    let mut quiet_regions = Vec::new();
    let mut quiet_start = None;
    for (index, energy) in rms_20ms.iter().enumerate() {
        if *energy <= quiet_threshold {
            quiet_start.get_or_insert(index);
        } else if let Some(start) = quiet_start.take() {
            if index - start >= 3 {
                quiet_regions.push((start, index));
            }
        }
    }
    if let Some(start) = quiet_start {
        quiet_regions.push((start, rms_20ms.len()));
    }
    let mean = rms_20ms.iter().sum::<f64>() / rms_20ms.len().max(1) as f64;
    let mut phrase_peaks = Vec::new();
    for index in 1..rms_20ms.len().saturating_sub(1) {
        if rms_20ms[index] >= mean * 1.35
            && rms_20ms[index] >= rms_20ms[index - 1]
            && rms_20ms[index] > rms_20ms[index + 1]
            && phrase_peaks
                .last()
                .is_none_or(|previous| index - previous >= 10)
        {
            phrase_peaks.push(index);
        }
    }
    Ok(SpeechAnalysis {
        rms_20ms,
        quiet_regions,
        phrase_peaks,
        duration_seconds,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::time::Duration;

    use crate::{AudioCommand, RecordingAudioDevice, UnavailableAudioDevice};

    use super::*;

    fn write_pcm16_mono_24khz(path: &Path) {
        let mut bytes = Vec::from(&b"RIFF"[..]);
        bytes.extend_from_slice(&38_u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&24_000_u32.to_le_bytes());
        bytes.extend_from_slice(&48_000_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&0_i16.to_le_bytes());
        fs::write(path, bytes).unwrap();
    }

    fn write_energy_test_wav(path: &Path) {
        let samples: Vec<i16> = (0..24_000)
            .map(|index| match index {
                0..=4_799 | 12_000..=16_799 => 0,
                4_800..=11_999 => {
                    if index % 2 == 0 {
                        18_000
                    } else {
                        -18_000
                    }
                }
                _ => {
                    if index % 2 == 0 {
                        8_000
                    } else {
                        -8_000
                    }
                }
            })
            .collect();
        let data_bytes = (samples.len() * 2) as u32;
        let mut bytes = Vec::from(&b"RIFF"[..]);
        bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&24_000_u32.to_le_bytes());
        bytes.extend_from_slice(&48_000_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_bytes.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn synthesizes_plays_completes_and_removes_ephemeral_wav() {
        let directory = tempfile::tempdir().unwrap();
        let socket_path = directory.path().join("tts.sock");
        let wav_path = directory.path().join("generated.wav");
        write_pcm16_mono_24khz(&wav_path);
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
        assert!(speech.active_analysis().is_some());
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

    #[test]
    fn waveform_analysis_is_deterministic_and_finds_quiet_regions_and_peaks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("energy.wav");
        write_energy_test_wav(&path);

        let first = analyze_pcm16_mono_wav(&path, Some(24_000)).unwrap();
        let second = analyze_pcm16_mono_wav(&path, Some(24_000)).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.rms_20ms.len(), 50);
        assert!((first.duration_seconds - 1.0).abs() < 1e-9);
        assert!(!first.quiet_regions.is_empty());
        assert!(!first.phrase_peaks.is_empty());
    }

    #[test]
    fn spooled_wav_is_removed_after_completion_cancellation_and_playback_failure() {
        let directory = tempfile::tempdir().unwrap();
        let spool = directory.path();

        let completed_path = spool.join("completed.wav");
        write_pcm16_mono_24khz(&completed_path);
        let mut speech = SpeechCoordinator::with_spool("/tmp/not-used.sock", spool).unwrap();
        speech.start_spooled("completed").unwrap();
        let mut automatic_audio = RecordingAudioDevice::default();
        speech.tick(&mut automatic_audio);
        speech.tick(&mut automatic_audio);
        assert_eq!(speech.last_status().unwrap().state, SpeechPhase::Completed);
        assert!(!completed_path.exists());

        let cancelled_path = spool.join("cancelled.wav");
        write_pcm16_mono_24khz(&cancelled_path);
        speech.start_spooled("cancelled").unwrap();
        let mut blocking_audio = RecordingAudioDevice::blocking();
        speech.tick(&mut blocking_audio);
        assert_eq!(
            speech.cancel(&mut blocking_audio).unwrap().state,
            SpeechPhase::Cancelled
        );
        assert!(!cancelled_path.exists());

        let failed_path = spool.join("failed.wav");
        write_pcm16_mono_24khz(&failed_path);
        speech.start_spooled("failed").unwrap();
        speech.tick(&mut UnavailableAudioDevice);
        assert_eq!(speech.last_status().unwrap().state, SpeechPhase::Failed);
        assert!(!failed_path.exists());
    }
}
