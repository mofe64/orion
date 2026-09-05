use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use crate::{AudioDevice, Error, Result};

pub const DEFAULT_SPEECH_SPOOL_PATH: &str = "/tmp/orion-speech-spool";
pub const MAX_SPEECH_WAV_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_SPEECH_SECONDS: f64 = 120.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechPhase {
    Queued,
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
    pub first_playback_ms: Option<u64>,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpeechAnalysis {
    pub rms_20ms: Vec<f64>,
    pub quiet_regions: Vec<(usize, usize)>,
    pub phrase_peaks: Vec<usize>,
    pub duration_seconds: f64,
    pub streaming: bool,
}

struct ActiveSpeech {
    status: SpeechStatus,
    wav_path: PathBuf,
    analysis: SpeechAnalysis,
    energy_frame: usize,
    stream: Option<StreamSpeech>,
    created: Instant,
    playing_at: Option<Instant>,
}

struct StreamSpeech {
    pcm: Vec<u8>,
    pending: VecDeque<Vec<u8>>,
    next_sequence: usize,
    finished: bool,
    updated: Instant,
}

pub struct SpeechCoordinator {
    spool_path: PathBuf,
    next_run_id: u64,
    active: Option<ActiveSpeech>,
    last: Option<SpeechStatus>,
}

impl SpeechCoordinator {
    pub fn new(spool_path: impl Into<PathBuf>) -> Self {
        Self {
            spool_path: spool_path.into(),
            next_run_id: 1,
            active: None,
            last: None,
        }
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
        let analysis = analyze_pcm16_mono_wav(&path)?;
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
            first_playback_ms: None,
            elapsed_ms: 0,
        };
        self.active = Some(ActiveSpeech {
            status: status.clone(),
            wav_path: path,
            analysis,
            energy_frame: 0,
            stream: None,
            created: Instant::now(),
            playing_at: None,
        });
        Ok(status)
    }

    pub fn start_stream(&mut self, identifier: &str) -> Result<SpeechStatus> {
        let status = self.start_spooled(identifier)?;
        let active = self.active.as_mut().unwrap();
        let pcm = decode_pcm16_mono_wav(&active.wav_path)?;
        if pcm.len() > 96_000 {
            self.finish_active();
            return Err(Error::InvalidArgument(
                "Stream chunks must be at most two seconds.".into(),
            ));
        }
        active.analysis.streaming = true;
        active.stream = Some(StreamSpeech {
            pending: pcm.chunks(24_000).map(Vec::from).collect(),
            pcm,
            next_sequence: 1,
            finished: false,
            updated: Instant::now(),
        });
        let _ = fs::remove_file(&active.wav_path);
        Ok(status)
    }

    pub fn append_stream(&mut self, run_id: u64, sequence: usize, identifier: &str) -> Result<()> {
        if identifier.is_empty()
            || identifier.len() > 80
            || !identifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(Error::InvalidArgument(
                "Invalid speech chunk identifier.".into(),
            ));
        }
        let path = self.spool_path.join(format!("{identifier}.wav"));
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_file() || metadata.len() > 100_000 {
            return Err(Error::InvalidArgument("Invalid speech chunk file.".into()));
        }
        let pcm = decode_pcm16_mono_wav(&path)?;
        let active = self
            .active
            .as_mut()
            .filter(|a| a.status.run_id == run_id)
            .ok_or_else(|| Error::InvalidState("Stale speech run.".into()))?;
        let stream = active
            .stream
            .as_mut()
            .ok_or_else(|| Error::InvalidState("Not a speech stream.".into()))?;
        if stream.finished
            || sequence != stream.next_sequence
            || pcm.len() > 96_000
            || stream.pcm.len() + pcm.len() > 120 * 48_000
        {
            return Err(Error::InvalidArgument(
                "Invalid, out-of-order or oversized speech stream.".into(),
            ));
        }
        stream.pcm.extend_from_slice(&pcm);
        stream.pending.extend(pcm.chunks(24_000).map(Vec::from));
        stream.next_sequence += 1;
        stream.updated = Instant::now();
        active.analysis = analyze_pcm(&stream.pcm)?;
        active.analysis.streaming = true;
        let _ = fs::remove_file(path);
        Ok(())
    }

    pub fn end_stream(&mut self, run_id: u64, sequence: usize) -> Result<()> {
        let active = self
            .active
            .as_mut()
            .filter(|a| a.status.run_id == run_id)
            .ok_or_else(|| Error::InvalidState("Stale speech run.".into()))?;
        let stream = active
            .stream
            .as_mut()
            .ok_or_else(|| Error::InvalidState("Not a speech stream.".into()))?;
        if stream.finished || sequence != stream.next_sequence {
            return Err(Error::InvalidArgument("Invalid speech stream end.".into()));
        }
        stream.finished = true;
        active.analysis.streaming = false;
        Ok(())
    }

    pub fn tick<A: AudioDevice + ?Sized>(&mut self, audio: &mut A) {
        let Some(active) = self.active.as_mut() else {
            return;
        };

        active.status.elapsed_ms = active.created.elapsed().as_millis() as u64;
        if let Some(stream) = active.stream.as_ref() {
            let underrun = active.playing_at.is_some_and(|at| {
                !stream.finished
                    && at.elapsed().as_secs_f64() > active.analysis.duration_seconds + 0.12
            });
            if underrun || (!stream.finished && stream.updated.elapsed().as_secs_f64() > 10.0) {
                let _ = audio.stop();
                active.status.state = SpeechPhase::Failed;
                active.status.error =
                    Some("Speech stream stalled or playback exhausted its buffer.".into());
            }
        }
        if active.status.state == SpeechPhase::Queued {
            if active
                .stream
                .as_ref()
                .is_some_and(|s| !s.finished && s.pcm.len() < 96_000)
            {
                return;
            }
            let path = &active.wav_path;
            let label = format!("speech-{}", active.status.run_id);
            let start = if active.stream.is_some() {
                audio.start_pcm(&label)
            } else {
                audio.play_file(&label, path)
            };
            match start {
                Ok(()) => {
                    active.status.state = SpeechPhase::Playing;
                    active.playing_at = Some(Instant::now());
                    active.status.first_playback_ms = Some(active.status.elapsed_ms);
                }
                Err(error) => {
                    active.status.state = SpeechPhase::Failed;
                    active.status.error = Some(error.to_string());
                }
            }
        }
        if active.status.state == SpeechPhase::Playing {
            if let Some(stream) = active.stream.as_mut() {
                while let Some(pcm) = stream.pending.front() {
                    match audio.queue_pcm(pcm) {
                        Ok(true) => {
                            stream.pending.pop_front();
                        }
                        Ok(false) => break,
                        Err(error) => {
                            active.status.state = SpeechPhase::Failed;
                            active.status.error = Some(error.to_string());
                            let _ = audio.stop();
                            break;
                        }
                    }
                }
                if stream.finished && stream.pending.is_empty() {
                    audio.finish_pcm();
                }
            }
            match audio.update() {
                Ok(()) if !audio.is_playing() && active.status.state == SpeechPhase::Playing => {
                    if active
                        .stream
                        .as_ref()
                        .is_some_and(|s| !s.finished || !s.pending.is_empty())
                    {
                        active.status.state = SpeechPhase::Failed;
                        active.status.error =
                            Some("Speech stream ended before all audio arrived.".into());
                    } else {
                        active.status.state = SpeechPhase::Completed;
                    }
                }
                Ok(()) => {
                    active.energy_frame = (active
                        .playing_at
                        .map(|at| at.elapsed().as_secs_f64())
                        .unwrap_or(0.0)
                        / 0.020) as usize
                }
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
        self.active.as_ref().map(|speech| &speech.analysis)
    }

    pub fn active_energy_frame(&self) -> Option<usize> {
        self.active
            .as_ref()
            .filter(|speech| speech.status.state == SpeechPhase::Playing)
            .map(|speech| speech.energy_frame)
    }

    pub fn active_energy(&self) -> Option<f64> {
        let speech = self
            .active
            .as_ref()
            .filter(|speech| speech.status.state == SpeechPhase::Playing)?;
        let analysis = &speech.analysis;
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
        let _ = fs::remove_file(active.wav_path);
        self.last = Some(active.status);
    }
}

fn decode_pcm16_mono_wav(path: &Path) -> Result<Vec<u8>> {
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
    if encoding != 1 || channels != 1 || bits_per_sample != 16 || sample_rate != 24_000 {
        return Err(Error::InvalidArgument(
            "Speech WAV must be PCM16, mono, 24 kHz.".into(),
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
    Ok(pcm.to_vec())
}

fn analyze_pcm16_mono_wav(path: &Path) -> Result<SpeechAnalysis> {
    analyze_pcm(&decode_pcm16_mono_wav(path)?)
}

fn analyze_pcm(pcm: &[u8]) -> Result<SpeechAnalysis> {
    let sample_rate = 24_000;
    let duration_seconds = pcm.len() as f64 / 48_000.0;
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
        streaming: false,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{RecordingAudioDevice, UnavailableAudioDevice};

    use super::*;

    #[test]
    fn streaming_uses_one_player_and_requires_ordered_end() {
        let directory = tempfile::tempdir().unwrap();
        write_energy_test_wav(&directory.path().join("first.wav"));
        write_energy_test_wav(&directory.path().join("second.wav"));
        let mut speech = SpeechCoordinator::new(directory.path());
        let mut audio = RecordingAudioDevice::blocking();
        let run = speech.start_stream("first").unwrap().run_id;
        speech.tick(&mut audio);
        assert!(
            audio.commands().is_empty(),
            "must prebuffer before starting"
        );
        assert!(
            speech.active_energy().is_none(),
            "buffering must not drive speaking light"
        );
        assert!(speech.append_stream(run, 2, "second").is_err());
        speech.append_stream(run, 1, "second").unwrap();
        speech.tick(&mut audio);
        assert_eq!(speech.active_status().unwrap().state, SpeechPhase::Playing);
        assert_eq!(audio.commands().len(), 1);
        assert!(speech.end_stream(run, 1).is_err());
        speech.end_stream(run, 2).unwrap();
        assert!(speech.end_stream(run, 2).is_err());
        speech.tick(&mut audio);
        assert!(speech.is_active(), "end of upload is not end of playback");
        audio.finish();
        speech.tick(&mut audio);
        assert_eq!(speech.last_status().unwrap().state, SpeechPhase::Completed);
        assert_eq!(speech.last_status().unwrap().run_id, run);
        assert!(speech.last_status().unwrap().first_playback_ms.is_some());
        assert!(directory.path().read_dir().unwrap().next().is_none());
    }

    #[test]
    fn stream_cancel_rejects_late_chunks_and_short_end_can_play() {
        let directory = tempfile::tempdir().unwrap();
        write_energy_test_wav(&directory.path().join("first.wav"));
        write_energy_test_wav(&directory.path().join("late.wav"));
        let mut speech = SpeechCoordinator::new(directory.path());
        let mut audio = RecordingAudioDevice::blocking();
        let run = speech.start_stream("first").unwrap().run_id;
        speech.cancel(&mut audio).unwrap();
        assert!(speech.append_stream(run, 1, "late").is_err());
        let next = speech.start_stream("late").unwrap().run_id;
        speech.end_stream(next, 1).unwrap();
        speech.tick(&mut audio);
        assert_eq!(speech.active_status().unwrap().state, SpeechPhase::Playing);
        speech.cancel(&mut audio).unwrap();
        assert!(!audio.is_playing());
    }

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
    fn waveform_analysis_is_deterministic_and_finds_quiet_regions_and_peaks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("energy.wav");
        write_energy_test_wav(&path);

        let first = analyze_pcm16_mono_wav(&path).unwrap();
        let second = analyze_pcm16_mono_wav(&path).unwrap();
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
        let mut speech = SpeechCoordinator::new(spool);
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
