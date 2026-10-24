use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{OrionRuntimeError as Error, Result};

mod mixer;
mod volume;
pub use volume::{VolumeControl, VolumeStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioSource {
    Cue,
    Speech,
    Alarm,
    Music,
}

impl AudioSource {
    pub fn gain(self, volume: u8) -> f64 {
        let level = if self == Self::Alarm {
            volume.max(25)
        } else {
            volume
        };
        f64::from(level) / 100.0
    }
}

pub const ORION_AUDIO_CARD: &str = "seeed2micvoicec";
pub const ORION_AUDIO_PCM_DEVICE: &str = "plughw:CARD=seeed2micvoicec,DEV=0";
pub const ORION_AMIXER_PATH: &str = "/usr/bin/amixer";
pub const ORION_APLAY_PATH: &str = "/usr/bin/aplay";

pub trait AudioDevice {
    fn music_pulse(&self) -> Option<crate::expression::music::MusicPulse> {
        None
    }
    fn set_music_ducking(&mut self, _gain: f64) {}
    fn music_playing(&self) -> bool {
        false
    }
    fn play(&mut self, cue: &str) -> Result<()>;
    fn play_file(&mut self, label: &str, path: &Path) -> Result<()>;
    fn start_pcm(&mut self, _label: &str) -> Result<()> {
        Err(Error::InvalidState(
            "Streaming audio is unavailable.".into(),
        ))
    }
    fn start_alarm_pcm(&mut self) -> Result<()> {
        self.start_pcm("orion_alert")
    }
    fn volume_status(&self) -> VolumeStatus {
        VolumeStatus {
            volume: 100,
            muted: false,
            epoch: "unavailable".into(),
            revision: 0,
            save_error: None,
        }
    }
    fn set_volume(&mut self, _level: u8) -> Result<()> {
        Err(Error::InvalidState("Volume control is unavailable.".into()))
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

#[derive(Clone, Copy)]
struct PcmFormat {
    rate: u32,
    channels: u16,
    bits: u16,
    float: bool,
}

impl PcmFormat {
    fn speech() -> Self {
        Self {
            rate: 24_000,
            channels: 1,
            bits: 16,
            float: false,
        }
    }
    fn frame_bytes(self) -> usize {
        usize::from(self.channels) * usize::from(self.bits / 8)
    }
    fn from_wav(bytes: &[u8]) -> std::io::Result<Self> {
        if bytes.len() < 16 {
            return Err(invalid_audio("Incomplete WAV format"));
        }
        let mut tag = u16::from_le_bytes(bytes[0..2].try_into().unwrap());
        if tag == 0xfffe && bytes.len() >= 40 {
            tag = u16::from_le_bytes(bytes[24..26].try_into().unwrap());
        }
        let format = Self {
            channels: u16::from_le_bytes(bytes[2..4].try_into().unwrap()),
            rate: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            bits: u16::from_le_bytes(bytes[14..16].try_into().unwrap()),
            float: tag == 3,
        };
        if format.channels == 0
            || format.rate == 0
            || !matches!((tag, format.bits), (1, 8 | 16 | 24 | 32) | (3, 32 | 64))
        {
            return Err(invalid_audio("Unsupported WAV sample format"));
        }
        Ok(format)
    }
}

fn invalid_audio(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

// Each source reads the same master level. Only alarms have a volume floor.
// Gain and file I/O run on the playback worker, outside the motor loop.
struct PcmPlayback {
    format: PcmFormat,
    volume: Arc<AtomicU8>,
    source: AudioSource,
    stop: Arc<AtomicBool>,
    gain: f64,
    deadline: Instant,
}

impl PcmPlayback {
    fn new(
        format: PcmFormat,
        volume: Arc<AtomicU8>,
        source: AudioSource,
        stop: Arc<AtomicBool>,
    ) -> Self {
        let gain = source.gain(volume.load(Ordering::Relaxed));
        Self {
            format,
            volume,
            source,
            stop,
            gain,
            deadline: Instant::now(),
        }
    }
    fn write(&mut self, output: &mut impl Write, pcm: &[u8]) -> std::io::Result<()> {
        let frame = self.format.frame_bytes();
        if pcm.len() % frame != 0 {
            return Err(invalid_audio("Incomplete PCM frame"));
        }
        let chunk_bytes = (self.format.rate as usize / 100).max(1) * frame;
        for chunk in pcm.chunks(chunk_bytes) {
            while self.deadline.saturating_duration_since(Instant::now())
                > Duration::from_millis(40)
            {
                if self.stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                thread::park_timeout(
                    self.deadline
                        .saturating_duration_since(Instant::now())
                        .saturating_sub(Duration::from_millis(40)),
                );
            }
            if self.stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let target = self.source.gain(self.volume.load(Ordering::Relaxed));
            let mut scaled = chunk.to_vec();
            scale_pcm(&mut scaled, self.format, self.gain, target);
            output.write_all(&scaled)?;
            self.gain = target;
            self.deadline = self.deadline.max(Instant::now())
                + Duration::from_secs_f64(
                    chunk.len() as f64 / frame as f64 / f64::from(self.format.rate),
                );
        }
        Ok(())
    }
}

fn scale_pcm(pcm: &mut [u8], format: PcmFormat, from: f64, to: f64) {
    let frames = pcm.len() / format.frame_bytes();
    let width = usize::from(format.bits / 8);
    for (index, frame) in pcm.chunks_exact_mut(format.frame_bytes()).enumerate() {
        let gain = from + (to - from) * (index + 1) as f64 / frames as f64;
        if gain == 1.0 {
            continue;
        }
        for sample in frame.chunks_exact_mut(width) {
            match (format.float, format.bits) {
                (true, 32) => {
                    let value = f32::from_le_bytes(sample.try_into().unwrap());
                    sample.copy_from_slice(&(value * gain as f32).to_le_bytes());
                }
                (true, 64) => {
                    let value = f64::from_le_bytes(sample.try_into().unwrap());
                    sample.copy_from_slice(&(value * gain).to_le_bytes());
                }
                (false, 8) => {
                    sample[0] = ((f64::from(sample[0]) - 128.) * gain + 128.)
                        .round()
                        .clamp(0., 255.) as u8
                }
                (false, _) => {
                    let mut bytes = if sample[width - 1] & 0x80 != 0 {
                        [0xff; 4]
                    } else {
                        [0; 4]
                    };
                    bytes[..width].copy_from_slice(sample);
                    let value = (f64::from(i32::from_le_bytes(bytes)) * gain).round() as i32;
                    sample.copy_from_slice(&value.to_le_bytes()[..width]);
                }
                _ => unreachable!("validated PCM format"),
            }
        }
    }
}

fn stream_wav(
    path: &Path,
    output: &mut impl Write,
    volume: Arc<AtomicU8>,
    source: AudioSource,
    stop: Arc<AtomicBool>,
) -> std::io::Result<()> {
    let mut file = fs::File::open(path)?;
    let mut header = [0; 12];
    file.read_exact(&mut header)?;
    output.write_all(&header)?;
    let mut format = None;
    loop {
        let mut header = [0; 8];
        match file.read(&mut header[..1])? {
            0 => return Ok(()),
            _ => file.read_exact(&mut header[1..])?,
        }
        let size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        output.write_all(&header)?;
        if &header[..4] == b"data" {
            let format = format.ok_or_else(|| invalid_audio("WAV data precedes its format"))?;
            let mut playback =
                PcmPlayback::new(format, Arc::clone(&volume), source, Arc::clone(&stop));
            let mut remaining = size;
            let mut chunk = vec![0; (format.rate as usize / 100).max(1) * format.frame_bytes()];
            while remaining > 0 && !stop.load(Ordering::Relaxed) {
                let length = remaining.min(chunk.len());
                file.read_exact(&mut chunk[..length])?;
                playback.write(output, &chunk[..length])?;
                remaining -= length;
            }
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
        } else if &header[..4] == b"fmt " {
            if size > 4096 {
                return Err(invalid_audio("Invalid WAV format size"));
            }
            let mut bytes = vec![0; size];
            file.read_exact(&mut bytes)?;
            format = Some(PcmFormat::from_wav(&bytes)?);
            output.write_all(&bytes)?;
        } else {
            std::io::copy(&mut (&mut file).take(size as u64), output)?;
        }
        if size % 2 != 0 {
            let mut pad = [0];
            file.read_exact(&mut pad)?;
            output.write_all(&pad)?;
        }
    }
}

pub fn configure_respeaker_v2_mixer(card_name: &str) -> Result<()> {
    let settings: &[&[&str]] = &[
        &["sset", "PCM", "0dB"],
        &["sset", "Right DAC Mux", "DAC_R1"],
        &["sset", "Right Line Mixer DACR1", "on"],
        &["sset", "Line DAC", "0dB"],
        &["sset", "Line", "0dB", "unmute"],
    ];
    apply_mixer_settings(card_name, "ReSpeaker", settings)
}

/// The XVF3800's USB playback controls default to -20 dB (40 of 60) on V2,
/// which leaves Orion's speaker barely audible. Both PCM controls (the
/// stereo stream and its mono mirror) are set to unity gain and unmuted.
pub const XVF3800_MIXER_SETTINGS: &[&[&str]] = &[
    &["sset", "PCM,0", "0dB", "unmute"],
    &["sset", "PCM,1", "0dB", "unmute"],
];

pub fn configure_xvf3800_mixer(card_name: &str) -> Result<()> {
    apply_mixer_settings(card_name, "XVF3800", XVF3800_MIXER_SETTINGS)
}

fn apply_mixer_settings(card_name: &str, device: &str, settings: &[&[&str]]) -> Result<()> {
    if card_name.trim().is_empty() {
        return Err(Error::InvalidArgument(
            "ALSA audio card name cannot be empty.".into(),
        ));
    }
    for setting in settings {
        let output = Command::new(ORION_AMIXER_PATH)
            .args(["-q", "-c", card_name, "--"])
            .args(*setting)
            .output()
            .map_err(|error| {
                Error::Runtime(format!(
                    "Could not run {device} mixer command '{}': {error}",
                    ORION_AMIXER_PATH
                ))
            })?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(Error::Runtime(format!(
                "{device} mixer configuration failed for '{}': {}",
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
    writer_stop: Option<Arc<AtomicBool>>,
    volume: VolumeControl,
    mixer: Option<mixer::Mixer>,
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
            writer_stop: None,
            volume: VolumeControl::default(),
            mixer: None,
        })
    }

    pub fn active_label(&self) -> Option<&str> {
        self.active_label.as_deref()
    }

    pub fn with_music_socket(mut self, path: PathBuf) -> Result<Self> {
        self.mixer = Some(mixer::Mixer::open(
            &self.player_path,
            &self.pcm_device,
            path,
            Arc::clone(&self.volume.level),
        )?);
        Ok(self)
    }

    pub fn with_volume_path(mut self, path: Option<PathBuf>) -> Result<Self> {
        if let Some(path) = path {
            self.volume = VolumeControl::persistent(path)?;
        }
        Ok(self)
    }

    fn start_playback(&mut self, label: &str, path: &Path, source: AudioSource) -> Result<()> {
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
        if let Some(mixer) = &mut self.mixer {
            mixer.file(path.to_owned(), source)?;
            self.active_label = Some(label.into());
            return Ok(());
        }
        let mut child = Command::new(&self.player_path)
            .args([
                "-q",
                "-D",
                &self.pcm_device,
                "--buffer-time=40000",
                "--period-time=10000",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                Error::Runtime(format!(
                    "Could not start audio source '{label}' with '{}': {error}",
                    self.player_path.display()
                ))
            })?;
        let mut stdin = child.stdin.take().expect("piped WAV input");
        let path = path.to_owned();
        let volume = Arc::clone(&self.volume.level);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        self.pcm_writer = Some(thread::spawn(move || {
            stream_wav(&path, &mut stdin, volume, source, worker_stop)
        }));
        self.writer_stop = Some(stop);
        self.child = Some(child);
        self.active_label = Some(label.to_owned());
        Ok(())
    }

    fn start_stream(&mut self, label: &str, source: AudioSource) -> Result<()> {
        self.update()?;
        if let Some(mixer) = &mut self.mixer {
            mixer.stream(source)?;
            self.active_label = Some(label.into());
            return Ok(());
        }
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
                "--buffer-time=40000",
                "--period-time=10000",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("piped audio input");
        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(4);
        let volume = Arc::clone(&self.volume.level);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        self.pcm_writer = Some(thread::spawn(move || {
            let mut playback = PcmPlayback::new(PcmFormat::speech(), volume, source, worker_stop);
            loop {
                match receiver.recv_timeout(Duration::from_secs(3)) {
                    Ok(pcm) => playback.write(&mut stdin, &pcm)?,
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
        self.writer_stop = Some(stop);
        self.pcm_sender = Some(sender);
        self.child = Some(child);
        self.active_label = Some(label.into());
        Ok(())
    }
}

impl AudioDevice for AlsaAudioDevice {
    fn music_pulse(&self) -> Option<crate::expression::music::MusicPulse> {
        self.mixer.as_ref().and_then(|mixer| mixer.pulse())
    }
    fn set_music_ducking(&mut self, gain: f64) {
        if let Some(mixer) = &self.mixer {
            mixer.duck_to(gain);
        }
    }
    fn music_playing(&self) -> bool {
        self.mixer
            .as_ref()
            .is_some_and(|mixer| mixer.music_playing())
    }
    fn play(&mut self, cue: &str) -> Result<()> {
        let path = self.cues.cue(cue)?.to_owned();
        self.start_playback(cue, &path, AudioSource::Cue)
    }

    fn play_file(&mut self, label: &str, path: &Path) -> Result<()> {
        self.start_playback(label, path, AudioSource::Speech)
    }

    fn start_pcm(&mut self, label: &str) -> Result<()> {
        self.start_stream(label, AudioSource::Speech)
    }

    fn start_alarm_pcm(&mut self) -> Result<()> {
        self.start_stream("orion_alert", AudioSource::Alarm)
    }
    fn volume_status(&self) -> VolumeStatus {
        self.volume.status()
    }
    fn set_volume(&mut self, level: u8) -> Result<()> {
        self.volume.set(level)
    }

    fn queue_pcm(&mut self, pcm: &[u8]) -> Result<bool> {
        if let Some(mixer) = &self.mixer {
            return mixer.queue(pcm);
        }
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
        if let Some(mixer) = &mut self.mixer {
            mixer.finish();
        }
        self.pcm_sender = None;
    }

    fn update(&mut self) -> Result<()> {
        if let Some(mixer) = &mut self.mixer {
            mixer.update()?;
            if !mixer.playing() {
                self.active_label = None;
            }
            return Ok(());
        }
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        let Some(status) = child.try_wait()? else {
            return Ok(());
        };
        self.pcm_sender = None;
        self.writer_stop.take();
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
        self.mixer
            .as_ref()
            .map_or(self.child.is_some(), |mixer| mixer.playing())
    }

    fn stop(&mut self) -> Result<()> {
        if let Some(mixer) = &mut self.mixer {
            self.active_label = None;
            return mixer.stop();
        }
        self.pcm_sender = None;
        if let Some(stop) = self.writer_stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some(writer) = &self.pcm_writer {
            writer.thread().unpark();
        }
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
    volume: VolumeControl,
    music_playing: bool,
}

impl Default for RecordingAudioDevice {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            playing: false,
            auto_complete: true,
            volume: VolumeControl::default(),
            music_playing: false,
        }
    }
}

impl RecordingAudioDevice {
    #[cfg(test)]
    pub(crate) fn set_test_music_playing(&mut self, playing: bool) {
        self.music_playing = playing;
    }
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
    fn music_playing(&self) -> bool {
        self.music_playing
    }
    fn volume_status(&self) -> VolumeStatus {
        self.volume.status()
    }
    fn set_volume(&mut self, level: u8) -> Result<()> {
        self.volume.set(level)
    }
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

    #[test]
    fn master_gain_scales_each_source_and_only_alarms_have_a_floor() {
        for (volume, source, expected) in [
            (40, AudioSource::Speech, 4000),
            (40, AudioSource::Cue, 4000),
            (40, AudioSource::Music, 4000),
            (0, AudioSource::Speech, 0),
            (0, AudioSource::Music, 0),
            (0, AudioSource::Alarm, 2500),
            (80, AudioSource::Alarm, 8000),
        ] {
            let mut samples = [10_000_i16.to_le_bytes(), (-10_000_i16).to_le_bytes()].concat();
            let gain = source.gain(volume);
            scale_pcm(&mut samples, PcmFormat::speech(), gain, gain);
            assert_eq!(
                i16::from_le_bytes(samples[..2].try_into().unwrap()),
                expected
            );
            assert_eq!(
                i16::from_le_bytes(samples[2..].try_into().unwrap()),
                -expected
            );
        }
    }

    #[test]
    fn volume_changes_reach_an_open_stream_with_a_short_gain_ramp() {
        struct Sink {
            bytes: Vec<u8>,
            volume: Arc<AtomicU8>,
        }
        impl Write for Sink {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.bytes.extend_from_slice(bytes);
                self.volume.store(0, Ordering::Relaxed);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let volume = Arc::new(AtomicU8::new(100));
        let mut sink = Sink {
            bytes: vec![],
            volume: Arc::clone(&volume),
        };
        let mut playback = PcmPlayback::new(
            PcmFormat::speech(),
            volume,
            AudioSource::Speech,
            Arc::new(AtomicBool::new(false)),
        );
        playback
            .write(&mut sink, &10_000_i16.to_le_bytes().repeat(720))
            .unwrap();
        let samples: Vec<i16> = sink
            .bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert!(samples[..240].iter().all(|value| *value == 10_000));
        assert!(samples[240..480].windows(2).all(|pair| pair[0] >= pair[1]));
        assert!(samples[480..].iter().all(|value| *value == 0));
    }

    #[test]
    fn wav_playback_scales_data_and_preserves_the_format_and_chunk_headers() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("speech.wav");
        let mut wav = b"RIFF".to_vec();
        wav.extend(40_u32.to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16_u32.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(24_000_u32.to_le_bytes());
        wav.extend(48_000_u32.to_le_bytes());
        wav.extend(2_u16.to_le_bytes());
        wav.extend(16_u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(4_u32.to_le_bytes());
        wav.extend(10_000_i16.to_le_bytes());
        wav.extend((-10_000_i16).to_le_bytes());
        fs::write(&path, &wav).unwrap();
        let mut output = vec![];
        stream_wav(
            &path,
            &mut output,
            Arc::new(AtomicU8::new(40)),
            AudioSource::Speech,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(&output[..44], &wav[..44]);
        assert_eq!(
            &output[44..],
            &[4000_i16.to_le_bytes(), (-4000_i16).to_le_bytes()].concat()
        );
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
