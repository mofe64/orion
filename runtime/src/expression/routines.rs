//! Persistent user modes and alerts. Deadlines and audio belong to the Pi runtime.
use crate::AudioDevice;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, fs, io::Write, path::PathBuf};

pub const ALERT_LIMIT_SECONDS: f64 = 300.0;
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserMode {
    #[default]
    Idle,
    Lamp,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Alert {
    pub id: u64,
    pub kind: String,
    pub label: String,
    pub due_unix: f64,
    pub state: String,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    mode: UserMode,
    next_id: u64,
    alerts: Vec<Alert>,
    ring_until_unix: Option<f64>,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    SetMode { mode: UserMode },
    Timer { seconds: f64, label: String },
    Alarm { due_unix: f64, label: String },
    List,
    Cancel { id: u64 },
    Stop,
}
pub struct Routines {
    saved: Saved,
    path: Option<PathBuf>,
    timer_deadlines: HashMap<u64, f64>,
    ring_deadline: Option<f64>,
    audio_started: bool,
    sample: usize,
    pub error: Option<String>,
}
impl Routines {
    pub fn load(path: Option<PathBuf>, wall: f64, now: f64) -> Result<Self, String> {
        let saved: Saved = match path.as_ref().map(fs::read) {
            Some(Ok(bytes)) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("Invalid routines file: {e}"))?
            }
            Some(Err(e)) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => Saved::default(),
        };
        let mut ids = std::collections::HashSet::new();
        if saved.ring_until_unix.is_some_and(|at| !at.is_finite())
            || saved.ring_until_unix.is_some() != saved.alerts.iter().any(|a| a.state == "ringing")
            || saved.alerts.len() > 32
            || saved.alerts.iter().any(|a| {
                a.id == 0
                    || a.id > saved.next_id
                    || !ids.insert(a.id)
                    || a.label.chars().count() > 80
                    || a.label.chars().any(char::is_control)
                    || !a.due_unix.is_finite()
                    || !["timer", "alarm"].contains(&a.kind.as_str())
                    || ![
                        "pending",
                        "ringing",
                        "dismissed",
                        "expired",
                        "missed",
                        "cancelled",
                        "failed",
                    ]
                    .contains(&a.state.as_str())
            })
        {
            return Err("Invalid saved alerts".into());
        }
        let timer_deadlines = saved
            .alerts
            .iter()
            .filter(|a| a.kind == "timer" && a.state == "pending")
            .map(|a| (a.id, now + a.due_unix - wall))
            .collect();
        let ring_deadline = saved
            .ring_until_unix
            .map(|at| now + (at - wall).clamp(0., ALERT_LIMIT_SECONDS));
        Ok(Self {
            saved,
            path,
            timer_deadlines,
            ring_deadline,
            audio_started: false,
            sample: 0,
            error: None,
        })
    }
    pub fn mode(&self) -> UserMode {
        self.saved.mode
    }
    pub fn ringing(&self) -> bool {
        self.ring_deadline.is_some()
    }
    pub fn status(&self, wall: f64, now: f64) -> Value {
        let mut alerts = self.saved.alerts.clone();
        for alert in &mut alerts {
            if alert.state == "pending" {
                if let Some(at) = self.timer_deadlines.get(&alert.id) {
                    alert.due_unix = wall + at - now;
                }
            }
        }
        json!({"mode":self.mode(), "now_unix":wall, "alerts":alerts, "ringing":self.ringing(),
            "remaining_ring_seconds":self.ring_deadline.map(|at|(at-now).max(0.)), "error":self.error})
    }
    fn commit(&mut self, saved: Saved) -> Result<(), String> {
        if let Some(path) = &self.path {
            let parent = path.parent().ok_or("Routines path needs a parent")?;
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            let temporary = path.with_extension("json.tmp");
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
            file.write_all(&serde_json::to_vec(&saved).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            // Atomic replacement survives service restarts. Avoid forcing an SD
            // card flush in the motion loop; sudden power loss can lose a recent write.
            fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        }
        self.saved = saved;
        Ok(())
    }
    pub fn request(
        &mut self,
        request: Request,
        wall: f64,
        now: f64,
        audio: &mut dyn AudioDevice,
    ) -> Result<Value, String> {
        let mut saved = self.saved.clone();
        let mut created = None;
        match request {
            Request::List => return Ok(self.status(wall, now)),
            Request::Stop => {
                self.stop("dismissed", audio)?;
                return Ok(self.status(wall, now));
            }
            Request::SetMode { mode } => saved.mode = mode,
            Request::Cancel { id } => {
                let alert = saved
                    .alerts
                    .iter_mut()
                    .find(|a| a.id == id)
                    .ok_or("Unknown alert ID")?;
                if !["pending", "ringing"].contains(&alert.state.as_str()) {
                    return Err("Alert already finished".into());
                }
                alert.state = "cancelled".into();
            }
            request => {
                let (kind, due, label) = match request {
                    Request::Timer { seconds, label } => {
                        if !seconds.is_finite() || !(1.0..=604800.).contains(&seconds) {
                            return Err("Timer must be between one second and seven days".into());
                        }
                        ("timer", wall + seconds, label)
                    }
                    Request::Alarm { due_unix, label } => {
                        if !due_unix.is_finite()
                            || due_unix <= wall
                            || due_unix > wall + 366. * 86400.
                        {
                            return Err("Alarm must be in the next 366 days".into());
                        }
                        ("alarm", due_unix, label)
                    }
                    _ => unreachable!(),
                };
                if label.chars().count() > 80 || label.chars().any(char::is_control) {
                    return Err("Use a label of at most 80 printable characters".into());
                }
                if saved
                    .alerts
                    .iter()
                    .filter(|a| ["pending", "ringing"].contains(&a.state.as_str()))
                    .count()
                    >= 16
                {
                    return Err("At most 16 active alerts are allowed".into());
                }
                if saved.alerts.len() >= 32 {
                    let index = saved
                        .alerts
                        .iter()
                        .position(|a| !["pending", "ringing"].contains(&a.state.as_str()))
                        .unwrap();
                    saved.alerts.remove(index);
                }
                saved.next_id = saved.next_id.checked_add(1).ok_or("Alert IDs exhausted")?;
                let alert = Alert {
                    id: saved.next_id,
                    kind: kind.into(),
                    label,
                    due_unix: due,
                    state: "pending".into(),
                };
                created = Some(alert.clone());
                saved.alerts.push(alert);
            }
        }
        self.commit(saved)?;
        self.timer_deadlines.retain(|id, _| {
            self.saved
                .alerts
                .iter()
                .any(|a| a.id == *id && a.state == "pending")
        });
        if let Some(alert) = &created {
            if alert.kind == "timer" {
                self.timer_deadlines
                    .insert(alert.id, now + alert.due_unix - wall);
            }
        }
        if self.ringing() && !self.saved.alerts.iter().any(|a| a.state == "ringing") {
            self.stop("dismissed", audio)?;
        }
        Ok(json!({"alert":created,"status":self.status(wall,now)}))
    }
    /// Persist firing before starting playback; restarting cannot give an alert another five minutes.
    pub fn due(
        &mut self,
        wall: f64,
        now: f64,
        audio: &mut dyn AudioDevice,
    ) -> Result<bool, String> {
        // Expire the old group first so an alert due on the boundary gets its own window.
        if self.ring_deadline.is_some_and(|at| now >= at) {
            self.stop("expired", audio)?;
        }
        let mut saved = self.saved.clone();
        let mut changed = false;
        let was_ringing = self.ringing();
        for alert in &mut saved.alerts {
            if alert.state != "pending" {
                continue;
            }
            let late = if alert.kind == "timer" {
                now - self
                    .timer_deadlines
                    .get(&alert.id)
                    .copied()
                    .unwrap_or(now + alert.due_unix - wall)
            } else {
                wall - alert.due_unix
            };
            if late < 0. {
                continue;
            }
            changed = true;
            if late >= ALERT_LIMIT_SECONDS {
                alert.state = "missed".into();
                continue;
            }
            alert.state = "ringing".into();
            saved
                .ring_until_unix
                .get_or_insert(wall + ALERT_LIMIT_SECONDS - late);
        }
        if changed {
            self.commit(saved)?;
        }
        if self.ring_deadline.is_none() {
            self.ring_deadline = self
                .saved
                .ring_until_unix
                .map(|at| now + (at - wall).clamp(0., ALERT_LIMIT_SECONDS));
        }
        Ok((!was_ringing || !self.audio_started) && self.ringing())
    }
    pub fn stop(&mut self, state: &str, audio: &mut dyn AudioDevice) -> Result<(), String> {
        // Always silence first, including when the disk has become unwritable.
        if self.audio_started {
            audio.stop().map_err(|e| e.to_string())?;
        }
        self.audio_started = false;
        self.ring_deadline = None;
        let mut saved = self.saved.clone();
        saved.ring_until_unix = None;
        for alert in &mut saved.alerts {
            if alert.state == "ringing" {
                alert.state = state.into();
            }
        }
        // Keep the in-memory dismissal even if persistence fails; expose the error.
        self.saved = saved.clone();
        self.commit(saved)
    }
    pub fn tick_audio(&mut self, now: f64, audio: &mut dyn AudioDevice) -> Result<(), String> {
        let Some(until) = self.ring_deadline else {
            return Ok(());
        };
        if now >= until {
            return self.stop("expired", audio);
        }
        let result = (|| {
            if !self.audio_started {
                audio.start_pcm("orion_alert")?;
                self.audio_started = true;
                self.sample = 0;
            }
            audio.update()?;
            if !audio.is_playing() {
                return Err(crate::Error::Runtime(
                    "Alarm audio stopped unexpectedly".into(),
                ));
            }
            // A bounded 80 ms queue keeps capture and the 50 Hz motor loop responsive.
            for _ in 0..4 {
                let pcm = alert_pcm(self.sample, 480);
                if !audio.queue_pcm(&pcm)? {
                    break;
                }
                self.sample += 480;
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.error = Some(format!("Alarm playback failed: {error}"));
            let _ = self.stop("failed", audio);
            return Err(self.error.clone().unwrap());
        }
        Ok(())
    }
}
/// Repeating high/low double pulse with a short listening gap, 24 kHz PCM16.
/// Fixed 0.65 peak leaves headroom; no mixer or user volume settings are changed.
fn alert_pcm(start: usize, count: usize) -> Vec<u8> {
    (start..start + count)
        .map(|sample| {
            let t = (sample % 28800) as f64 / 24000.;
            let phase = t % 0.6;
            let envelope = if phase < 0.48 {
                (phase / 0.012).min(1.) * ((0.48 - phase) / 0.012).min(1.)
            } else {
                0.
            };
            let hz = if t < 0.6 { 880. } else { 1174.66 };
            ((std::f64::consts::TAU * hz * t).sin() * envelope * 0.65 * i16::MAX as f64) as i16
        })
        .flat_map(i16::to_le_bytes)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RecordingAudioDevice;
    #[test]
    fn timer_ignores_clock_jumps_and_ringing_restart_keeps_original_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        let mut audio = RecordingAudioDevice::blocking();
        let mut r = Routines::load(Some(path.clone()), 1000., 0.).unwrap();
        r.request(
            Request::Timer {
                seconds: 10.,
                label: "tea".into(),
            },
            1000.,
            0.,
            &mut audio,
        )
        .unwrap();
        assert!(!r.due(5000., 9., &mut audio).unwrap());
        assert!(r.due(5001., 10., &mut audio).unwrap());
        r.tick_audio(10., &mut audio).unwrap();
        assert!(audio.is_playing());
        let mut restarted = Routines::load(Some(path.clone()), 5101., 0.).unwrap();
        assert_eq!(restarted.status(5101., 0.)["remaining_ring_seconds"], 200.);
        audio.stop().unwrap();
        restarted.tick_audio(199.99, &mut audio).unwrap();
        assert!(audio.is_playing());
        restarted.tick_audio(200., &mut audio).unwrap();
        assert!(!audio.is_playing());
        assert!(!restarted.ringing());
        assert_eq!(
            Routines::load(Some(path), 5400., 0.).unwrap().saved.alerts[0].state,
            "expired"
        );
    }
    #[test]
    fn alarms_due_in_rest_merge_without_extending_ring_and_stop_preserves_future_alerts() {
        let mut audio = RecordingAudioDevice::blocking();
        let mut r = Routines::load(None, 100., 0.).unwrap();
        for due in [101., 120., 900.] {
            r.request(
                Request::Alarm {
                    due_unix: due,
                    label: "alarm".into(),
                },
                100.,
                0.,
                &mut audio,
            )
            .unwrap();
        }
        assert!(r.due(101., 1., &mut audio).unwrap());
        r.tick_audio(1., &mut audio).unwrap();
        assert!(!r.due(120., 20., &mut audio).unwrap());
        assert_eq!(r.ring_deadline, Some(301.));
        r.request(Request::Stop, 120., 20., &mut audio).unwrap();
        assert!(!audio.is_playing());
        assert_eq!(r.saved.alerts[0].state, "dismissed");
        assert_eq!(r.saved.alerts[1].state, "dismissed");
        assert_eq!(r.saved.alerts[2].state, "pending");
    }
    #[test]
    fn alert_due_at_cutoff_starts_a_new_ring_window() {
        let mut audio = RecordingAudioDevice::blocking();
        let mut r = Routines::load(None, 100., 0.).unwrap();
        for due in [101., 401.] {
            r.request(
                Request::Alarm {
                    due_unix: due,
                    label: "alarm".into(),
                },
                100.,
                0.,
                &mut audio,
            )
            .unwrap();
        }
        assert!(r.due(101., 1., &mut audio).unwrap());
        r.tick_audio(1., &mut audio).unwrap();
        assert!(r.due(401., 301., &mut audio).unwrap());
        assert_eq!(r.saved.alerts[0].state, "expired");
        assert_eq!(r.saved.alerts[1].state, "ringing");
        assert_eq!(r.ring_deadline, Some(601.));
        r.tick_audio(301., &mut audio).unwrap();
        assert!(audio.is_playing());
    }
    #[test]
    fn cancelled_timers_and_old_missed_alarms_do_not_play_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut audio = RecordingAudioDevice::blocking();
        let mut r = Routines::load(Some(path.clone()), 100., 0.).unwrap();
        r.request(
            Request::Timer {
                seconds: 10.,
                label: "first".into(),
            },
            100.,
            0.,
            &mut audio,
        )
        .unwrap();
        r.request(
            Request::Timer {
                seconds: 20.,
                label: "second".into(),
            },
            100.,
            0.,
            &mut audio,
        )
        .unwrap();
        r.request(Request::Cancel { id: 1 }, 100., 0., &mut audio)
            .unwrap();
        let mut r = Routines::load(Some(path), 500., 0.).unwrap();
        assert!(!r.due(500., 0., &mut audio).unwrap());
        assert_eq!(r.saved.alerts[1].state, "missed");
        assert!(!audio.is_playing());
    }
    #[test]
    fn mode_is_durable_and_failed_write_is_not_acknowledged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut audio = RecordingAudioDevice::blocking();
        let mut r = Routines::load(Some(path.clone()), 100., 0.).unwrap();
        r.request(
            Request::SetMode {
                mode: UserMode::Lamp,
            },
            100.,
            0.,
            &mut audio,
        )
        .unwrap();
        assert_eq!(
            Routines::load(Some(path), 100., 0.).unwrap().mode(),
            UserMode::Lamp
        );
        r.path = Some(dir.path().join("missing-parent-file/state.json"));
        fs::write(dir.path().join("missing-parent-file"), "blocked").unwrap();
        assert!(
            r.request(
                Request::SetMode {
                    mode: UserMode::Idle
                },
                100.,
                0.,
                &mut audio
            )
            .is_err()
        );
        assert_eq!(r.mode(), UserMode::Lamp);
    }
    #[test]
    fn alert_limits_and_pcm_headroom() {
        let mut audio = RecordingAudioDevice::blocking();
        let mut r = Routines::load(None, 100., 0.).unwrap();
        assert!(
            r.request(
                Request::Timer {
                    seconds: f64::NAN,
                    label: "".into()
                },
                100.,
                0.,
                &mut audio
            )
            .is_err()
        );
        for _ in 0..16 {
            r.request(
                Request::Timer {
                    seconds: 10.,
                    label: "tea".into(),
                },
                100.,
                0.,
                &mut audio,
            )
            .unwrap();
        }
        assert!(
            r.request(
                Request::Timer {
                    seconds: 10.,
                    label: "extra".into()
                },
                100.,
                0.,
                &mut audio
            )
            .is_err()
        );
        let pcm = alert_pcm(0, 28800);
        let samples: Vec<_> = pcm
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert!(samples.iter().map(|v| v.abs()).max().unwrap() > 20000);
        assert!(samples.iter().all(|v| v.abs() < 22000));
        assert!(samples[11520..14400].iter().all(|v| *v == 0));
    }
}
