//! Shared character state. The persistence worker owns all file and clock access.
use super::speech::SpeechCue;
use chrono::{Local, Timelike};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttentionTarget {
    #[default]
    None,
    Speaker {
        track_id: u64,
    },
    Person {
        track_id: u64,
    },
    Hands,
    Object {
        class: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attention {
    #[serde(flatten)]
    pub target: AttentionTarget,
    pub since: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MindState {
    pub energy: f64,
    pub mood: f64,
    pub activity: f64,
    pub label: String,
    pub attention: Attention,
    pub updated_at: f64,
}

impl Default for MindState {
    fn default() -> Self {
        Self {
            energy: 0.5,
            mood: 0.,
            activity: 0.,
            label: "content".into(),
            attention: Attention {
                target: AttentionTarget::None,
                since: 0.,
            },
            updated_at: 0.,
        }
    }
}

pub fn circadian(hour: f64) -> f64 {
    let hour = hour.rem_euclid(24.);
    let points = [
        (0., 0.15),
        (6., 0.15),
        (10., 0.9),
        (15., 0.7),
        (18., 0.8),
        (22., 0.3),
        (23., 0.15),
        (24., 0.15),
    ];
    for pair in points.windows(2) {
        let [(start, a), (end, b)] = pair else {
            unreachable!()
        };
        if hour <= *end {
            return a + (b - a) * (hour - start) / (end - start);
        }
    }
    0.15
}

impl MindState {
    pub fn advance(&mut self, wall: f64, hour: f64) {
        let elapsed = (wall - self.updated_at).max(0.);
        self.activity *= (-elapsed / 1800.).exp2();
        self.mood *= (-elapsed / 1200.).exp2();
        self.updated_at = wall;
        self.refresh(hour);
    }
    fn refresh(&mut self, hour: f64) {
        self.energy = (0.6 * circadian(hour) + 0.4 * self.activity).clamp(0., 1.);
        self.label = if self.energy < 0.3 {
            "sleepy"
        } else if self.mood > 0.3 {
            "cheerful"
        } else if self.mood < -0.3 {
            "subdued"
        } else if self.activity >= 0.5 && self.mood.abs() <= 0.3 {
            "curious"
        } else {
            "content"
        }
        .into();
    }
    pub fn confirmed_turn(&mut self) {
        self.activity = (self.activity + 0.15).min(1.);
    }
    pub fn cue(&mut self, cue: SpeechCue) {
        self.mood = (self.mood
            + match cue {
                SpeechCue::Happy | SpeechCue::Laugh => 0.15,
                SpeechCue::Agree | SpeechCue::Curious => 0.05,
                SpeechCue::Sympathy | SpeechCue::Unsure | SpeechCue::Disagree => -0.05,
                _ => 0.,
            })
        .clamp(-1., 1.);
        if cue == SpeechCue::Surprised {
            self.activity = (self.activity + 0.1).min(1.);
        }
    }
    pub fn attend(&mut self, target: AttentionTarget) {
        if self.attention.target != target {
            self.attention = Attention {
                target,
                since: self.updated_at,
            };
        }
    }
    fn valid(&self) -> bool {
        (0. ..=1.).contains(&self.energy)
            && (0. ..=1.).contains(&self.activity)
            && (-1. ..=1.).contains(&self.mood)
            && self.updated_at.is_finite()
            && self.updated_at >= 0.
            && self.attention.since.is_finite()
            && self.attention.since >= 0.
            && self.attention.since <= self.updated_at
    }
    pub fn load(path: &Path, wall: f64, hour: f64) -> Self {
        let mut state = (|| -> Option<Self> {
            let mut bytes = Vec::new();
            fs::File::open(path)
                .ok()?
                .take(4097)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() > 4096 {
                return None;
            }
            let state: Self = serde_json::from_slice(&bytes).ok()?;
            (state.valid() && state.updated_at <= wall).then_some(state)
        })()
        .unwrap_or_default();
        state.advance(wall, hour);
        state.attend(AttentionTarget::None);
        state
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("Invalid mind path"))?;
        fs::create_dir_all(parent)?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&serde_json::to_vec(self)?)?;
            file.sync_all()?;
            fs::rename(&temporary, path)?;
            fs::File::open(parent)?.sync_all()
        })();
        let _ = fs::remove_file(temporary);
        result
    }
}

