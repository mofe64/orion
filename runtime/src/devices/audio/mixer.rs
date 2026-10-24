use super::{AudioSource, PcmFormat, invalid_audio};
use crate::expression::music::{BeatTracker, MusicPulse};
use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::{
    fs::PermissionsExt,
    net::{UnixListener, UnixStream},
};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    mpsc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

type Frame = [f32; 2];
const RATE: u32 = 48_000;
const FRAMES: usize = 480;

enum Input {
    Speech(mpsc::Receiver<Vec<u8>>),
    File(mpsc::Receiver<Vec<Frame>>),
}
enum Control {
    Start(u64, AudioSource, Input),
    Stop,
    Quit,
}

/// One supervised worker and one ALSA client own the speaker.
/// Foreground ownership excludes music: stopping a cue never stops the song.
pub(super) struct Mixer {
    sender: mpsc::Sender<Control>,
    active: Arc<AtomicU64>,
    next_id: u64,
    pcm: Option<mpsc::SyncSender<Vec<u8>>>,
    duck: Arc<AtomicU8>,
    music: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    worker: Option<JoinHandle<()>>,
    loaders: Vec<JoinHandle<()>>,
    player: Arc<Mutex<Option<Child>>>,
    stopping: Arc<AtomicBool>,
    pulse: Arc<Mutex<MusicPulse>>,
}

