use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use crate::{AudioDevice, Error, Result};

pub const DEFAULT_SPEECH_SPOOL_PATH: &str = "/tmp/orion-speech-spool";
pub const MAX_SPEECH_WAV_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_SPEECH_SECONDS: f64 = 120.0;
// Absolute-indexed animation analysis and unpaced external uploads retain state.
// Bound that state without constraining ordinary multi-minute spoken answers.
const MAX_STREAM_SAMPLES: u64 = 30 * 60 * 24_000;
const STREAM_LIMIT_ERROR: &str =
    "Speech stream exceeds the 30-minute audio sanity limit; playback cannot complete this answer.";

/// Canonical wire vocabulary; aliases are normalized by the agent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechCue {
    Agree,
    Disagree,
    Happy,
    Curious,
    Thinking,
    Surprised,
    Sympathy,
    Unsure,
    Laugh,
}
impl SpeechCue {
    pub const ALL: [Self; 9] = [
        Self::Agree,
        Self::Disagree,
        Self::Happy,
        Self::Curious,
        Self::Thinking,
        Self::Surprised,
        Self::Sympathy,
        Self::Unsure,
        Self::Laugh,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agree => "agree",
            Self::Disagree => "disagree",
            Self::Happy => "happy",
            Self::Curious => "curious",
            Self::Thinking => "thinking",
            Self::Surprised => "surprised",
            Self::Sympathy => "sympathy",
            Self::Unsure => "unsure",
            Self::Laugh => "laugh",
        }
    }
    pub fn motion(self) -> &'static str {
        match self {
            Self::Agree => "speak_react_agree",
            Self::Disagree => "speak_react_disagree",
            Self::Happy => "speak_react_happy",
            Self::Curious => "speak_react_curious",
            Self::Thinking => "speak_react_thinking",
            Self::Surprised => "speak_react_surprised",
            Self::Sympathy => "speak_react_sympathy",
            Self::Unsure => "speak_react_unsure",
            Self::Laugh => "speak_react_laugh",
        }
    }
}
impl std::str::FromStr for SpeechCue {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|cue| cue.as_str() == value)
            .ok_or_else(|| Error::InvalidArgument(format!("Unknown speech cue: {value}")))
    }
}
fn record_cues(
    analysis: &mut SpeechAnalysis,
    samples: u64,
    cues: Vec<SpeechCue>,
    run: u64,
    sequence: usize,
) {
    let frame = (samples / 480) as usize;
    for cue in cues {
        if analysis.cues.len() >= 32 {
            eprintln!(
                "{}",
                serde_json::json!({"event":"speech.cue_dropped", "run_id":run, "sequence":sequence, "cue":cue.as_str(), "reason":"limit"})
            );
            continue;
        }
        analysis.cues.push((frame, cue));
        eprintln!(
            "{}",
            serde_json::json!({"event":"speech.cue_received", "run_id":run, "sequence":sequence, "cue":cue.as_str(), "frame":frame})
        );
    }
}

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
    /// Received audio minus software playback elapsed; not an ALSA measurement.
    pub buffered_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpeechAnalysis {
    pub rms_20ms: Vec<f64>,
    pub quiet_regions: Vec<(usize, usize)>,
    pub phrase_peaks: Vec<usize>,
    pub cues: Vec<(usize, SpeechCue)>,
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
    analyzer: EnergyAnalyzer,
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
            buffered_ms: None,
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

    pub fn start_stream(&mut self, identifier: &str, cues: Vec<SpeechCue>) -> Result<SpeechStatus> {
        let status = self.start_spooled(identifier)?;
        let active = self.active.as_mut().unwrap();
        let pcm = decode_pcm16_mono_wav(&active.wav_path)?;
        if pcm.len() > 96_000 {
            self.finish_active();
            return Err(Error::InvalidArgument(
                "Stream chunks must be at most two seconds.".into(),
            ));
        }
        active.analysis = empty_analysis();
        let mut analyzer = EnergyAnalyzer::default();
        record_cues(
            &mut active.analysis,
            analyzer.samples,
            cues,
            status.run_id,
            0,
        );
        analyzer.append(&pcm, &mut active.analysis);
        active.analysis.streaming = true;
        active.stream = Some(StreamSpeech {
            pending: pcm.chunks(24_000).map(Vec::from).collect(),
            analyzer,
            next_sequence: 1,
            finished: false,
            updated: Instant::now(),
        });
        let _ = fs::remove_file(&active.wav_path);
        Ok(status)
    }

    pub fn append_stream(
        &mut self,
        run_id: u64,
        sequence: usize,
        identifier: &str,
        cues: Vec<SpeechCue>,
    ) -> Result<()> {
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
        if stream.finished || sequence != stream.next_sequence || pcm.len() > 96_000 {
            return Err(Error::InvalidArgument(
                "Invalid, out-of-order or oversized speech stream.".into(),
            ));
        }
        if stream.analyzer.samples + (pcm.len() / 2) as u64 > MAX_STREAM_SAMPLES {
            return Err(Error::InvalidArgument(STREAM_LIMIT_ERROR.into()));
        }
        record_cues(
            &mut active.analysis,
            stream.analyzer.samples,
            cues,
            run_id,
            sequence,
        );
        stream.analyzer.append(&pcm, &mut active.analysis);
        stream.pending.extend(pcm.chunks(24_000).map(Vec::from));
        stream.next_sequence += 1;
        stream.updated = Instant::now();
        active.analysis.streaming = true;
        eprintln!(
            "{}",
            serde_json::json!({
                "event": "speech.chunk_received", "run_id": run_id, "sequence": sequence,
                "audio_ms": pcm.len() / 48,
                "buffered_ms": ((active.analysis.duration_seconds
                    - active.playing_at.map(|at| at.elapsed().as_secs_f64()).unwrap_or(0.0)) * 1000.0) as i64,
            })
        );
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
        eprintln!(
            "{}",
            serde_json::json!({
                "event": "speech.stream_end", "run_id": run_id, "sequence": sequence,
                "audio_ms": (active.analysis.duration_seconds * 1000.0) as u64,
                "playback_started": active.playing_at.is_some(),
            })
        );
        Ok(())
    }

    pub fn tick<A: AudioDevice + ?Sized>(&mut self, audio: &mut A) {
        self.tick_when_ready(audio, true);
    }

    /// Continue upload timeout checks while home/attention owns movement, but
    /// keep accepted audio queued until the runtime permits playback.
    pub fn tick_when_ready<A: AudioDevice + ?Sized>(&mut self, audio: &mut A, ready: bool) {
        let Some(active) = self.active.as_mut() else {
            return;
        };

        active.status.elapsed_ms = active.created.elapsed().as_millis() as u64;
        if let Some(stream) = active.stream.as_ref() {
            active.status.buffered_ms = Some(
                ((active.analysis.duration_seconds
                    - active
                        .playing_at
                        .map(|at| at.elapsed().as_secs_f64())
                        .unwrap_or(0.0))
                    * 1000.0) as i64,
            );
            let underrun = active.playing_at.is_some_and(|at| {
                !stream.finished
                    && at.elapsed().as_secs_f64() > active.analysis.duration_seconds + 0.12
            });
            if underrun || (!stream.finished && stream.updated.elapsed().as_secs_f64() > 10.0) {
                let _ = audio.stop();
                active.status.state = SpeechPhase::Failed;
                active.status.error = Some(if underrun {
                    "Speech playback exhausted its buffer before more audio arrived.".into()
                } else {
                    "Speech upload stalled: no audio or end marker arrived for 10 seconds.".into()
                });
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "speech.failed", "run_id": active.status.run_id,
                        "reason": if underrun { "buffer_exhausted" } else { "upload_timeout" },
                        "buffered_ms": active.status.buffered_ms,
                        "received_ms": (active.analysis.duration_seconds * 1000.0) as u64,
                        "last_upload_age_ms": stream.updated.elapsed().as_millis() as u64,
                        "elapsed_ms": active.status.elapsed_ms,
                    })
                );
            }
        }
        if active.status.state == SpeechPhase::Queued {
            if !ready {
                return;
            }
            if active
                .stream
                .as_ref()
                .is_some_and(|s| !s.finished && s.analyzer.samples < 48_000)
            {
                return;
            }
            let path = &active.wav_path;
            let label = format!("speech-{}", active.status.run_id);

            let start = (if audio.is_playing() {
                audio.stop()
            } else {
                Ok(())
            })
            .and_then(|_| {
                if active.stream.is_some() {
                    audio.start_pcm(&label)
                } else {
                    audio.play_file(&label, path)
                }
            });
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
        eprintln!(
            "{}",
            serde_json::json!({
                "event": "speech.terminal", "run_id": active.status.run_id,
                "state": active.status.state, "elapsed_ms": active.status.elapsed_ms,
            })
        );
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

fn empty_analysis() -> SpeechAnalysis {
    SpeechAnalysis {
        rms_20ms: Vec::new(),
        quiet_regions: Vec::new(),
        phrase_peaks: Vec::new(),
        cues: Vec::new(),
        duration_seconds: 0.0,
        streaming: false,
    }
}

#[derive(Default)]
struct EnergyAnalyzer {
    samples: u64,
    frame_samples: usize,
    frame_sum: f64,
    smoothed: f64,
}
impl EnergyAnalyzer {
    fn append(&mut self, pcm: &[u8], analysis: &mut SpeechAnalysis) {
        // A partial final frame is provisional. Replace it on the next append
        // so WAV chunk boundaries do not change the 20 ms energy timeline.
        if self.frame_samples != 0 {
            analysis.rms_20ms.pop();
        }
        for sample in pcm.as_chunks::<2>().0 {
            let value = i16::from_le_bytes([sample[0], sample[1]]) as f64 / 32768.0;
            self.samples += 1;
            self.frame_samples += 1;
            self.frame_sum += value * value;
            if self.frame_samples == 480 {
                self.smoothed = 0.65 * self.smoothed + 0.35 * (self.frame_sum / 480.0).sqrt();
                analysis.rms_20ms.push(self.smoothed);
                self.frame_sum = 0.0;
                self.frame_samples = 0;
            }
        }
        if self.frame_samples != 0 {
            analysis.rms_20ms.push(
                0.65 * self.smoothed + 0.35 * (self.frame_sum / self.frame_samples as f64).sqrt(),
            );
        }
        analysis.duration_seconds = self.samples as f64 / 24_000.0;
        // Thresholds depend on the whole energy envelope, not raw samples.
        // Rebuild compact landmarks to preserve the character's existing contract.
        refresh_landmarks(analysis);
    }
}

fn analyze_pcm(pcm: &[u8]) -> Result<SpeechAnalysis> {
    let mut analysis = empty_analysis();
    EnergyAnalyzer::default().append(pcm, &mut analysis);
    Ok(analysis)
}

fn refresh_landmarks(analysis: &mut SpeechAnalysis) {
    let rms_20ms = &analysis.rms_20ms;
    let maximum = rms_20ms.iter().copied().fold(0.0, f64::max);
    let quiet_threshold = (maximum * 0.12).max(0.004);
    let mut quiet_regions = Vec::new();
    let mut quiet_start = None;
    for (index, energy) in rms_20ms.iter().enumerate() {
        if *energy <= quiet_threshold {
            quiet_start.get_or_insert(index);
        } else if let Some(start) = quiet_start.take()
            && index - start >= 3
        {
            quiet_regions.push((start, index));
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
    analysis.quiet_regions = quiet_regions;
    analysis.phrase_peaks = phrase_peaks;
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{RecordingAudioDevice, UnavailableAudioDevice};

    use super::*;

    #[test]
    fn cue_vocabulary_and_chunk_frames_are_canonical_and_bounded() {
        assert_eq!(
            SpeechCue::ALL.map(SpeechCue::as_str),
            [
                "agree",
                "disagree",
                "happy",
                "curious",
                "thinking",
                "surprised",
                "sympathy",
                "unsure",
                "laugh"
            ]
        );
        assert_eq!(
            SpeechCue::ALL.map(SpeechCue::motion),
            [
                "speak_react_agree",
                "speak_react_disagree",
                "speak_react_happy",
                "speak_react_curious",
                "speak_react_thinking",
                "speak_react_surprised",
                "speak_react_sympathy",
                "speak_react_unsure",
                "speak_react_laugh"
            ]
        );
        assert!("nod".parse::<SpeechCue>().is_err());
        let directory = tempfile::tempdir().unwrap();
        write_energy_test_wav(&directory.path().join("first.wav"));
        write_energy_test_wav(&directory.path().join("next.wav"));
        let mut speech = SpeechCoordinator::new(directory.path());
        let run = speech
            .start_stream("first", vec![SpeechCue::Agree])
            .unwrap()
            .run_id;
        assert_eq!(
            speech.active_analysis().unwrap().cues,
            [(0, SpeechCue::Agree)]
        );
        assert!(
            speech
                .append_stream(run, 2, "next", vec![SpeechCue::Disagree])
                .is_err()
        );
        assert_eq!(speech.active_analysis().unwrap().cues.len(), 1);
        speech
            .append_stream(run, 1, "next", vec![SpeechCue::Disagree])
            .unwrap();
        assert_eq!(
            speech.active_analysis().unwrap().cues,
            [(0, SpeechCue::Agree), (50, SpeechCue::Disagree)]
        );
        let mut analysis = empty_analysis();
        record_cues(&mut analysis, 479, vec![SpeechCue::Agree], run, 0);
        record_cues(&mut analysis, 480, vec![SpeechCue::Disagree; 40], run, 1);
        assert_eq!(analysis.cues.len(), 32);
        assert_eq!(analysis.cues[0].0, 0);
        assert_eq!(analysis.cues[31].0, 1);
    }
    #[test]
    fn a_stream_accepts_30_minutes_and_rejects_more_with_a_clear_reason() {
        let directory = tempfile::tempdir().unwrap();
        write_energy_test_wav(&directory.path().join("first.wav"));
        write_energy_test_wav(&directory.path().join("next.wav"));
        let mut speech = SpeechCoordinator::new(directory.path());
        let run = speech.start_stream("first", Vec::new()).unwrap().run_id;
        speech
            .active
            .as_mut()
            .unwrap()
            .stream
            .as_mut()
            .unwrap()
            .analyzer
            .samples = MAX_STREAM_SAMPLES - 24_000;
        speech.append_stream(run, 1, "next", Vec::new()).unwrap();
        assert_eq!(speech.active_analysis().unwrap().duration_seconds, 1800.);
        write_energy_test_wav(&directory.path().join("extra.wav"));
        let error = speech
            .append_stream(run, 2, "extra", Vec::new())
            .unwrap_err()
            .to_string();
        assert!(error.contains(STREAM_LIMIT_ERROR));
        assert_eq!(speech.active_analysis().unwrap().duration_seconds, 1800.);
        speech.end_stream(run, 2).unwrap();
    }

    #[test]
    fn incremental_analysis_matches_whole_pcm_across_partial_frames() {
        let pcm: Vec<u8> = (0..27_173i16).flat_map(i16::to_le_bytes).collect();
        let expected = analyze_pcm(&pcm).unwrap();
        let mut actual = empty_analysis();
        let mut analyzer = EnergyAnalyzer::default();
        for chunk in pcm.chunks(734) {
            analyzer.append(chunk, &mut actual);
        }
        assert_eq!(actual, expected);
        assert_eq!(actual.rms_20ms.len(), 57);
    }

    #[test]
    fn accepted_pcm_is_released_while_analysis_keeps_absolute_frames() {
        let directory = tempfile::tempdir().unwrap();
        write_energy_test_wav(&directory.path().join("first.wav"));
        let mut speech = SpeechCoordinator::new(directory.path());
        let run = speech.start_stream("first", Vec::new()).unwrap().run_id;
        write_energy_test_wav(&directory.path().join("second.wav"));
        speech.append_stream(run, 1, "second", Vec::new()).unwrap();
        let mut audio = RecordingAudioDevice::blocking();
        speech.tick(&mut audio);
        let active = speech.active.as_ref().unwrap();
        assert!(active.stream.as_ref().unwrap().pending.is_empty());
        assert_eq!(active.analysis.rms_20ms.len(), 100);
        assert_eq!(active.analysis.duration_seconds, 2.);
    }

    #[test]
    fn streaming_uses_one_player_and_requires_ordered_end() {
        let directory = tempfile::tempdir().unwrap();
        write_energy_test_wav(&directory.path().join("first.wav"));
        write_energy_test_wav(&directory.path().join("second.wav"));
        let mut speech = SpeechCoordinator::new(directory.path());
        let mut audio = RecordingAudioDevice::blocking();
        let run = speech.start_stream("first", Vec::new()).unwrap().run_id;
        speech.tick(&mut audio);
        assert!(
            audio.commands().is_empty(),
            "must prebuffer before starting"
        );
        assert!(
            speech.active_energy().is_none(),
            "buffering must not drive speaking light"
        );
        assert!(speech.append_stream(run, 2, "second", Vec::new()).is_err());
        speech.append_stream(run, 1, "second", Vec::new()).unwrap();
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
    fn speech_preempts_a_processing_cue_only_when_ready_to_play() {
        let directory = tempfile::tempdir().unwrap();
        write_energy_test_wav(&directory.path().join("first.wav"));
        let mut speech = SpeechCoordinator::new(directory.path());
        let mut audio = RecordingAudioDevice::blocking();
        audio.play("voice_processing").unwrap();
        let run = speech.start_stream("first", Vec::new()).unwrap().run_id;
        speech.tick(&mut audio);
        assert_eq!(
            audio.commands().len(),
            1,
            "queued speech leaves the cue alone"
        );
        speech.end_stream(run, 1).unwrap();
        speech.tick(&mut audio);
        assert_eq!(speech.active_status().unwrap().state, SpeechPhase::Playing);
        assert!(matches!(
            audio.commands()[1],
            crate::audio::AudioCommand::Stop
        ));
        assert_eq!(audio.commands().len(), 3);
    }

    #[test]
    fn underrun_and_upload_timeout_have_distinct_diagnostics() {
        use std::time::Duration;
        let directory = tempfile::tempdir().unwrap();
        write_energy_test_wav(&directory.path().join("first.wav"));
        write_energy_test_wav(&directory.path().join("second.wav"));
        let mut speech = SpeechCoordinator::new(directory.path());
        let mut audio = RecordingAudioDevice::blocking();
        let run = speech.start_stream("first", Vec::new()).unwrap().run_id;
        speech.append_stream(run, 1, "second", Vec::new()).unwrap();
        speech.tick(&mut audio);
        speech.active.as_mut().unwrap().playing_at =
            Some(Instant::now() - Duration::from_millis(2200));
        speech.tick(&mut audio);
        let failed = speech.last_status().unwrap();
        assert_eq!(failed.state, SpeechPhase::Failed);
        assert!(
            failed
                .error
                .as_ref()
                .unwrap()
                .contains("exhausted its buffer")
        );
        assert!(failed.buffered_ms.unwrap() <= -200);
        assert!(!audio.is_playing());

        write_energy_test_wav(&directory.path().join("third.wav"));
        speech.start_stream("third", Vec::new()).unwrap();
        speech
            .active
            .as_mut()
            .unwrap()
            .stream
            .as_mut()
            .unwrap()
            .updated = Instant::now() - Duration::from_secs(11);
        speech.tick(&mut audio);
        let failed = speech.last_status().unwrap();
        assert_eq!(failed.state, SpeechPhase::Failed);
        assert!(failed.error.as_ref().unwrap().contains("upload stalled"));
        assert_eq!(failed.first_playback_ms, None);
    }

    #[test]
    fn stream_cancel_rejects_late_chunks_and_short_end_can_play() {
        let directory = tempfile::tempdir().unwrap();
        write_energy_test_wav(&directory.path().join("first.wav"));
        write_energy_test_wav(&directory.path().join("late.wav"));
        let mut speech = SpeechCoordinator::new(directory.path());
        let mut audio = RecordingAudioDevice::blocking();
        let run = speech.start_stream("first", Vec::new()).unwrap().run_id;
        speech.cancel(&mut audio).unwrap();
        assert!(speech.append_stream(run, 1, "late", Vec::new()).is_err());
        let next = speech.start_stream("late", Vec::new()).unwrap().run_id;
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
