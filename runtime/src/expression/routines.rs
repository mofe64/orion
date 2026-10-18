//! Persistent user modes and alerts. Deadlines and audio belong to the Pi runtime.
use super::alert_sound::AlertSound;
use crate::{
    AudioDevice,
    lamp::{LampPatch, LampProgram},
    lighting::Rgbw8,
};
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
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    Alarm,
    Timer,
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
struct SoundSettings {
    alarm: AlertSound,
    timer: AlertSound,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SavedSounds {
    settings: SoundSettings,
    ring_until_unix: Option<f64>,
    ring_sound: Option<AlertSound>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Alert {
    pub id: u64,
    pub kind: String,
    pub label: String,
    pub due_unix: f64,
    pub state: String,
    #[serde(default)]
    pub repeat: Option<Repeat>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub relative: bool,
    #[serde(default)]
    pub announcements: u8,
    #[serde(default)]
    pub announced_at: Option<f64>,
    #[serde(default)]
    pub announce_failures: u8,
    #[serde(default)]
    pub announce_after: Option<f64>,
    #[serde(default)]
    pub dismissal_notice: bool,
    #[serde(default)]
    pub series: u64,
    #[serde(default)]
    pub briefing: bool,
    #[serde(default)]
    pub snoozed: bool,
    #[serde(default)]
    pub sunrise_minutes: Option<u8>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Repeat {
    pub days: Vec<String>,
    pub time: String,
}
impl Repeat {
    pub fn validate(&self) -> Result<(), String> {
        let unique: std::collections::HashSet<_> = self.days.iter().collect();
        if self.days.is_empty()
            || unique.len() != self.days.len()
            || self.days.iter().any(|day| {
                !["mon", "tue", "wed", "thu", "fri", "sat", "sun"].contains(&day.as_str())
            })
            || self.time.len() != 5
            || chrono::NaiveTime::parse_from_str(&self.time, "%H:%M").is_err()
        {
            return Err("Repeat requires unique weekdays and HH:MM time".into());
        }
        Ok(())
    }
    pub fn next(&self, after: f64, zone: &str) -> Result<f64, String> {
        use chrono::{Datelike, TimeZone};
        self.validate()?;
        let zone: chrono_tz::Tz = zone
            .parse()
            .map_err(|_| "Set a valid Pi IANA timezone for repeats")?;
        let after =
            chrono::DateTime::from_timestamp(after.floor() as i64, 0).ok_or("Invalid date")?;
        let date = after.with_timezone(&zone).date_naive();
        let time = chrono::NaiveTime::parse_from_str(&self.time, "%H:%M").unwrap();
        for offset in 0..=8 {
            let day = date + chrono::Duration::days(offset);
            let name = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
                [day.weekday().num_days_from_monday() as usize];
            if self.days.iter().any(|d| d == name) {
                // A clock gap shifts this occurrence to the first valid minute,
                // including zones that skip a whole date. Folds use the earlier
                // instant so a recurring alert rings once on that local day.
                let local = day.and_time(time);
                let valid = (0..=1440).find_map(|minute| {
                    zone.from_local_datetime(&(local + chrono::Duration::minutes(minute)))
                        .earliest()
                });
                if let Some(at) = valid
                    && at.with_timezone(&chrono::Utc) > after
                {
                    return Ok(at.timestamp() as f64);
                }
            }
        }
        Err("No next repeat occurrence".into())
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Dismissed {
    alert: Alert,
    at: f64,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    mode: UserMode,
    next_id: u64,
    alerts: Vec<Alert>,
    ring_until_unix: Option<f64>,
    #[serde(default)]
    last_dismissed: Option<Dismissed>,
    // Sound state stays separate; new alert fields default when loading older files.
    #[serde(skip)]
    sounds: SoundSettings,
    // Latched when the first alert rings; preference changes apply to the next group.
    #[serde(skip)]
    ring_sound: Option<AlertSound>,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    StartLightRoutine {
        routine: LightRoutine,
        minutes: u16,
    },
    StopLightRoutine,
    SetMode {
        mode: UserMode,
    },
    SetSound {
        kind: AlertKind,
        sound: AlertSound,
    },
    Timer {
        seconds: f64,
        label: String,
    },
    Alarm {
        #[serde(default)]
        sunrise_minutes: Option<u8>,
        #[serde(default)]
        briefing: bool,
        #[serde(default)]
        due_unix: Option<f64>,
        label: String,
        #[serde(default)]
        repeat: Option<Repeat>,
    },
    Reminder {
        label: String,
        #[serde(default)]
        due_unix: Option<f64>,
        #[serde(default)]
        seconds: Option<f64>,
        #[serde(default)]
        repeat: Option<Repeat>,
    },
    Snooze {
        #[serde(default)]
        minutes: Option<u8>,
    },
    Announced {
        id: u64,
    },
    AnnounceFailed {
        id: u64,
    },
    DismissAnnouncement {
        id: u64,
    },
    List,
    Cancel {
        id: u64,
        #[serde(default)]
        series: bool,
    },
    Stop,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LightRoutine {
    WindDown,
    Focus,
}
struct LightRun {
    routine: LightRoutine,
    until: f64,
    timer: Option<u64>,
    previous_lamp: Option<LampProgram>,
    ending: bool,
}
struct Sunrise {
    id: u64,
    previous_lamp: Option<LampProgram>,
}
pub struct Routines {
    light_run: Option<LightRun>,
    sunrise: Option<Sunrise>,
    sunrise_latched: bool,
    sunrises_seen: std::collections::HashSet<u64>,
    saved: Saved,
    path: Option<PathBuf>,
    timer_deadlines: HashMap<u64, f64>,
    ring_deadline: Option<f64>,
    reminder_deadlines: HashMap<u64, f64>,
    announcement_deadlines: HashMap<u64, f64>,
    audio_started: bool,
    sample: usize,
    pub error: Option<String>,
}
impl Routines {
    /// Loads saved routines. A damaged save, for example one cut short by a
    /// power loss, is moved aside so the runtime still starts.
    pub fn load(path: Option<PathBuf>, wall: f64, now: f64) -> Result<Self, String> {
        let error = match Self::load_saved(path.clone(), wall, now) {
            Ok(routines) => return Ok(routines),
            Err(error) => error,
        };
        let Some(file) = path.as_ref() else {
            return Err(error);
        };
        // Damaged sound settings only reset the sounds; other damage resets both files.
        let sources = if error.starts_with("Invalid alarm sound settings") {
            vec![file.with_extension("sounds.json")]
        } else {
            vec![file.clone(), file.with_extension("sounds.json")]
        };
        let mut damaged = false;
        for source in sources {
            if source.exists() {
                let mut target = source.clone().into_os_string();
                target.push(".damaged");
                fs::rename(&source, &target).map_err(|e| format!("{error}; {e}"))?;
                damaged = true;
            }
        }
        if !damaged {
            return Err(error);
        }
        eprintln!("oriond: saved routines reset: {error}");
        let mut routines = Self::load_saved(path, wall, now)?;
        routines.error = Some(format!("Saved alarms and timers were reset: {error}"));
        Ok(routines)
    }
    fn load_saved(path: Option<PathBuf>, wall: f64, now: f64) -> Result<Self, String> {
        let mut saved: Saved = match path.as_ref().map(fs::read) {
            Some(Ok(bytes)) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("Invalid routines file: {e}"))?
            }
            Some(Err(e)) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => Saved::default(),
        };
        let mut ids = std::collections::HashSet::new();
        if saved.ring_until_unix.is_some_and(|at| !at.is_finite())
            || saved.ring_until_unix.is_some() != saved.alerts.iter().any(|a| a.state == "ringing")
            || saved.alerts.len() > 64
            || saved
                .last_dismissed
                .as_ref()
                .is_some_and(|d| !d.at.is_finite() || d.alert.id == 0 || d.alert.id > saved.next_id)
            || saved.alerts.iter().any(|a| {
                a.id == 0
                    || a.id > saved.next_id
                    || !ids.insert(a.id)
                    || a.label.chars().count() > if a.kind == "reminder" { 200 } else { 80 }
                    || a.label.chars().any(char::is_control)
                    || a.sunrise_minutes
                        .is_some_and(|m| a.kind != "alarm" || !(5..=60).contains(&m))
                    || !a.due_unix.is_finite()
                    || a.announcements > 2
                    || a.announce_failures > 3
                    || a.announce_after.is_some_and(|at| !at.is_finite())
                    || a.announced_at.is_some_and(|at| !at.is_finite())
                    || a.repeat.as_ref().is_some_and(|rule| {
                        rule.validate().is_err()
                            || a.kind == "timer"
                            || a.timezone
                                .as_ref()
                                .is_none_or(|z| z.parse::<chrono_tz::Tz>().is_err())
                    })
                    || !["timer", "alarm", "reminder"].contains(&a.kind.as_str())
                    || ![
                        "pending",
                        "ringing",
                        "announcing",
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
            .filter(|a| (a.kind == "timer" || a.relative) && a.state == "pending")
            .map(|a| (a.id, now + a.due_unix - wall))
            .collect();
        let ring_deadline = saved
            .ring_until_unix
            .map(|at| now + (at - wall).clamp(0., ALERT_LIMIT_SECONDS));
        let sounds: SavedSounds = match path
            .as_ref()
            .map(|p| fs::read(p.with_extension("sounds.json")))
        {
            Some(Ok(bytes)) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("Invalid alarm sound settings: {e}"))?,
            Some(Err(e)) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => SavedSounds::default(),
        };
        saved.sounds = sounds.settings;
        // A stale sound record can follow an interrupted pair of writes.
        // Only reuse it for the same ringing window; otherwise use two-tone.
        saved.ring_sound = ring_deadline.map(|_| {
            if sounds.ring_until_unix == saved.ring_until_unix {
                sounds.ring_sound.unwrap_or_default()
            } else {
                AlertSound::TwoTone
            }
        });
        let reminder_deadlines = saved
            .alerts
            .iter()
            .filter(|a| a.state == "announcing")
            .filter_map(|a| a.announced_at.map(|at| (a.id, now + at + 120. - wall)))
            .collect();
        let saved_deadlines = saved
            .alerts
            .iter()
            .filter_map(|a| a.announce_after.map(|at| (a.id, now + at - wall)))
            .collect();
        Ok(Self {
            light_run: None,
            sunrise: None,
            sunrise_latched: false,
            sunrises_seen: Default::default(),
            saved,
            path,
            timer_deadlines,
            ring_deadline,
            reminder_deadlines,
            announcement_deadlines: saved_deadlines,
            audio_started: false,
            sample: 0,
            error: None,
        })
    }
    pub fn mode(&self) -> UserMode {
        if self.light_run.is_some() {
            UserMode::Lamp
        } else {
            self.saved.mode
        }
    }
    pub fn owner_lamp<'a>(&'a self, manual: &'a Option<LampProgram>) -> Option<&'a LampProgram> {
        if let Some(run) = &self.light_run {
            run.previous_lamp.as_ref()
        } else if let Some(sunrise) = &self.sunrise {
            sunrise.previous_lamp.as_ref()
        } else {
            manual.as_ref()
        }
    }
    pub fn start_light(
        &mut self,
        routine: LightRoutine,
        minutes: u16,
        wall: f64,
        now: f64,
        manual: &mut Option<LampProgram>,
        audio: &mut dyn AudioDevice,
    ) -> Result<Value, String> {
        let valid = match routine {
            LightRoutine::WindDown => 10..=60,
            LightRoutine::Focus => 5..=120,
        };
        if !valid.contains(&minutes) {
            return Err("Wind-down needs 10–60 minutes; focus needs 5–120 minutes".into());
        }
        self.stop_light(wall, now, manual, audio)?;
        let previous_lamp = manual.clone();
        let seconds = f64::from(minutes) * 60.;
        let timer = if routine == LightRoutine::Focus {
            Some(
                self.request(
                    Request::Timer {
                        seconds,
                        label: "Focus session".into(),
                    },
                    wall,
                    now,
                    audio,
                )?["alert"]["id"]
                    .as_u64()
                    .ok_or("Missing focus timer")?,
            )
        } else {
            None
        };
        let patch = if routine == LightRoutine::Focus {
            json!({"brightness":1.,"effect":"solid","colors":[[150,190,255,110]]})
        } else {
            json!({"brightness":0.08,"effect":"solid","colors":[[40,20,4,210]],"transition_seconds":seconds})
        };
        *manual = Some(
            manual
                .clone()
                .unwrap_or_default()
                .updated_at(serde_json::from_value::<LampPatch>(patch).unwrap(), now)
                .map_err(|e| e.to_string())?,
        );
        self.light_run = Some(LightRun {
            routine,
            until: now + seconds,
            timer,
            previous_lamp,
            ending: false,
        });
        Ok(self.status(wall, now))
    }
    pub fn stop_light(
        &mut self,
        wall: f64,
        now: f64,
        manual: &mut Option<LampProgram>,
        audio: &mut dyn AudioDevice,
    ) -> Result<(), String> {
        if let Some(run) = self.light_run.as_ref()
            && let Some(id) = run.timer
            && self
                .saved
                .alerts
                .iter()
                .any(|a| a.id == id && ["pending", "ringing"].contains(&a.state.as_str()))
        {
            self.request(Request::Cancel { id, series: false }, wall, now, audio)?;
        }
        self.sunrise_latched = false;
        if let Some(run) = self.light_run.take() {
            *manual = run.previous_lamp;
        }
        if let Some(sunrise) = self.sunrise.take() {
            *manual = sunrise.previous_lamp;
        }
        Ok(())
    }
    pub fn curiosity_quiet(&self) -> bool {
        self.light_run.is_some() || self.sunrise_active() || self.ringing()
    }
    pub fn sunrise_active(&self) -> bool {
        self.sunrise.is_some() || self.sunrise_latched
    }
    /// Only this runtime-owned scheduler creates light deadlines. Returns true
    /// once when wind-down should enter the ordinary deferred sleep lifecycle.
    pub fn light_tick(
        &mut self,
        wall: f64,
        now: f64,
        manual: &mut Option<LampProgram>,
        available: bool,
    ) -> Result<bool, String> {
        if let Some(sunrise) = &self.sunrise
            && !self
                .saved
                .alerts
                .iter()
                .any(|a| a.id == sunrise.id && ["pending", "ringing"].contains(&a.state.as_str()))
        {
            let sunrise = self.sunrise.take().unwrap();
            let cancelled = self
                .saved
                .alerts
                .iter()
                .find(|a| a.id == sunrise.id)
                .is_none_or(|a| a.state == "cancelled" || a.state == "failed");
            if cancelled {
                *manual = sunrise.previous_lamp;
            } else {
                *manual = manual.as_ref().map(LampProgram::finish);
                self.sunrise_latched = true;
            }
        }
        if let Some(run) = self.light_run.as_mut() {
            match run.routine {
                LightRoutine::WindDown if now >= run.until => {
                    self.light_run = None;
                    if let Some(lamp) = manual.as_ref() {
                        *manual = Some(lamp.freeze(now).map_err(|e| e.to_string())?);
                    }
                    return Ok(true);
                }
                LightRoutine::Focus => {
                    let state = self
                        .saved
                        .alerts
                        .iter()
                        .find(|a| Some(a.id) == run.timer)
                        .map(|a| a.state.as_str())
                        .unwrap_or("cancelled");
                    if state != "pending" && !run.ending {
                        *manual = Some(LampProgram::from_color(Rgbw8::new(40, 20, 4, 210)));
                        run.ending = true;
                    }
                    if !["pending", "ringing"].contains(&state) {
                        self.light_run = None;
                    }
                }
                _ => {}
            }
            return Ok(false);
        }
        if self.sunrise.is_none()
            && available
            && let Some(alert) = self
                .saved
                .alerts
                .iter()
                .filter(|a| {
                    a.state == "pending"
                        && a.kind == "alarm"
                        && !self.sunrises_seen.contains(&a.id)
                        && a.sunrise_minutes.is_some_and(|minutes| {
                            wall >= a.due_unix - f64::from(minutes) * 60. && wall < a.due_unix
                        })
                })
                .min_by(|a, b| a.due_unix.total_cmp(&b.due_unix))
        {
            let previous_lamp = manual.clone();
            let duration = f64::from(alert.sunrise_minutes.unwrap()) * 60.;
            let start = now - (wall - (alert.due_unix - duration));
            let dim = LampProgram::from_color(Rgbw8::new(255, 100, 0, 80))
                .updated_at(
                    serde_json::from_value(json!({"brightness":0.03})).unwrap(),
                    now,
                )
                .map_err(|e| e.to_string())?;
            *manual=Some(dim.updated_at(serde_json::from_value(json!({"brightness":1.,"effect":"solid","colors":[[40,20,4,210]],"transition_seconds":duration})).unwrap(),start).map_err(|e|e.to_string())?);
            self.sunrises_seen.insert(alert.id);
            self.sunrise = Some(Sunrise {
                id: alert.id,
                previous_lamp,
            });
        }
        self.sunrises_seen
            .retain(|id| self.saved.alerts.iter().any(|a| a.id == *id));
        Ok(false)
    }
    pub fn ringing(&self) -> bool {
        self.ring_deadline.is_some()
    }
    pub fn status(&self, wall: f64, now: f64) -> Value {
        let mut alerts = self.saved.alerts.clone();
        for alert in &mut alerts {
            if alert.state == "pending"
                && let Some(at) = self.timer_deadlines.get(&alert.id)
            {
                alert.due_unix = wall + at - now;
            }
        }
        let announcements: Vec<_> = alerts.iter().filter_map(|alert| {
            if alert.announce_after.is_some_and(|at| now < self.announcement_deadlines.get(&alert.id).copied().unwrap_or(now + at - wall)) { return None; }
            let text = if alert.dismissal_notice {
                format!("That was your {} alarm: {}", local_clock(alert.due_unix), alert.label)
            } else if alert.state == "announcing" && alert.announced_at.is_none() {
                if wall - alert.due_unix > 300. { format!("Earlier reminder, from {}: {}", local_clock(alert.due_unix), alert.label) }
                else { format!("Reminder: {}", alert.label) }
            } else { return None; };
            Some(json!({"id":alert.id,"text":text,"attempt":u16::from(alert.announcements) * 4 + u16::from(alert.announce_failures),"purpose":if alert.dismissal_notice && alert.briefing {"briefing"} else {"announcement"}}))
        }).collect();
        json!({"mode":self.mode(), "now_unix":wall, "alerts":alerts, "announcements":announcements, "ringing":self.ringing(),
            "sounds":self.saved.sounds, "available_sounds":AlertSound::catalog(), "ring_sound":self.saved.ring_sound,
            "light_routine":self.light_run.as_ref().map(|run|json!({"routine":run.routine,"remaining_seconds":(run.until-now).max(0.),"timer_id":run.timer,"ending":run.ending})), "sunrise_alert":self.sunrise.as_ref().map(|s|s.id),
            "remaining_ring_seconds":self.ring_deadline.map(|at|(at-now).max(0.)), "error":self.error})
    }
    fn commit(&mut self, saved: Saved) -> Result<(), String> {
        if let Some(path) = &self.path {
            if saved.sounds != self.saved.sounds
                || (saved.ring_sound.is_some()
                    && (saved.ring_sound != self.saved.ring_sound
                        || saved.ring_until_unix != self.saved.ring_until_unix))
            {
                // Store the chosen sound before firing. Its window key prevents a
                // partial write from changing another alert's sound after restart.
                write_saved(
                    &path.with_extension("sounds.json"),
                    &SavedSounds {
                        settings: saved.sounds.clone(),
                        ring_until_unix: saved.ring_until_unix,
                        ring_sound: saved.ring_sound,
                    },
                )?;
            }
            // Sound-only changes leave the alert file untouched.
            if serde_json::to_value(&saved).map_err(|e| e.to_string())?
                != serde_json::to_value(&self.saved).map_err(|e| e.to_string())?
            {
                write_saved(path, &saved)?;
            }
        }
        self.saved = saved;
        self.reminder_deadlines.retain(|id, _| {
            self.saved
                .alerts
                .iter()
                .any(|a| a.id == *id && a.state == "announcing" && a.announced_at.is_some())
        });
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
            Request::StartLightRoutine { .. } | Request::StopLightRoutine => {
                return Err("Light routines require the lamp dispatcher".into());
            }
            Request::List => return Ok(self.status(wall, now)),
            Request::Stop => {
                self.stop_at("dismissed", wall, now, audio)?;
                return Ok(self.status(wall, now));
            }
            Request::DismissAnnouncement { id } => {
                let alert = saved
                    .alerts
                    .iter_mut()
                    .find(|a| a.id == id)
                    .ok_or("Unknown alert ID")?;
                alert.dismissal_notice = false;
                if alert.state == "announcing" {
                    alert.state = "dismissed".into();
                }
                alert.announce_after = None;
            }
            Request::Announced { id } => {
                let alert = saved
                    .alerts
                    .iter_mut()
                    .find(|a| a.id == id)
                    .ok_or("Unknown alert ID")?;
                alert.announce_after = None;
                if alert.dismissal_notice {
                    alert.dismissal_notice = false;
                } else if alert.state == "announcing" && alert.announced_at.is_none() {
                    alert.announcements += 1;
                    alert.announced_at = Some(wall);
                }
            }
            Request::AnnounceFailed { id } => {
                let alert = saved
                    .alerts
                    .iter_mut()
                    .find(|a| a.id == id)
                    .ok_or("Unknown alert ID")?;
                let ready = alert.dismissal_notice
                    || (alert.state == "announcing" && alert.announced_at.is_none());
                let waiting = alert.announce_after.is_some_and(|at| {
                    now < self
                        .announcement_deadlines
                        .get(&id)
                        .copied()
                        .unwrap_or(now + at - wall)
                });
                if ready && !waiting {
                    alert.announce_failures += 1;
                    if alert.announce_failures >= 3 {
                        if alert.state == "announcing" {
                            alert.state = "missed".into();
                        }
                        alert.dismissal_notice = false;
                        alert.announce_after = None;
                    } else {
                        alert.announce_after = Some(
                            wall + if alert.announce_failures == 1 {
                                30.
                            } else {
                                120.
                            },
                        );
                    }
                }
            }
            Request::Snooze { minutes } => {
                if active_slots(&saved) + 1 > 32 {
                    return Err("At most 32 active alerts are allowed".into());
                }
                let minutes = minutes.unwrap_or(9);
                if !(1..=60).contains(&minutes) {
                    return Err("Snooze must be 1–60 minutes".into());
                }
                let previous = saved
                    .last_dismissed
                    .as_ref()
                    .filter(|d| wall >= d.at && wall - d.at <= 120.)
                    .ok_or("No alert was dismissed in the last two minutes")?;
                let mut alert = previous.alert.clone();
                saved.next_id += 1;
                alert.id = saved.next_id;
                alert.series = alert.id;
                alert.briefing = false;
                alert.snoozed = true;
                alert.due_unix = wall + f64::from(minutes) * 60.;
                alert.state = "pending".into();
                alert.relative = true;
                alert.sunrise_minutes = None;
                alert.repeat = None;
                alert.announcements = 0;
                alert.announced_at = None;
                alert.announce_failures = 0;
                alert.announce_after = None;
                alert.dismissal_notice = false;
                created = Some(alert.clone());
                saved.alerts.push(alert);
                saved.last_dismissed = None;
            }
            Request::SetMode { mode } => saved.mode = mode,
            Request::SetSound { kind, sound } => match kind {
                AlertKind::Alarm => saved.sounds.alarm = sound,
                AlertKind::Timer => saved.sounds.timer = sound,
            },
            Request::Cancel { id, series } => {
                let alert = saved
                    .alerts
                    .iter_mut()
                    .find(|a| a.id == id)
                    .ok_or("Unknown alert ID")?;
                if !series && !["pending", "ringing", "announcing"].contains(&alert.state.as_str())
                {
                    return Err("Alert already finished".into());
                }
                let series_id = alert.series;
                alert.state = "cancelled".into();
                alert.dismissal_notice = false;
                if series && series_id != 0 {
                    for sibling in &mut saved.alerts {
                        if sibling.series == series_id
                            && ["pending", "ringing", "announcing"]
                                .contains(&sibling.state.as_str())
                        {
                            sibling.state = "cancelled".into();
                        }
                    }
                }
            }
            request => {
                let (kind, due, label, repeat, zone, relative, briefing, sunrise_minutes) =
                    match request {
                        Request::Timer { seconds, label } => {
                            if !seconds.is_finite() || !(1.0..=604800.).contains(&seconds) {
                                return Err(
                                    "Timer must be between one second and seven days".into()
                                );
                            }
                            (
                                "timer",
                                wall + seconds,
                                label,
                                None,
                                None,
                                true,
                                false,
                                None,
                            )
                        }
                        Request::Alarm {
                            sunrise_minutes,
                            briefing,
                            due_unix,
                            label,
                            repeat,
                        } => {
                            if sunrise_minutes.is_some_and(|m| !(5..=60).contains(&m)) {
                                return Err("Sunrise must be 5–60 minutes".into());
                            }
                            if due_unix.is_some() == repeat.is_some() {
                                return Err("Choose exactly one alarm time or repeat".into());
                            }
                            let zone = repeat
                                .as_ref()
                                .map(|_| timezone().ok_or("Set the Pi IANA timezone for repeats"))
                                .transpose()?;
                            let due_unix = match &repeat {
                                Some(rule) => rule.next(wall, zone.as_deref().unwrap())?,
                                None => due_unix.unwrap(),
                            };
                            if !due_unix.is_finite()
                                || due_unix <= wall
                                || due_unix > wall + 366. * 86400.
                            {
                                return Err("Alarm must be in the next 366 days".into());
                            }
                            (
                                "alarm",
                                due_unix,
                                label,
                                repeat,
                                zone,
                                false,
                                briefing,
                                sunrise_minutes,
                            )
                        }
                        Request::Reminder {
                            label,
                            due_unix,
                            seconds,
                            repeat,
                        } => {
                            if usize::from(due_unix.is_some())
                                + usize::from(seconds.is_some())
                                + usize::from(repeat.is_some())
                                != 1
                            {
                                return Err("Choose exactly one reminder trigger".into());
                            }
                            let zone = repeat
                                .as_ref()
                                .map(|_| timezone().ok_or("Set the Pi IANA timezone for repeats"))
                                .transpose()?;
                            let due = if let Some(rule) = &repeat {
                                rule.next(wall, zone.as_deref().unwrap())?
                            } else if let Some(seconds) = seconds {
                                if !seconds.is_finite() || !(1.0..=604800.).contains(&seconds) {
                                    return Err(
                                        "Reminder duration must be one second to seven days".into(),
                                    );
                                }
                                wall + seconds
                            } else {
                                due_unix.unwrap()
                            };
                            if label.trim().is_empty()
                                || !due.is_finite()
                                || due <= wall
                                || due > wall + 366. * 86400.
                            {
                                return Err(
                                    "Reminder needs text and a future time within 366 days".into(),
                                );
                            }
                            (
                                "reminder",
                                due,
                                label,
                                repeat,
                                zone,
                                seconds.is_some(),
                                false,
                                None,
                            )
                        }
                        _ => unreachable!(),
                    };
                if label.chars().count() > if kind == "reminder" { 200 } else { 80 }
                    || label.chars().any(char::is_control)
                {
                    return Err("Use a label of at most 80 printable characters".into());
                }
                if active_slots(&saved) + if repeat.is_some() { 2 } else { 1 } > 32 {
                    return Err("At most 32 active alerts are allowed".into());
                }
                if saved.alerts.len() >= 64 {
                    let index = saved
                        .alerts
                        .iter()
                        .position(|a| {
                            !["pending", "ringing", "announcing"].contains(&a.state.as_str())
                                && !a.dismissal_notice
                        })
                        .ok_or("Alert store is full; acknowledge pending announcements")?;
                    saved.alerts.remove(index);
                }
                saved.next_id = saved.next_id.checked_add(1).ok_or("Alert IDs exhausted")?;
                let alert = Alert {
                    id: saved.next_id,
                    kind: kind.into(),
                    label,
                    due_unix: due,
                    state: "pending".into(),
                    repeat,
                    timezone: zone,
                    relative,
                    announcements: 0,
                    announced_at: None,
                    announce_failures: 0,
                    announce_after: None,
                    dismissal_notice: false,
                    series: saved.next_id,
                    briefing,
                    snoozed: false,
                    sunrise_minutes,
                };
                created = Some(alert.clone());
                saved.alerts.push(alert);
            }
        }
        trim_history(&mut saved)?;
        self.commit(saved)?;
        self.announcement_deadlines.retain(|_, at| *at > now);
        self.announcement_deadlines.retain(|id, _| {
            self.saved
                .alerts
                .iter()
                .any(|a| a.id == *id && a.announce_after.is_some())
        });
        for alert in &self.saved.alerts {
            if let Some(at) = alert.announce_after {
                self.announcement_deadlines
                    .entry(alert.id)
                    .or_insert(now + at - wall);
            }
            if alert.state == "announcing" && alert.announced_at.is_some() {
                self.reminder_deadlines
                    .entry(alert.id)
                    .or_insert(now + 120.);
            }
        }
        self.timer_deadlines.retain(|id, _| {
            self.saved
                .alerts
                .iter()
                .any(|a| a.id == *id && a.state == "pending")
        });
        if let Some(alert) = &created
            && (alert.kind == "timer" || alert.relative)
        {
            self.timer_deadlines
                .insert(alert.id, now + alert.due_unix - wall);
        }
        if self.ringing() && !self.saved.alerts.iter().any(|a| a.state == "ringing") {
            self.stop_at("dismissed", wall, now, audio)?;
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
            self.stop_at("expired", wall, now, audio)?;
        }
        let mut saved = self.saved.clone();
        let mut changed = false;
        let mut next_occurrences = Vec::new();
        let was_ringing = self.ringing();
        for alert in &mut saved.alerts {
            if alert.state == "announcing" {
                if let Some(at) = alert.announced_at
                    && now
                        >= self
                            .reminder_deadlines
                            .get(&alert.id)
                            .copied()
                            .unwrap_or(now + at + 120. - wall)
                {
                    changed = true;
                    if alert.announcements >= 2 {
                        alert.state = "missed".into();
                    } else {
                        alert.announced_at = None;
                        self.reminder_deadlines.remove(&alert.id);
                    }
                }
                continue;
            }
            if alert.state != "pending" {
                continue;
            }
            let late = if alert.kind == "timer" || alert.relative {
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
            if let Some(repeat) = &alert.repeat {
                let mut next = alert.clone();
                next.due_unix = repeat.next(
                    wall.max(alert.due_unix),
                    alert
                        .timezone
                        .as_deref()
                        .ok_or("Recurring alert has no zone")?,
                )?;
                next_occurrences.push(next);
            }
            if alert.kind == "reminder" {
                alert.state = if late > 12. * 3600. {
                    "missed"
                } else {
                    "announcing"
                }
                .into();
                continue;
            }
            if late >= ALERT_LIMIT_SECONDS {
                alert.state = "missed".into();
                continue;
            }
            alert.state = "ringing".into();
            saved.ring_sound.get_or_insert(if alert.kind == "timer" {
                saved.sounds.timer
            } else {
                saved.sounds.alarm
            });
            saved
                .ring_until_unix
                .get_or_insert(wall + ALERT_LIMIT_SECONDS - late);
        }
        if changed {
            for mut next in next_occurrences {
                saved.next_id = saved.next_id.checked_add(1).ok_or("Alert IDs exhausted")?;
                next.id = saved.next_id;
                saved.alerts.push(next);
            }
            trim_history(&mut saved)?;
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
    pub fn stop(
        &mut self,
        state: &str,
        now: f64,
        audio: &mut dyn AudioDevice,
    ) -> Result<(), String> {
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        self.stop_at(state, wall, now, audio)
    }
    pub fn stop_at(
        &mut self,
        state: &str,
        wall: f64,
        now: f64,
        audio: &mut dyn AudioDevice,
    ) -> Result<(), String> {
        // Always silence first, including when the disk has become unwritable.
        if self.audio_started {
            audio.stop().map_err(|e| e.to_string())?;
        }
        let was_ringing = self.ringing();
        self.audio_started = false;
        self.ring_deadline = None;
        let mut saved = self.saved.clone();
        saved.ring_until_unix = None;
        saved.ring_sound = None;
        for alert in &mut saved.alerts {
            // Give the speaker tail time to settle, preserving a longer retry backoff.
            if was_ringing && alert.state == "announcing" {
                let deadline = self
                    .announcement_deadlines
                    .get(&alert.id)
                    .copied()
                    .unwrap_or_else(|| now + alert.announce_after.unwrap_or(wall) - wall)
                    .max(now + 3.);
                alert.announce_after = Some(wall + deadline - now);
                self.announcement_deadlines.insert(alert.id, deadline);
            }
            if alert.state == "ringing" {
                if state == "dismissed" && ["alarm", "timer"].contains(&alert.kind.as_str()) {
                    saved.last_dismissed = Some(Dismissed {
                        alert: alert.clone(),
                        at: wall,
                    });
                    if alert.kind == "alarm"
                        && !alert.snoozed
                        && (!alert.label.is_empty() || alert.briefing)
                    {
                        alert.dismissal_notice = true;
                        if alert.briefing {
                            alert.announce_after = Some(wall + 3.);
                            self.announcement_deadlines.insert(alert.id, now + 3.);
                        }
                    }
                }
                alert.state = state.into();
            }
        }
        // Keep the in-memory dismissal even if persistence fails; expose the error.
        // Write before updating self.saved, so commit can compare the prior state.
        let result = self.commit(saved.clone());
        self.saved = saved;
        result
    }
    pub fn tick_audio(&mut self, now: f64, audio: &mut dyn AudioDevice) -> Result<(), String> {
        let Some(until) = self.ring_deadline else {
            return Ok(());
        };
        if now >= until {
            return self.stop("expired", now, audio);
        }
        let result = (|| {
            if !self.audio_started {
                audio.start_alarm_pcm()?;
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
                let pcm = self
                    .saved
                    .ring_sound
                    .unwrap_or_default()
                    .pcm(self.sample, 480);
                if !audio.queue_pcm(&pcm)? {
                    break;
                }
                self.sample += 480;
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.error = Some(format!("Alarm playback failed: {error}"));
            let _ = self.stop("failed", now, audio);
            return Err(self.error.clone().unwrap());
        }
        Ok(())
    }
}
fn local_clock(due: f64) -> String {
    chrono::DateTime::from_timestamp(due.floor() as i64, 0)
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%-I:%M %p")
                .to_string()
        })
        .unwrap_or_else(|| "the scheduled time".into())
}

fn active_slots(saved: &Saved) -> usize {
    saved
        .alerts
        .iter()
        .filter(|a| ["pending", "ringing", "announcing"].contains(&a.state.as_str()))
        .map(|a| {
            if a.state == "pending" && a.repeat.is_some() {
                2
            } else {
                1
            }
        })
        .sum()
}
fn trim_history(saved: &mut Saved) -> Result<(), String> {
    while saved.alerts.len() > 64 {
        let index = saved
            .alerts
            .iter()
            .position(|a| {
                !["pending", "ringing", "announcing"].contains(&a.state.as_str())
                    && !a.dismissal_notice
            })
            .ok_or("Alert store is full")?;
        saved.alerts.remove(index);
    }
    Ok(())
}
// Same discovery order as agent/src/tools/routines.rs; keep the runtime
// independent of agent/network dependencies.
fn timezone() -> Option<String> {
    std::env::var("TZ")
        .ok()
        .filter(|s| s.parse::<chrono_tz::Tz>().is_ok())
        .or_else(|| {
            fs::read_link("/etc/localtime")
                .ok()?
                .to_str()?
                .split_once("zoneinfo/")
                .map(|(_, s)| s.to_owned())
                .filter(|s| s.parse::<chrono_tz::Tz>().is_ok())
        })
        .or_else(|| {
            fs::read_to_string("/etc/timezone")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| s.parse::<chrono_tz::Tz>().is_ok())
        })
}

fn write_saved(path: &std::path::Path, value: &impl Serialize) -> Result<(), String> {
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
    file.write_all(&serde_json::to_vec(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    // Avoid forcing an SD card flush in the motion loop. Sudden power loss can
    // lose a recent write; atomic replacement protects service restarts.
    fs::rename(&temporary, path).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damaged_saves_are_moved_aside_and_the_runtime_starts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        fs::write(
            &path,
            r#"{"mode":"lamp","next_id":1,"alerts":[],"ring_until_unix":null}"#,
        )
        .unwrap();
        fs::write(path.with_extension("sounds.json"), "").unwrap();
        let r = Routines::load(Some(path.clone()), 100., 0.).unwrap();
        assert_eq!(r.mode(), UserMode::Lamp);
        assert!(r.error.as_deref().unwrap().contains("alarm sound settings"));
        assert!(path.exists());
        assert!(dir.path().join("routines.sounds.json.damaged").exists());
        fs::write(&path, "").unwrap();
        let r = Routines::load(Some(path.clone()), 100., 0.).unwrap();
        assert_eq!(r.mode(), UserMode::Idle);
        assert!(r.error.is_some());
        assert!(!path.exists());
        assert!(dir.path().join("routines.json.damaged").exists());
    }
    use crate::RecordingAudioDevice;
    fn at(text: &str) -> f64 {
        chrono::DateTime::parse_from_rfc3339(text)
            .unwrap()
            .timestamp() as f64
    }
    fn request(
        r: &mut Routines,
        body: Value,
        wall: f64,
        now: f64,
        audio: &mut RecordingAudioDevice,
    ) -> Value {
        r.request(serde_json::from_value(body).unwrap(), wall, now, audio)
            .unwrap()
    }
    #[test]
    fn recurrence_uses_local_days_across_both_london_dst_changes() {
        let rule = Repeat {
            days: vec!["sun".into()],
            time: "09:00".into(),
        };
        assert_eq!(
            rule.next(at("2026-03-28T10:00:00Z"), "Europe/London")
                .unwrap(),
            at("2026-03-29T08:00:00Z")
        );
        assert_eq!(
            rule.next(at("2026-10-24T10:00:00Z"), "Europe/London")
                .unwrap(),
            at("2026-10-25T09:00:00Z")
        );
        let fold = Repeat {
            time: "01:30".into(),
            ..rule.clone()
        };
        assert_eq!(
            fold.next(at("2026-10-24T10:00:00Z"), "Europe/London")
                .unwrap(),
            at("2026-10-25T00:30:00Z")
        );
        assert_eq!(
            fold.next(at("2026-03-28T10:00:00Z"), "Europe/London")
                .unwrap(),
            at("2026-03-29T01:00:00Z")
        );
    }
    #[test]
    fn protected_dismissal_notices_are_not_evicted_to_make_room() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        let alerts:Vec<_>=(1..=64).map(|id|json!({"id":id,"kind":"alarm","label":"gym","due_unix":90.,"state":"dismissed","dismissal_notice":true})).collect();
        fs::write(
            &path,
            json!({"mode":"idle","next_id":64,"ring_until_unix":null,"alerts":alerts}).to_string(),
        )
        .unwrap();
        let mut r = Routines::load(Some(path), 100., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        assert!(
            r.request(
                Request::Timer {
                    seconds: 30.,
                    label: "tea".into()
                },
                100.,
                0.,
                &mut audio
            )
            .is_err()
        );
        assert_eq!(r.saved.alerts.len(), 64);
        assert!(r.saved.alerts.iter().all(|a| a.dismissal_notice));
    }
    #[test]
    fn snooze_cannot_exceed_the_active_alert_limit() {
        let mut r = Routines::load(None, 100., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        request(
            &mut r,
            json!({"action":"timer","label":"tea","seconds":1}),
            100.,
            0.,
            &mut audio,
        );
        r.due(101., 1., &mut audio).unwrap();
        r.stop_at("dismissed", 102., 2., &mut audio).unwrap();
        for i in 0..32 {
            request(
                &mut r,
                json!({"action":"timer","label":format!("timer {i}"),"seconds":1000}),
                102.,
                2.,
                &mut audio,
            );
        }
        assert!(
            r.request(Request::Snooze { minutes: Some(9) }, 103., 3., &mut audio)
                .is_err()
        );
        assert_eq!(active_slots(&r.saved), 32);
    }
    #[test]
    fn ring_dismissal_preserves_due_reminder_and_snoozes_the_alarm() {
        let mut r = Routines::load(None, 100., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        request(
            &mut r,
            json!({"action":"alarm","label":"wake","due_unix":101.}),
            100.,
            0.,
            &mut audio,
        );
        request(
            &mut r,
            json!({"action":"reminder","label":"stretch","due_unix":102.}),
            100.,
            0.,
            &mut audio,
        );
        r.due(101., 1., &mut audio).unwrap();
        r.due(102., 2., &mut audio).unwrap();
        r.request(Request::Stop, 103., 3., &mut audio).unwrap();
        assert_eq!(r.saved.alerts[1].state, "announcing");
        assert_eq!(r.saved.alerts[1].announcements, 0);
        assert!(
            r.status(105.9, 5.9)["announcements"]
                .as_array()
                .unwrap()
                .iter()
                .all(|a| a["id"] != 2)
        );
        assert!(
            r.status(106., 6.)["announcements"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["id"] == 2 && a["text"] == "Reminder: stretch")
        );
        let snoozed = r
            .request(Request::Snooze { minutes: None }, 107., 7., &mut audio)
            .unwrap();
        assert_eq!(snoozed["alert"]["kind"], "alarm");
        assert_eq!(snoozed["alert"]["label"], "wake");
    }
    #[test]
    fn ring_dismissal_preserves_longer_reminder_backoff_and_unrelated_notices() {
        let mut r = Routines::load(None, 100., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        request(
            &mut r,
            json!({"action":"alarm","label":"wake","due_unix":101.}),
            100.,
            0.,
            &mut audio,
        );
        request(
            &mut r,
            json!({"action":"reminder","label":"stretch","due_unix":101.}),
            100.,
            0.,
            &mut audio,
        );
        r.due(101., 1., &mut audio).unwrap();
        r.request(Request::AnnounceFailed { id: 2 }, 101., 1., &mut audio)
            .unwrap();
        r.request(Request::Stop, 103., 3., &mut audio).unwrap();
        assert_eq!(r.saved.alerts[1].announce_after, Some(131.));
        assert_eq!(r.saved.alerts[1].announce_failures, 1);
        r.request(Request::Stop, 104., 4., &mut audio).unwrap();
        assert!(r.saved.alerts[0].dismissal_notice);
        assert_eq!(r.saved.alerts[1].state, "announcing");
        // Wall-clock changes cannot shorten the live monotonic retry deadline.
        assert!(
            r.status(500., 30.)["announcements"]
                .as_array()
                .unwrap()
                .iter()
                .all(|a| a["id"] != 2)
        );
        assert!(
            r.status(501., 31.)["announcements"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["id"] == 2)
        );
        r.request(
            Request::DismissAnnouncement { id: 1 },
            501.,
            31.,
            &mut audio,
        )
        .unwrap();
        assert!(!r.saved.alerts[0].dismissal_notice);
        assert_eq!(r.saved.alerts[1].state, "announcing");
    }
    #[test]
    fn cancelling_a_ringing_alarm_preserves_the_unspoken_reminder() {
        let mut r = Routines::load(None, 100., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        request(
            &mut r,
            json!({"action":"alarm","label":"wake","due_unix":101.}),
            100.,
            0.,
            &mut audio,
        );
        request(
            &mut r,
            json!({"action":"reminder","label":"stretch","due_unix":102.}),
            100.,
            0.,
            &mut audio,
        );
        r.due(101., 1., &mut audio).unwrap();
        r.due(102., 2., &mut audio).unwrap();
        r.request(
            Request::Cancel {
                id: 1,
                series: false,
            },
            103.,
            3.,
            &mut audio,
        )
        .unwrap();
        assert!(!r.ringing());
        assert_eq!(r.saved.alerts[0].state, "cancelled");
        assert_eq!(r.saved.alerts[1].state, "announcing");
        assert_eq!(r.saved.alerts[1].announced_at, None);
        assert_eq!(r.saved.alerts[1].announce_failures, 0);
        assert!(r.saved.last_dismissed.is_none());
        assert_eq!(r.status(106., 6.)["announcements"][0]["id"], 2);
    }
    #[test]
    fn wake_during_a_briefing_clears_the_persisted_dismissal_notice() {
        let mut r = Routines::load(None, 100., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        request(
            &mut r,
            json!({"action":"alarm","label":"gym","due_unix":101.,"briefing":true}),
            100.,
            0.,
            &mut audio,
        );
        r.due(101., 1., &mut audio).unwrap();
        r.stop_at("dismissed", 102., 2., &mut audio).unwrap();
        assert_eq!(
            r.status(105., 5.)["announcements"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        r.request(Request::DismissAnnouncement { id: 1 }, 106., 6., &mut audio)
            .unwrap();
        assert!(
            r.status(106., 6.)["announcements"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn sunrise_schedules_once_and_cancellation_restores_the_previous_lamp() {
        let mut r = Routines::load(None, 100., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        request(
            &mut r,
            json!({"action":"alarm","label":"wake","due_unix":1000.,"sunrise_minutes":5}),
            100.,
            0.,
            &mut audio,
        );
        let mut manual = None;
        r.light_tick(699., 599., &mut manual, true).unwrap();
        assert!(!r.sunrise_active());
        r.light_tick(700., 600., &mut manual, true).unwrap();
        assert!(r.sunrise_active());
        assert_eq!(
            manual.as_ref().unwrap().status_at(600.)["colors"][0],
            json!([255, 100, 0, 80])
        );
        let half = manual.as_ref().unwrap().status_at(750.);
        assert!((half["brightness"].as_f64().unwrap() - 51.5).abs() < 0.001);
        request(
            &mut r,
            json!({"action":"cancel","id":1}),
            850.,
            750.,
            &mut audio,
        );
        r.light_tick(850., 750., &mut manual, true).unwrap();
        assert!(!r.sunrise_active());
        assert!(manual.is_none());
        request(
            &mut r,
            json!({"action":"alarm","label":"another","due_unix":1200.,"sunrise_minutes":5}),
            850.,
            750.,
            &mut audio,
        );
        r.light_tick(900., 800., &mut manual, true).unwrap();
        assert!(r.sunrise_active());
        r.stop_light(900., 800., &mut manual, &mut audio).unwrap();
        r.light_tick(901., 801., &mut manual, true).unwrap();
        assert!(!r.sunrise_active());
    }
    #[test]
    fn dismissed_sunrise_keeps_the_end_light_until_a_lamp_or_character_change() {
        let mut r = Routines::load(None, 100., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        let mut manual = None;
        request(
            &mut r,
            json!({"action":"alarm","label":"wake","due_unix":1000.,"sunrise_minutes":5}),
            100.,
            0.,
            &mut audio,
        );
        r.light_tick(700., 600., &mut manual, true).unwrap();
        r.due(1000., 900., &mut audio).unwrap();
        r.stop_at("dismissed", 1001., 901., &mut audio).unwrap();
        r.light_tick(1001., 901., &mut manual, true).unwrap();
        assert!(r.sunrise_active()); // The mechanical rest darkness mask stays released.
        assert_eq!(manual.as_ref().unwrap().status()["brightness"], 100.);
        assert_eq!(
            manual.as_ref().unwrap().status()["colors"][0],
            json!([40, 20, 4, 210])
        );
        r.light_tick(2000., 1900., &mut manual, true).unwrap();
        assert!(r.sunrise_active());
        r.stop_light(2001., 1901., &mut manual, &mut audio).unwrap();
        assert!(!r.sunrise_active());
    }
    #[test]
    fn focus_uses_the_timer_and_restores_mode_after_ring_and_wind_down_completes_once() {
        let mut r = Routines::load(None, 100., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        let mut manual = None;
        r.start_light(LightRoutine::Focus, 5, 100., 0., &mut manual, &mut audio)
            .unwrap();
        assert_eq!(r.mode(), UserMode::Lamp);
        assert_eq!(r.saved.mode, UserMode::Idle);
        assert_eq!(r.saved.alerts[0].label, "Focus session");
        assert_eq!(
            manual.as_ref().unwrap().status()["colors"][0],
            json!([150, 190, 255, 110])
        );
        r.due(400., 300., &mut audio).unwrap();
        r.light_tick(400., 300., &mut manual, true).unwrap();
        assert!(r.ringing());
        assert_eq!(r.mode(), UserMode::Lamp);
        assert_eq!(
            manual.as_ref().unwrap().status()["colors"][0],
            json!([40, 20, 4, 210])
        );
        r.stop_at("dismissed", 401., 301., &mut audio).unwrap();
        r.light_tick(401., 301., &mut manual, true).unwrap();
        assert_eq!(r.mode(), UserMode::Idle);
        r.start_light(LightRoutine::Focus, 5, 402., 302., &mut manual, &mut audio)
            .unwrap();
        r.stop_light(403., 303., &mut manual, &mut audio).unwrap();
        assert_eq!(r.saved.alerts[1].state, "cancelled");
        assert_eq!(r.mode(), UserMode::Idle);
        r.start_light(
            LightRoutine::WindDown,
            10,
            404.,
            304.,
            &mut manual,
            &mut audio,
        )
        .unwrap();
        assert!(!r.light_tick(1003., 903., &mut manual, true).unwrap());
        assert!(r.light_tick(1004., 904., &mut manual, true).unwrap());
        assert!(!r.light_tick(1005., 905., &mut manual, true).unwrap());
        assert_eq!(manual.as_ref().unwrap().status_at(904.)["brightness"], 8.);
    }
    #[test]
    fn briefing_dismissal_survives_restart_and_acknowledges_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        let mut r = Routines::load(Some(path.clone()), 100., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        request(
            &mut r,
            json!({"action":"alarm","label":"gym","due_unix":101.,"briefing":true}),
            100.,
            0.,
            &mut audio,
        );
        r.due(101., 1., &mut audio).unwrap();
        r.stop_at("dismissed", 102., 2., &mut audio).unwrap();
        let mut r = Routines::load(Some(path), 103., 3.).unwrap();
        assert!(
            r.status(103., 3.)["announcements"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(r.saved.last_dismissed.as_ref().unwrap().at, 102.);
        assert_eq!(
            r.status(105., 5.)["announcements"][0]["purpose"],
            "briefing"
        );
        request(
            &mut r,
            json!({"action":"announced","id":1}),
            103.,
            3.,
            &mut audio,
        );
        assert!(
            r.status(103., 3.)["announcements"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        r.request(Request::Announced { id: 1 }, 103., 3., &mut audio)
            .unwrap();
        assert!(
            r.status(103., 3.)["announcements"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn recurring_fire_and_next_occurrence_are_persisted_together() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        let wall = at("2026-03-22T09:00:00Z");
        // Construct persisted state to choose a zone without mutating process TZ.
        fs::write(&path, json!({"mode":"idle","next_id":1,"ring_until_unix":null,"alerts":[{"id":1,"kind":"alarm","label":"gym","due_unix":wall,"state":"pending","repeat":{"days":["sun"],"time":"09:00"},"timezone":"Europe/London","series":1}]}).to_string()).unwrap();
        let mut r = Routines::load(Some(path.clone()), wall, 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        r.due(wall, 0., &mut audio).unwrap();
        let restored = Routines::load(Some(path), wall + 10., 0.).unwrap();
        assert_eq!(restored.saved.alerts[0].state, "ringing");
        assert_eq!(
            restored.saved.alerts[1].due_unix,
            at("2026-03-29T08:00:00Z")
        );
        assert_eq!(restored.saved.alerts[1].state, "pending");
    }
    #[test]
    fn reminders_are_late_at_eleven_hours_missed_at_thirteen_and_repeat_only_once() {
        for (late, expected) in [(11. * 3600., "announcing"), (13. * 3600., "missed")] {
            let mut r = Routines::load(None, 100., 0.).unwrap();
            let mut audio = RecordingAudioDevice::default();
            request(
                &mut r,
                json!({"action":"reminder","label":"bins","due_unix":101.}),
                100.,
                0.,
                &mut audio,
            );
            r.due(101. + late, late + 1., &mut audio).unwrap();
            assert_eq!(r.saved.alerts[0].state, expected);
            if expected == "announcing" {
                assert!(
                    r.status(101. + late, late + 1.)["announcements"][0]["text"]
                        .as_str()
                        .unwrap()
                        .starts_with("Earlier reminder")
                );
                request(
                    &mut r,
                    json!({"action":"announced","id":1}),
                    101. + late,
                    late + 1.,
                    &mut audio,
                );
                r.due(221. + late, late + 121., &mut audio).unwrap();
                assert_eq!(r.saved.alerts[0].announced_at, None);
                request(
                    &mut r,
                    json!({"action":"announced","id":1}),
                    221. + late,
                    late + 121.,
                    &mut audio,
                );
                r.due(341. + late, late + 241., &mut audio).unwrap();
                assert_eq!(r.saved.alerts[0].state, "missed");
            }
        }
    }
    #[test]
    fn london_spring_gap_rings_at_two_local_instead_of_skipping_the_day() {
        let rule = Repeat {
            days: vec!["sun".into()],
            time: "01:30".into(),
        };
        let due = rule
            .next(at("2026-03-28T10:00:00Z"), "Europe/London")
            .unwrap();
        assert_eq!(due, at("2026-03-29T02:00:00+01:00"));
        let mut r = Routines::load(None, due - 1., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        request(
            &mut r,
            json!({"action":"alarm","label":"DST","due_unix":due}),
            due - 1.,
            0.,
            &mut audio,
        );
        assert!(r.due(due, 1., &mut audio).unwrap());
        assert!(r.ringing());
    }
    #[test]
    fn existing_routines_save_without_creating_backup_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        fs::write(
            &path,
            json!({"mode":"idle","next_id":2,"ring_until_unix":null,"alerts":[
                {"id":1,"kind":"timer","label":"tea","due_unix":160.,"state":"pending"},
                {"id":2,"kind":"alarm","label":"meeting","due_unix":300.,"state":"pending"}
            ]})
            .to_string(),
        )
        .unwrap();
        let mut r = Routines::load(Some(path.clone()), 100., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        let alerts = serde_json::to_value(&r.saved.alerts).unwrap();
        r.request(
            Request::SetMode {
                mode: UserMode::Lamp,
            },
            100.,
            0.,
            &mut audio,
        )
        .unwrap();

        let restored = Routines::load(Some(path.clone()), 101., 1.).unwrap();
        assert_eq!(restored.mode(), UserMode::Lamp);
        assert_eq!(
            serde_json::to_value(&restored.saved.alerts).unwrap(),
            alerts
        );
        assert_eq!(restored.timer_deadlines[&1], 60.);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn restart_cancels_light_routines_but_preserves_focus_timer_and_saved_mode() {
        for routine in [LightRoutine::WindDown, LightRoutine::Focus] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("routines.json");
            let mut r = Routines::load(Some(path.clone()), 100., 0.).unwrap();
            let mut audio = RecordingAudioDevice::default();
            let mut manual = None;
            r.request(
                Request::Timer {
                    seconds: 1000.,
                    label: "Existing".into(),
                },
                100.,
                0.,
                &mut audio,
            )
            .unwrap();
            r.start_light(routine, 10, 100., 0., &mut manual, &mut audio)
                .unwrap();
            assert!(r.light_run.is_some());
            let restored = Routines::load(Some(path), 200., 0.).unwrap();
            assert!(restored.light_run.is_none());
            assert_eq!(restored.mode(), UserMode::Idle);
            assert_eq!(
                restored.saved.alerts.len(),
                if routine == LightRoutine::Focus { 2 } else { 1 }
            );
            if routine == LightRoutine::Focus {
                assert_eq!(restored.saved.alerts[1].label, "Focus session");
                assert_eq!(restored.timer_deadlines[&2], 500.);
            }
        }
    }
    #[test]
    fn missed_reminders_release_their_acknowledgement_deadlines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        let mut r = Routines::load(Some(path.clone()), 100., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        request(
            &mut r,
            json!({"action":"reminder","label":"stretch","due_unix":101.}),
            100.,
            0.,
            &mut audio,
        );
        r.due(101., 1., &mut audio).unwrap();
        for (wall, now) in [(101., 1.), (221., 121.)] {
            request(
                &mut r,
                json!({"action":"announced","id":1}),
                wall,
                now,
                &mut audio,
            );
            assert_eq!(r.reminder_deadlines.len(), 1);
            r.due(wall + 120., now + 120., &mut audio).unwrap();
            assert!(r.reminder_deadlines.is_empty());
        }
        assert_eq!(r.saved.alerts[0].state, "missed");
        assert!(
            Routines::load(Some(path), 341., 241.)
                .unwrap()
                .reminder_deadlines
                .is_empty()
        );
    }
    #[test]
    fn failed_announcements_back_off_and_stop_after_three_even_after_restart() {
        for briefing in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("routines.json");
            let mut r = Routines::load(Some(path.clone()), 100., 0.).unwrap();
            let mut audio = RecordingAudioDevice::default();
            let create = if briefing {
                json!({"action":"alarm","label":"gym","due_unix":101.,"briefing":true})
            } else {
                json!({"action":"reminder","label":"stretch","due_unix":101.})
            };
            request(&mut r, create, 100., 0., &mut audio);
            r.due(101., 1., &mut audio).unwrap();
            if briefing {
                request(&mut r, json!({"action":"stop"}), 102., 2., &mut audio);
            }
            for (failure, wall) in [(1, 105.), (2, 135.), (3, 255.)] {
                assert_eq!(
                    r.status(wall, wall - 100.)["announcements"]
                        .as_array()
                        .unwrap()
                        .len(),
                    1
                );
                request(
                    &mut r,
                    json!({"action":"announce_failed","id":1}),
                    wall,
                    wall - 100.,
                    &mut audio,
                );
                // Polling at 5 Hz and duplicate failure reports cannot create another attempt.
                request(
                    &mut r,
                    json!({"action":"announce_failed","id":1}),
                    wall + 0.2,
                    wall - 99.8,
                    &mut audio,
                );
                assert_eq!(r.saved.alerts[0].announce_failures, failure);
                r = Routines::load(Some(path.clone()), wall + 1., wall - 99.).unwrap();
                assert!(
                    r.status(wall + 1., wall - 99.)["announcements"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
            }
            assert!(
                r.status(10000., 9900.)["announcements"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            assert!(!r.saved.alerts[0].dismissal_notice);
            if !briefing {
                assert_eq!(r.saved.alerts[0].state, "missed");
            }
        }
    }
    fn recurring_briefing() -> Routines {
        let mut r = Routines::load(None, 100., 0.).unwrap();
        let mut audio = RecordingAudioDevice::default();
        request(
            &mut r,
            json!({"action":"alarm","label":"gym","due_unix":101.,"briefing":true,"sunrise_minutes":5}),
            100.,
            0.,
            &mut audio,
        );
        r.saved.alerts[0].repeat = Some(Repeat {
            days: vec!["mon".into()],
            time: "07:00".into(),
        });
        r.saved.alerts[0].timezone = Some("Europe/London".into());
        r.due(101., 1., &mut audio).unwrap();
        r
    }
    #[test]
    fn snooze_is_independent_and_does_not_repeat_briefing_or_sunrise() {
        for cancel in [false, true] {
            let mut r = recurring_briefing();
            let mut audio = RecordingAudioDevice::default();
            request(&mut r, json!({"action":"stop"}), 102., 2., &mut audio);
            let result = request(
                &mut r,
                json!({"action":"snooze","minutes":1}),
                103.,
                3.,
                &mut audio,
            );
            let id = result["alert"]["id"].as_u64().unwrap();
            assert_eq!(result["alert"]["series"], id);
            assert_eq!(result["alert"]["briefing"], false);
            assert_eq!(result["alert"]["sunrise_minutes"], Value::Null);
            if cancel {
                request(
                    &mut r,
                    json!({"action":"cancel","id":id,"series":true}),
                    104.,
                    4.,
                    &mut audio,
                );
            } else {
                request(
                    &mut r,
                    json!({"action":"announced","id":1}),
                    105.,
                    5.,
                    &mut audio,
                );
                r.due(163., 63., &mut audio).unwrap();
                request(&mut r, json!({"action":"stop"}), 164., 64., &mut audio);
                assert!(
                    !r.saved
                        .alerts
                        .iter()
                        .find(|a| a.id == id)
                        .unwrap()
                        .dismissal_notice
                );
            }
            assert_eq!(r.saved.alerts[1].state, "pending");
        }
    }
    #[test]
    fn cancellation_only_follows_the_series_when_explicitly_requested() {
        for series in [false, true] {
            let mut r = recurring_briefing();
            let mut audio = RecordingAudioDevice::default();
            request(
                &mut r,
                json!({"action":"cancel","id":1,"series":series}),
                102.,
                2.,
                &mut audio,
            );
            assert_eq!(r.saved.alerts[0].state, "cancelled");
            assert_eq!(
                r.saved.alerts[1].state,
                if series { "cancelled" } else { "pending" }
            );
            assert!(!r.ringing());
        }
    }
    #[test]
    fn snooze_uses_the_two_minute_dismissal_window_and_relative_deadline() {
        for delay in [119., 121.] {
            let mut r = Routines::load(None, 100., 0.).unwrap();
            let mut audio = RecordingAudioDevice::default();
            request(
                &mut r,
                json!({"action":"alarm","label":"gym","due_unix":101.}),
                100.,
                0.,
                &mut audio,
            );
            r.due(101., 1., &mut audio).unwrap();
            request(&mut r, json!({"action":"stop"}), 102., 2., &mut audio);
            let snooze = r.request(
                Request::Snooze { minutes: None },
                102. + delay,
                2. + delay,
                &mut audio,
            );
            assert_eq!(snooze.is_ok(), delay < 120.);
            if delay < 120. {
                let alert = &r.saved.alerts[1];
                assert_eq!(alert.kind, "alarm");
                assert_eq!(alert.label, "gym");
                assert_eq!(r.timer_deadlines[&alert.id], 2. + delay + 540.);
            }
        }
    }
    #[test]
    fn old_state_keeps_mode_alerts_and_timer_deadlines_when_sounds_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        fs::write(&path, r#"{"mode":"lamp","next_id":1,"alerts":[{"id":1,"kind":"timer","label":"Tea","due_unix":110,"state":"pending"}],"ring_until_unix":null}"#).unwrap();
        let original = fs::read(&path).unwrap();
        let mut audio = RecordingAudioDevice::blocking();
        let mut r = Routines::load(Some(path.clone()), 100., 0.).unwrap();
        assert_eq!(
            r.status(100., 0.)["sounds"],
            json!({"alarm":"two_tone","timer":"two_tone"})
        );
        for (kind, sound) in [
            (AlertKind::Alarm, AlertSound::ClubAlarm),
            (AlertKind::Timer, AlertSound::FunnyAlarm),
        ] {
            r.request(Request::SetSound { kind, sound }, 1000., 5., &mut audio)
                .unwrap();
        }
        assert_eq!(r.timer_deadlines[&1], 10.);
        assert_eq!(fs::read(&path).unwrap(), original);
        let restored = Routines::load(Some(path), 100., 0.).unwrap();
        assert_eq!(restored.mode(), UserMode::Lamp);
        assert_eq!(restored.saved.alerts[0].label, "Tea");
        assert_eq!(restored.saved.alerts[0].due_unix, 110.);
        assert_eq!(
            restored.status(100., 0.)["sounds"],
            json!({"alarm":"club_alarm","timer":"funny_alarm"})
        );
        for body in [
            r#"{"action":"set_sound","kind":"alarm","sound":"/tmp/file.mp3"}"#,
            r#"{"action":"set_sound","kind":"scene","sound":"two_tone"}"#,
        ] {
            assert!(serde_json::from_str::<Request>(body).is_err());
        }
    }

    #[test]
    fn sound_is_chosen_at_firing_and_survives_preference_changes_overlap_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        let mut audio = RecordingAudioDevice::blocking();
        let mut r = Routines::load(Some(path.clone()), 100., 0.).unwrap();
        r.request(
            Request::Timer {
                seconds: 1.,
                label: "Tea".into(),
            },
            100.,
            0.,
            &mut audio,
        )
        .unwrap();
        r.request(
            Request::Alarm {
                sunrise_minutes: None,
                briefing: false,
                due_unix: Some(102.),
                repeat: None,
                label: "Morning".into(),
            },
            100.,
            0.,
            &mut audio,
        )
        .unwrap();
        r.request(
            Request::Alarm {
                sunrise_minutes: None,
                briefing: false,
                due_unix: Some(500.),
                repeat: None,
                label: "Later".into(),
            },
            100.,
            0.,
            &mut audio,
        )
        .unwrap();
        r.request(
            Request::SetSound {
                kind: AlertKind::Timer,
                sound: AlertSound::FunnyAlarm,
            },
            100.,
            0.,
            &mut audio,
        )
        .unwrap();
        r.request(
            Request::SetSound {
                kind: AlertKind::Alarm,
                sound: AlertSound::ClubAlarm,
            },
            100.,
            0.,
            &mut audio,
        )
        .unwrap();
        assert!(r.due(101., 1., &mut audio).unwrap());
        r.tick_audio(1., &mut audio).unwrap();
        r.request(
            Request::SetSound {
                kind: AlertKind::Timer,
                sound: AlertSound::TwoTone,
            },
            101.,
            1.,
            &mut audio,
        )
        .unwrap();
        assert!(!r.due(102., 2., &mut audio).unwrap());
        audio.stop().unwrap();
        let mut r = Routines::load(Some(path), 151., 0.).unwrap();
        assert_eq!(r.saved.ring_sound, Some(AlertSound::FunnyAlarm));
        assert_eq!(r.status(151., 0.)["remaining_ring_seconds"], 250.);
        r.tick_audio(0., &mut audio).unwrap();
        assert!(audio.is_playing());
        r.tick_audio(250., &mut audio).unwrap();
        assert!(!audio.is_playing());
        assert_eq!(r.saved.ring_sound, None);
        assert!(r.due(500., 349., &mut audio).unwrap());
        assert_eq!(r.saved.ring_sound, Some(AlertSound::ClubAlarm));
        r.tick_audio(349., &mut audio).unwrap();
        r.request(Request::Stop, 500., 349., &mut audio).unwrap();
        assert!(!audio.is_playing());
    }

    #[test]
    fn stale_sound_cache_cannot_change_an_alert_created_by_an_older_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        fs::write(&path, r#"{"mode":"idle","next_id":1,"alerts":[{"id":1,"kind":"timer","label":"Tea","due_unix":100,"state":"ringing"}],"ring_until_unix":400}"#).unwrap();
        write_saved(
            &path.with_extension("sounds.json"),
            &SavedSounds {
                settings: SoundSettings {
                    alarm: AlertSound::ClubAlarm,
                    timer: AlertSound::FunnyAlarm,
                },
                ring_until_unix: Some(200.),
                ring_sound: Some(AlertSound::FunnyAlarm),
            },
        )
        .unwrap();
        let r = Routines::load(Some(path), 150., 0.).unwrap();
        assert_eq!(r.saved.ring_sound, Some(AlertSound::TwoTone));
        assert_eq!(r.saved.sounds.timer, AlertSound::FunnyAlarm);
        assert_eq!(r.status(150., 0.)["remaining_ring_seconds"], 250.);
    }
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
                    sunrise_minutes: None,
                    briefing: false,
                    due_unix: Some(due),
                    repeat: None,
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
                    sunrise_minutes: None,
                    briefing: false,
                    due_unix: Some(due),
                    repeat: None,
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
        r.request(
            Request::Cancel {
                id: 1,
                series: false,
            },
            100.,
            0.,
            &mut audio,
        )
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
        assert!(
            r.request(
                Request::SetSound {
                    kind: AlertKind::Timer,
                    sound: AlertSound::FunnyAlarm
                },
                100.,
                0.,
                &mut audio
            )
            .is_err()
        );
        assert_eq!(r.saved.sounds.timer, AlertSound::TwoTone);
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
        for _ in 0..32 {
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
        let pcm = AlertSound::TwoTone.pcm(0, 28800);
        let samples: Vec<_> = pcm
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert!(samples.iter().map(|v| v.abs()).max().unwrap() > 20000);
        assert!(samples.iter().all(|v| v.abs() < 22000));
        assert!(samples[11520..14400].iter().all(|v| *v == 0));
    }
}