impl Mixer {
    pub fn open(
        player: &Path,
        device: &str,
        socket: PathBuf,
        volume: Arc<AtomicU8>,
    ) -> crate::Result<Self> {
        let mut listener = listen(&socket)?;
        let mut child = spawn_player(player, device)?;
        let mut output = PlayerOutput::new(child.stdin.take().expect("piped mixer output"))?;
        let player_path = player.to_owned();
        let device = device.to_owned();
        let player = Arc::new(Mutex::new(Some(child)));
        let worker_player = Arc::clone(&player);
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = Arc::clone(&stopping);
        let (sender, receiver) = mpsc::channel();
        let active = Arc::new(AtomicU64::new(0));
        let duck = Arc::new(AtomicU8::new(100));
        let music = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let pulse = Arc::new(Mutex::new(MusicPulse::default()));
        let shared = (
            Arc::clone(&active),
            Arc::clone(&duck),
            Arc::clone(&music),
            Arc::clone(&pulse),
        );
        let worker = thread::Builder::new()
            .name("orion-audio-mixer".into())
            .spawn(move || {
                // Keep the command queue and unread foreground samples across output failures.
                // In particular, an alarm queued during recovery must still reach the speaker.
                let mut foreground = None;
                loop {
                    let result = run(
                        &mut output,
                        listener,
                        &receiver,
                        &mut foreground,
                        &volume,
                        &shared.0,
                        &shared.1,
                        &shared.2,
                        &shared.3,
                    );
                    if result.is_ok() || worker_stopping.load(Ordering::Relaxed) {
                        break;
                    }
                    eprintln!(
                        "Audio output failed; reopening the speaker: {}",
                        result.unwrap_err()
                    );
                    shared.2.store(false, Ordering::Relaxed);
                    *shared.3.lock().unwrap() = MusicPulse::default();
                    drop(output);
                    let Some(restarted) = restart_output(
                        &player_path,
                        &device,
                        &socket,
                        &worker_player,
                        &worker_stopping,
                    ) else {
                        break;
                    };
                    (output, listener) = restarted;
                }
                shared.0.store(0, Ordering::Relaxed);
                let _ = fs::remove_file(socket);
            })?;
        Ok(Self {
            sender,
            active,
            next_id: 0,
            pcm: None,
            duck,
            music,
            error,
            worker: Some(worker),
            loaders: vec![],
            player,
            stopping,
            pulse,
        })
    }
    fn start(&mut self, source: AudioSource, input: Input) -> crate::Result<u64> {
        self.update()?;
        if self.playing() {
            return Err(crate::Error::InvalidState("Audio already playing.".into()));
        }
        self.next_id += 1;
        self.active.store(self.next_id, Ordering::Relaxed);
        if self
            .sender
            .send(Control::Start(self.next_id, source, input))
            .is_err()
        {
            self.active.store(0, Ordering::Relaxed);
            return Err(crate::Error::Runtime("Audio mixer stopped.".into()));
        }
        Ok(self.next_id)
    }
    pub fn file(&mut self, path: PathBuf, source: AudioSource) -> crate::Result<()> {
        let (sender, receiver) = mpsc::sync_channel(4);
        self.start(source, Input::File(receiver))?;
        let error = Arc::clone(&self.error);
        self.loaders
            .push(thread::spawn(move || match read_wav(&path) {
                Ok(frames) => {
                    for chunk in frames.chunks(FRAMES) {
                        if sender.send(chunk.to_vec()).is_err() {
                            break;
                        }
                    }
                }
                Err(reason) => {
                    *error.lock().unwrap() = Some(format!("Could not play audio: {reason}"))
                }
            }));
        Ok(())
    }
    pub fn stream(&mut self, source: AudioSource) -> crate::Result<()> {
        let (sender, receiver) = mpsc::sync_channel(4);
        self.start(source, Input::Speech(receiver))?;
        self.pcm = Some(sender);
        Ok(())
    }
    pub fn queue(&self, bytes: &[u8]) -> crate::Result<bool> {
        let sender = self
            .pcm
            .as_ref()
            .ok_or_else(|| crate::Error::InvalidState("No open audio stream.".into()))?;
        match sender.try_send(bytes.to_vec()) {
            Ok(()) => Ok(true),
            Err(mpsc::TrySendError::Full(_)) => Ok(false),
            Err(_) => Err(crate::Error::Runtime("Audio stream stopped.".into())),
        }
    }
    pub fn finish(&mut self) {
        self.pcm = None;
    }
    pub fn stop(&mut self) -> crate::Result<()> {
        self.pcm = None;
        self.active.store(0, Ordering::Relaxed);
        self.sender
            .send(Control::Stop)
            .map_err(|_| crate::Error::Runtime("Audio mixer stopped.".into()))
    }
    pub fn playing(&self) -> bool {
        self.active.load(Ordering::Relaxed) != 0
    }
    pub fn music_playing(&self) -> bool {
        self.music.load(Ordering::Relaxed)
    }
    pub fn pulse(&self) -> Option<MusicPulse> {
        self.pulse.try_lock().ok().map(|pulse| *pulse)
    }
    #[cfg(test)]
    pub fn duck(&self, value: bool) {
        self.duck_to(if value { 0.25 } else { 1. });
    }
    pub fn duck_to(&self, gain: f64) {
        self.duck
            .store((gain.clamp(0., 1.) * 100.).round() as u8, Ordering::Relaxed);
    }
    pub fn update(&mut self) -> crate::Result<()> {
        self.loaders.retain(|worker| !worker.is_finished());
        if let Some(reason) = self
            .error
            .try_lock()
            .ok()
            .and_then(|mut error| error.take())
        {
            return Err(crate::Error::Runtime(reason));
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            return Err(crate::Error::Runtime("Audio mixer stopped.".into()));
        }
        Ok(())
    }
}
impl Drop for Mixer {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        let _ = self.sender.send(Control::Quit);
        if let Some(mut player) = self.player.lock().unwrap().take() {
            let _ = player.kill();
            let _ = player.wait();
        }
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
        for loader in self.loaders.drain(..) {
            let _ = loader.join();
        }
    }
}