enum Update {
    Snapshot(MindState),
    Shutdown(MindState),
}
pub(crate) struct MindStore {
    sender: mpsc::SyncSender<Update>,
    worker: Option<thread::JoinHandle<()>>,
    hour: Arc<AtomicU64>,
}
impl MindStore {
    pub fn start(path: Option<PathBuf>) -> (Self, MindState) {
        let clock = Local::now();
        let hour_value = clock.hour() as f64 + clock.minute() as f64 / 60.;
        let wall = clock.timestamp_millis() as f64 / 1000.;
        let state = path
            .as_ref()
            .map(|p| MindState::load(p, wall, hour_value))
            .unwrap_or_else(|| {
                let mut state = MindState::default();
                state.advance(wall, hour_value);
                state
            });
        let hour = Arc::new(AtomicU64::new(hour_value.to_bits()));
        let worker_hour = Arc::clone(&hour);
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let mut latest = None;
            let mut saved = Instant::now() - Duration::from_secs(10);
            loop {
                let clock = Local::now();
                worker_hour.store(
                    (clock.hour() as f64 + clock.minute() as f64 / 60.).to_bits(),
                    Ordering::Relaxed,
                );
                let shutdown = match receiver.recv_timeout(Duration::from_secs(1)) {
                    Ok(Update::Snapshot(state)) => {
                        latest = Some(state);
                        false
                    }
                    Ok(Update::Shutdown(state)) => {
                        latest = Some(state);
                        true
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => true,
                    Err(mpsc::RecvTimeoutError::Timeout) => false,
                };
                if shutdown || saved.elapsed() >= Duration::from_secs(10) {
                    if let (Some(path), Some(state)) = (&path, latest.take()) {
                        if let Err(error) = state.save(path) {
                            eprintln!("oriond: mind save failed: {error}");
                        }
                        saved = Instant::now();
                    }
                }
                if shutdown {
                    break;
                }
            }
        });
        (
            Self {
                sender,
                worker: Some(worker),
                hour,
            },
            state,
        )
    }
    pub fn hour(&self) -> f64 {
        f64::from_bits(self.hour.load(Ordering::Relaxed))
    }
    pub fn snapshot(&self, state: &MindState) {
        let _ = self.sender.try_send(Update::Snapshot(state.clone()));
    }
    pub fn shutdown(mut self, state: &MindState) {
        let _ = self.sender.send(Update::Shutdown(state.clone()));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curve_knots_and_interpolation() {
        for (hour, expected) in [
            (0., 0.15),
            (5., 0.15),
            (6., 0.15),
            (8., 0.525),
            (10., 0.9),
            (15., 0.7),
            (18., 0.8),
            (22., 0.3),
            (23., 0.15),
        ] {
            assert!((circadian(hour) - expected).abs() < 1e-9);
        }
    }
    #[test]
    fn decay_cues_and_labels() {
        let mut state = MindState::default();
        state.confirmed_turn();
        state.cue(SpeechCue::Happy);
        state.cue(SpeechCue::Laugh);
        state.cue(SpeechCue::Agree);
        state.advance(0., 12.);
        assert_eq!(state.label, "cheerful");
        state.advance(1200., 12.);
        assert!((state.mood - 0.175).abs() < 1e-9);
        state.advance(1800., 12.);
        assert!((state.activity - 0.075).abs() < 1e-9);
        state.cue(SpeechCue::Surprised);
        assert!((state.activity - 0.175).abs() < 1e-9);
        for cue in [
            SpeechCue::Curious,
            SpeechCue::Sympathy,
            SpeechCue::Unsure,
            SpeechCue::Disagree,
        ] {
            let before = state.mood;
            state.cue(cue);
            assert!(
                (state.mood
                    - before
                    - if cue == SpeechCue::Curious {
                        0.05
                    } else {
                        -0.05
                    })
                .abs()
                    < 1e-9
            );
        }
        state.advance(1800., 1.);
        assert_eq!(state.label, "sleepy");
    }
    #[test]
    fn persistence_is_private_atomic_and_decays_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mind.json");
        let mut state = MindState::default();
        state.advance(100., 12.);
        state.activity = 0.8;
        state.mood = -0.8;
        state.attend(AttentionTarget::Hands);
        state.save(&path).unwrap();
        let loaded = MindState::load(&path, 1900., 12.);
        assert!((loaded.activity - 0.4).abs() < 1e-9);
        assert!((loaded.mood + 0.8 * 2_f64.powf(-1.5)).abs() < 1e-9);
        assert_eq!(loaded.attention.target, AttentionTarget::None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::write(&path, "broken").unwrap();
        assert_eq!(MindState::load(&path, 1900., 12.).activity, 0.);
        fs::remove_file(&path).unwrap();
        assert_eq!(MindState::load(&path, 1900., 12.).mood, 0.);
    }
    #[test]
    fn attention_since_changes_only_with_target() {
        let mut state = MindState::default();
        state.advance(100., 12.);
        state.attend(AttentionTarget::Person { track_id: 1 });
        state.advance(110., 12.);
        state.attend(AttentionTarget::Person { track_id: 1 });
        assert_eq!(state.attention.since, 100.);
        state.attend(AttentionTarget::Hands);
        assert_eq!(state.attention.since, 110.);
    }
}