fn listen(socket: &Path) -> std::io::Result<UnixListener> {
    if let Some(parent) = socket.parent() {
        fs::create_dir_all(parent)?;
    }
    if socket.exists() {
        if UnixStream::connect(socket).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "The music receiver is already running.",
            ));
        }
        fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn spawn_player(player: &Path, device: &str) -> std::io::Result<Child> {
    Command::new(player)
        .args([
            "-q",
            "-D",
            device,
            "-t",
            "raw",
            "-f",
            "S16_LE",
            "-r",
            "48000",
            "-c",
            "2",
            "--buffer-time=40000",
            "--period-time=10000",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
}

// A child can remain alive while USB/ALSA stops consuming its pipe. A
// nonblocking pipe keeps both recovery and shutdown bounded in that case.
struct PlayerOutput(ChildStdin);
impl PlayerOutput {
    fn new(pipe: ChildStdin) -> std::io::Result<Self> {
        let fd = pipe.as_raw_fd();
        // SAFETY: fd belongs to the live pipe; fcntl does not retain it.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(Self(pipe))
    }
}
impl Write for PlayerOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let deadline = Instant::now() + Duration::from_millis(250);
        let mut written = 0;
        while written < bytes.len() {
            match self.0.write(&bytes[written..]) {
                Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
                Ok(count) => written += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(std::io::ErrorKind::TimedOut.into());
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn restart_output(
    path: &Path,
    device: &str,
    socket: &Path,
    player: &Mutex<Option<Child>>,
    stopping: &AtomicBool,
) -> Option<(PlayerOutput, UnixListener)> {
    if let Some(mut child) = player.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    loop {
        thread::park_timeout(Duration::from_millis(100));
        // Serialize child replacement with Drop, which kills a blocked output writer.
        let mut current = player.lock().unwrap();
        if stopping.load(Ordering::Relaxed) {
            return None;
        }
        match listen(socket)
            .and_then(|listener| spawn_player(path, device).map(|child| (child, listener)))
        {
            Ok((mut child, listener)) => {
                let output =
                    match PlayerOutput::new(child.stdin.take().expect("piped mixer output")) {
                        Ok(output) => output,
                        Err(error) => {
                            let _ = child.kill();
                            let _ = child.wait();
                            eprintln!("Audio pipe setup failed: {error}");
                            continue;
                        }
                    };
                *current = Some(child);
                return Some((output, listener));
            }
            Err(error) => eprintln!("Could not reopen audio output; retrying: {error}"),
        }
    }
}

struct Foreground {
    id: u64,
    source: AudioSource,
    input: Input,
    frames: VecDeque<Frame>,
    ended: bool,
}
impl Foreground {
    fn fill(&mut self) -> std::io::Result<()> {
        while self.frames.len() < FRAMES && !self.ended {
            let result = match &self.input {
                Input::Speech(receiver) => receiver.try_recv().map(|bytes| {
                    if bytes.len() % 2 != 0 {
                        return Err(invalid_audio("Incomplete speech sample"));
                    }
                    Ok(bytes
                        .chunks_exact(2)
                        .flat_map(|sample| {
                            let value =
                                f32::from(i16::from_le_bytes(sample.try_into().unwrap())) / 32768.;
                            [[value; 2]; 2]
                        })
                        .collect::<Vec<_>>())
                }),
                Input::File(receiver) => receiver.try_recv().map(Ok),
            };
            match result {
                Ok(frames) => self.frames.extend(frames?),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => self.ended = true,
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct Resampler {
    input: VecDeque<Frame>,
    phase: u32,
}
impl Resampler {
    fn take(&mut self, rate: u32) -> Option<Frame> {
        if self.input.len() < 2 {
            return None;
        }
        let a = self.input[0];
        let b = self.input[1];
        let f = self.phase as f32 / RATE as f32;
        let frame = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
        self.phase += rate;
        while self.phase >= RATE && self.input.len() > 1 {
            self.input.pop_front();
            self.phase -= RATE;
        }
        Some(frame)
    }
}

fn decode(bytes: &[u8], format: PcmFormat) -> std::io::Result<Vec<Frame>> {
    if bytes.len() % format.frame_bytes() != 0 {
        return Err(invalid_audio("Incomplete PCM frame"));
    }
    let width = usize::from(format.bits / 8);
    Ok(bytes
        .chunks_exact(format.frame_bytes())
        .map(|frame| {
            let channel = |index: usize| {
                let sample = &frame[index * width..(index + 1) * width];
                match (format.float, format.bits) {
                    (true, 32) => f32::from_le_bytes(sample.try_into().unwrap()),
                    (true, 64) => f64::from_le_bytes(sample.try_into().unwrap()) as f32,
                    (false, 8) => (f32::from(sample[0]) - 128.) / 128.,
                    _ => {
                        let mut value = if sample[width - 1] & 0x80 != 0 {
                            [0xff; 4]
                        } else {
                            [0; 4]
                        };
                        value[..width].copy_from_slice(sample);
                        i32::from_le_bytes(value) as f32 / (1_u64 << (format.bits - 1)) as f32
                    }
                }
            };
            [channel(0), channel(if format.channels > 1 { 1 } else { 0 })]
        })
        .collect())
}

fn read_wav(path: &Path) -> std::io::Result<Vec<Frame>> {
    let bytes = fs::read(path)?;
    let mut offset = 12;
    let mut format = None;
    let mut decoded = vec![];
    while offset + 8 <= bytes.len() {
        let name = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        offset += 8;
        let chunk = bytes
            .get(offset..offset + size)
            .ok_or_else(|| invalid_audio("Incomplete WAV chunk"))?;
        if name == b"fmt " {
            format = Some(PcmFormat::from_wav(chunk)?);
        }
        if name == b"data" {
            decoded.extend(decode(
                chunk,
                format.ok_or_else(|| invalid_audio("Missing WAV format"))?,
            )?);
        }
        offset += size + size % 2;
    }
    let format = format.ok_or_else(|| invalid_audio("Missing WAV format"))?;
    if format.rate == RATE {
        return Ok(decoded);
    }
    let length = (decoded.len() as u64 * u64::from(RATE) / u64::from(format.rate)) as usize;
    if let Some(last) = decoded.last().copied() {
        decoded.push(last);
    }
    let mut resampler = Resampler {
        input: decoded.into(),
        phase: 0,
    };
    Ok((0..length)
        .filter_map(|_| resampler.take(format.rate))
        .collect())
}

fn duck_gain(gain: f64, target: f64) -> f64 {
    let seconds = if target < gain { 0.04 } else { 0.4 };
    target + (gain - target) * (-1. / (f64::from(RATE) * seconds)).exp()
}

fn run(
    output: &mut impl Write,
    listener: UnixListener,
    controls: &mpsc::Receiver<Control>,
    foreground: &mut Option<Foreground>,
    volume: &AtomicU8,
    active: &AtomicU64,
    duck: &AtomicU8,
    music: &AtomicBool,
    pulse: &Mutex<MusicPulse>,
) -> std::io::Result<()> {
    let mut tracker = BeatTracker::default();
    let mut connection: Option<UnixStream> = None;
    let mut resampler = Resampler::default();
    let mut pending = vec![];
    let mut master = f64::from(volume.load(Ordering::Relaxed)) / 100.;
    let mut alarm = master.max(0.25);
    let mut ducking = 1.;
    let mut deadline = Instant::now();
    loop {
        for command in controls.try_iter() {
            match command {
                Control::Quit => return Ok(()),
                Control::Stop => *foreground = None,
                Control::Start(id, source, input) => {
                    *foreground = Some(Foreground {
                        id,
                        source,
                        input,
                        frames: VecDeque::new(),
                        ended: false,
                    })
                }
            }
        }
        if let Some(source) = foreground {
            source.fill()?;
            if source.ended && source.frames.is_empty() {
                let _ = active.compare_exchange(source.id, 0, Ordering::Relaxed, Ordering::Relaxed);
                *foreground = None;
            }
        }
        if connection.is_none() {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(true)?;
                    pending.clear();
                    connection = Some(stream);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error),
            }
        }
        if let Some(stream) = &mut connection {
            while resampler.input.len() < 443 {
                let mut buffer = [0; 4096];
                match stream.read(&mut buffer) {
                    Ok(0) => {
                        pending.clear();
                        connection = None;
                        break;
                    }
                    Ok(length) => {
                        pending.extend_from_slice(&buffer[..length]);
                        let count = pending.len() / 4 * 4;
                        resampler.input.extend(decode(
                            &pending[..count],
                            PcmFormat {
                                rate: 44_100,
                                channels: 2,
                                bits: 16,
                                float: false,
                            },
                        )?);
                        pending.drain(..count);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        pending.clear();
                        connection = None;
                        break;
                    }
                }
            }
        }
        let target = f64::from(volume.load(Ordering::Relaxed)) / 100.;
        let alarm_target = target.max(0.25);
        let duck_target = (f64::from(duck.load(Ordering::Relaxed)) / 100.)
            .min(if foreground.is_some() { 0.25 } else { 1. });
        let streaming = resampler.input.len() >= 2;
        let song: [Frame; FRAMES] =
            std::array::from_fn(|_| resampler.take(44_100).unwrap_or([0.; 2]));
        let mut analysis = tracker.process(&song);
        analysis.streaming = streaming;
        if target == 0. {
            analysis.audible = false;
            analysis.energy = 0.;
        }
        if let Ok(mut current) = pulse.try_lock() {
            *current = analysis;
        }
        let mut audible_music = false;
        let mut bytes = [0_u8; FRAMES * 4];
        for index in 0..FRAMES {
            let progress = (index + 1) as f64 / FRAMES as f64;
            let master_gain = master + (target - master) * progress;
            let alarm_gain = alarm + (alarm_target - alarm) * progress;
            ducking = duck_gain(ducking, duck_target);
            let song = song[index];
            audible_music |= song.iter().any(|sample| sample.abs() > 0.0001);
            let voice = foreground
                .as_ref()
                .and_then(|source| source.frames.get(index).copied())
                .unwrap_or([0.; 2]);
            let voice_gain = if foreground
                .as_ref()
                .is_some_and(|source| source.source == AudioSource::Alarm)
            {
                alarm_gain
            } else {
                master_gain
            };
            for channel in 0..2 {
                let value = (f64::from(song[channel]) * master_gain * ducking
                    + f64::from(voice[channel]) * voice_gain)
                    .clamp(-1., 1.);
                bytes[index * 4 + channel * 2..index * 4 + channel * 2 + 2]
                    .copy_from_slice(&((value * 32767.).round() as i16).to_le_bytes());
            }
        }
        master = target;
        alarm = alarm_target;
        music.store(audible_music, Ordering::Relaxed);
        output.write_all(&bytes)?;
        if let Some(source) = foreground {
            source.frames.drain(..source.frames.len().min(FRAMES));
        }
        deadline += Duration::from_millis(10);
        if let Some(delay) = deadline.checked_duration_since(Instant::now()) {
            thread::sleep(delay);
        } else {
            deadline = Instant::now();
        }
        if connection.is_none() && resampler.input.len() < 2 {
            resampler = Resampler::default();
            pending.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconnect_discards_partial_stereo_frames_at_every_byte_offset() {
        let directory = tempfile::tempdir().unwrap();
        let player = directory.path().join("player");
        let pcm = directory.path().join("player.pcm");
        fs::write(&player, "#!/bin/sh\nexec cat > \"$0.pcm\"\n").unwrap();
        fs::set_permissions(&player, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.path().join("music.sock");
        let mixer = Mixer::open(
            &player,
            "test",
            socket.clone(),
            Arc::new(AtomicU8::new(100)),
        )
        .unwrap();
        let wait = |condition: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !condition() {
                assert!(Instant::now() < deadline, "music did not reach the speaker");
                thread::sleep(Duration::from_millis(10));
            }
        };
        wait(&|| pcm.is_file());
        for offset in 0..4 {
            let before = fs::metadata(&pcm).unwrap().len() as usize;
            let mut old = UnixStream::connect(&socket).unwrap();
            old.write_all(&[0x7f; 3][..offset]).unwrap();
            drop(old);
            thread::sleep(Duration::from_millis(40));
            let mut current = UnixStream::connect(&socket).unwrap();
            let original = [1000_i16.to_le_bytes(), (-2000_i16).to_le_bytes()].concat();
            current.write_all(&original.repeat(4410)).unwrap();
            wait(&|| {
                fs::read(&pcm).unwrap()[before..]
                    .chunks_exact(4)
                    .filter(|frame| *frame != [0, 0, 0, 0])
                    .count()
                    >= 960
            });
            let output = fs::read(&pcm).unwrap();
            for frame in output[before..]
                .chunks_exact(4)
                .filter(|frame| *frame != [0, 0, 0, 0])
            {
                let left = i16::from_le_bytes([frame[0], frame[1]]);
                let right = i16::from_le_bytes([frame[2], frame[3]]);
                assert!(
                    (left - 1000).abs() <= 1 && (right + 2000).abs() <= 1,
                    "offset {offset}: {left}, {right}"
                );
            }
            drop(current);
            thread::sleep(Duration::from_millis(150));
        }
        drop(mixer);
    }

    #[test]
    fn one_speaker_mixes_music_speech_and_alarm_and_keeps_music_during_stop() {
        let directory = tempfile::tempdir().unwrap();
        let player = directory.path().join("player");
        fs::write(
            &player,
            "#!/bin/sh\necho started >> \"$0.starts\"\nexec cat > \"$0.pcm\"\n",
        )
        .unwrap();
        fs::set_permissions(&player, fs::Permissions::from_mode(0o700)).unwrap();
        let volume = Arc::new(AtomicU8::new(50));
        let socket = directory.path().join("music.sock");
        let mut mixer = Mixer::open(&player, "test", socket.clone(), Arc::clone(&volume)).unwrap();
        let mut stream = UnixStream::connect(&socket).unwrap();
        let song = thread::spawn(move || {
            let _ = stream.write_all(&2000_i16.to_le_bytes().repeat(44_100 * 2 * 2));
        });
        let ready_by = Instant::now() + Duration::from_secs(3);
        while !directory.path().join("player.pcm").exists() {
            assert!(
                Instant::now() < ready_by,
                "fake player did not start: {:?}",
                mixer.player.lock().unwrap().as_mut().unwrap().try_wait()
            );
            thread::sleep(Duration::from_millis(10));
        }
        let tail = || {
            let bytes = fs::read(directory.path().join("player.pcm")).unwrap();
            assert!(bytes.len() >= FRAMES * 4);
            let samples: Vec<_> = bytes[bytes.len() - FRAMES * 4..]
                .chunks_exact(2)
                .map(|sample| i16::from_le_bytes(sample.try_into().unwrap()))
                .collect();
            samples.iter().map(|sample| i32::from(*sample)).sum::<i32>() / samples.len() as i32
        };
        thread::sleep(Duration::from_millis(120));
        assert!((995..=1005).contains(&tail()));
        assert!(mixer.music_playing());
        assert!(!mixer.playing());
        mixer.duck(true); // The wake interval starts before foreground audio.
        thread::sleep(Duration::from_millis(180));
        assert!((240..=280).contains(&tail()));
        mixer.stream(AudioSource::Speech).unwrap();
        mixer.queue(&1000_i16.to_le_bytes().repeat(24_000)).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!((740..=765).contains(&tail()));
        mixer.stop().unwrap();
        mixer.duck(false);
        thread::sleep(Duration::from_millis(120));
        assert!((300..=550).contains(&tail()));
        assert!(mixer.music_playing());
        assert!(!mixer.playing());
        volume.store(0, Ordering::Relaxed);
        thread::sleep(Duration::from_millis(100));
        assert_eq!(tail(), 0);
        mixer.stream(AudioSource::Alarm).unwrap();
        mixer.queue(&1000_i16.to_le_bytes().repeat(24_000)).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!((245..=255).contains(&tail()));
        assert_eq!(
            fs::read_to_string(directory.path().join("player.starts"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        drop(mixer);
        song.join().unwrap();
    }
    #[test]
    fn dead_player_restarts_and_plays_an_alarm_queued_during_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let player = directory.path().join("player");
        let unavailable = directory.path().join("unavailable");
        let pcm = directory.path().join("player.pcm");
        fs::write(
            &player,
            "#!/bin/sh\necho started >> \"$0.starts\"\nexec cat >> \"$0.pcm\"\n",
        )
        .unwrap();
        fs::set_permissions(&player, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.path().join("music.sock");
        let mut mixer =
            Mixer::open(&player, "test", socket.clone(), Arc::new(AtomicU8::new(0))).unwrap();
        let wait = |condition: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !condition() {
                assert!(Instant::now() < deadline, "mixer did not recover");
                thread::sleep(Duration::from_millis(10));
            }
        };
        wait(&|| fs::metadata(&pcm).is_ok_and(|file| file.len() >= (FRAMES * 4) as u64));
        let mut old_music = UnixStream::connect(&socket).unwrap();
        old_music
            .write_all(&2000_i16.to_le_bytes().repeat(441))
            .unwrap();
        // Hold the outage open across several restart attempts, not just the backoff.
        fs::rename(&player, &unavailable).unwrap();
        mixer
            .player
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .kill()
            .unwrap();
        wait(&|| mixer.player.lock().unwrap().is_none());
        let before = fs::metadata(&pcm).unwrap().len() as usize;
        mixer.stream(AudioSource::Alarm).unwrap();
        assert!(mixer.queue(&1000_i16.to_le_bytes().repeat(2400)).unwrap());
        mixer.finish();
        thread::sleep(Duration::from_millis(250));
        mixer.update().unwrap();
        assert!(mixer.playing(), "the pending alarm must survive the outage");
        fs::rename(&unavailable, &player).unwrap();
        wait(&|| {
            fs::read(&pcm).unwrap()[before..]
                .chunks_exact(2)
                .any(|sample| i16::from_le_bytes(sample.try_into().unwrap()) == 250)
        });
        wait(&|| !mixer.playing());
        mixer.update().unwrap();
        assert_eq!(
            fs::read_to_string(directory.path().join("player.starts"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        // The receiver must accept a fresh Spotify connection after rebinding.
        let mut music = UnixStream::connect(&socket).unwrap();
        music
            .write_all(&2000_i16.to_le_bytes().repeat(4410 * 2))
            .unwrap();
        wait(&|| mixer.music_playing());
        mixer.stream(AudioSource::Speech).unwrap();
        assert!(mixer.queue(&1000_i16.to_le_bytes().repeat(2400)).unwrap());
        mixer.finish();
        wait(&|| !mixer.playing());
        mixer.update().unwrap();
    }
    #[test]
    fn stalled_player_restarts_without_losing_speech_and_music_can_return() {
        let directory = tempfile::tempdir().unwrap();
        let player = directory.path().join("player");
        // The first process stays alive but never reads stdin, like a stuck
        // ALSA write. Its replacement consumes the same raw PCM stream.
        fs::write(&player,"#!/bin/sh\nif test ! -f \"$0.once\"; then touch \"$0.once\"; exec sleep 30; fi\nexec cat > \"$0.pcm\"\n").unwrap();
        fs::set_permissions(&player, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.path().join("music.sock");
        let mut mixer = Mixer::open(
            &player,
            "test",
            socket.clone(),
            Arc::new(AtomicU8::new(100)),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !directory.path().join("player.pcm").exists() {
            assert!(Instant::now() < deadline, "stalled writer did not restart");
            thread::sleep(Duration::from_millis(10));
        }
        mixer.stream(AudioSource::Speech).unwrap();
        mixer.queue(&1000_i16.to_le_bytes().repeat(2400)).unwrap();
        mixer.finish();
        while mixer.playing() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        let mut music = UnixStream::connect(socket).unwrap();
        music
            .write_all(&2000_i16.to_le_bytes().repeat(4410))
            .unwrap();
        while !mixer.music_playing() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        // Leave the socket connected, as librespot does between songs. Its
        // resampler tail must not keep streaming or suppress idle behaviour.
        thread::sleep(Duration::from_millis(400));
        let pulse = mixer.pulse().unwrap();
        assert!(!pulse.streaming && !pulse.audible);
        mixer.update().unwrap();
    }

    #[test]
    fn ducking_reaches_one_quarter_smoothly_and_returns_after_voice() {
        let mut gain = 1.;
        let mut last = gain;
        for _ in 0..RATE / 2 {
            gain = duck_gain(gain, 0.25);
            assert!(gain <= last && gain >= 0.25);
            assert!((gain - last).abs() < 0.001);
            last = gain;
        }
        assert!((gain - 0.25).abs() < 0.001);
        for _ in 0..RATE * 3 {
            gain = duck_gain(gain, 1.);
            assert!(gain >= last && gain <= 1.);
            last = gain;
        }
        assert!((gain - 1.).abs() < 0.001);
    }
    #[test]
    fn streaming_music_resampling_keeps_channels_and_duration() {
        let mut source = Resampler::default();
        source
            .input
            .extend((0..442).map(|i| [i as f32, -(i as f32)]));
        for i in 0..480 {
            let sample = source.take(44_100).unwrap();
            assert!((sample[0] - i as f32 * 0.91875).abs() < 0.001);
            assert_eq!(sample[0], -sample[1]);
        }
        assert_eq!(source.input.len(), 1);
    }
}
