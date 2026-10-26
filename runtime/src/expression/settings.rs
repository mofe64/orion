use serde::Deserialize;
use std::{io::Read, path::Path};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DanceMode {
    Off,
    #[default]
    Sometimes,
    Always,
}
impl<'de> Deserialize<'de> for DanceMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Bool(true) => Ok(Self::Sometimes),
            serde_json::Value::Bool(false) => Ok(Self::Off),
            serde_json::Value::String(mode) => match mode.as_str() {
                "off" => Ok(Self::Off),
                "sometimes" => Ok(Self::Sometimes),
                "always" => Ok(Self::Always),
                _ => Err(serde::de::Error::custom("Choose off, sometimes or always")),
            },
            _ => Err(serde::de::Error::custom("Invalid dance setting")),
        }
    }
}

/// How much Orion looks around on its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Curiosity {
    Shy,
    #[default]
    Normal,
    Nosy,
}
/// How often a Lively Orion speaks on its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Chattiness {
    Reserved,
    #[default]
    Normal,
    Chatty,
}
/// Per-lamp character quirks from `personality.json`. Studio writes the file;
/// a missing or unknown value reads as normal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Quirks {
    pub curiosity: Curiosity,
    pub chattiness: Chattiness,
}
impl Quirks {
    pub fn load(path: &Path) -> Self {
        let Some(value) = read_json(path) else {
            return Self::default();
        };
        let quirk = |name: &str| {
            value
                .get("quirks")
                .and_then(|quirks| quirks.get(name))
                .cloned()
        };
        Self {
            curiosity: quirk("curiosity")
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or_default(),
            chattiness: quirk("chattiness")
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or_default(),
        }
    }
}
/// Lively's everyday rules from `personality.json`, as the owner set them in
/// Studio. Times of day are minutes after midnight. Demo mode keeps its own
/// short timings; the switches apply to both.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Rules {
    /// No remarks from this time until `quiet_until`, except a goodnight.
    pub quiet_from: u32,
    pub quiet_until: u32,
    /// Remarks in one hour for a normal Orion; chattiness scales it.
    pub per_hour: u32,
    /// Shortest time between two remarks for a normal Orion.
    pub spacing_minutes: f64,
    /// No remark this soon after a conversation.
    pub after_talk_minutes: f64,
    /// The room must be empty this long before a welcome; also ends a visit.
    pub welcome_after_minutes: f64,
    pub break_after_minutes: f64,
    /// A late goodbye is a goodnight from this time until 05:00.
    pub goodnight_from: u32,
    /// Rest this long after a goodnight; `None` stays awake.
    pub sleep_after_goodnight_minutes: Option<f64>,
    pub wave_back: bool,
    pub light_care: bool,
    pub break_nudge: bool,
    pub goodnight: bool,
    pub follow_ups: bool,
    pub new_things: bool,
    pub looks: bool,
}
impl Default for Rules {
    fn default() -> Self {
        Self {
            quiet_from: 22 * 60,
            quiet_until: 7 * 60,
            per_hour: 4,
            spacing_minutes: 10.0,
            after_talk_minutes: 5.0,
            welcome_after_minutes: 10.0,
            break_after_minutes: 90.0,
            goodnight_from: 21 * 60,
            sleep_after_goodnight_minutes: Some(2.0),
            wave_back: true,
            light_care: true,
            break_nudge: true,
            goodnight: true,
            follow_ups: true,
            new_things: true,
            looks: true,
        }
    }
}
impl Rules {
    /// Reads the rules; a value out of range is brought into range, and a
    /// file that cannot be read gives the defaults.
    pub fn load(path: &Path) -> Self {
        read_json(path)
            .and_then(|value| value.get("rules").cloned())
            .and_then(|value| serde_json::from_value::<Self>(value).ok())
            .unwrap_or_default()
            .bounded()
    }
    fn bounded(mut self) -> Self {
        let minutes = |value: f64, low: f64, high: f64, default: f64| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                default
            }
        };
        let defaults = Self::default();
        self.quiet_from = self.quiet_from.min(24 * 60 - 1);
        self.quiet_until = self.quiet_until.min(24 * 60 - 1);
        self.per_hour = self.per_hour.clamp(1, 12);
        self.spacing_minutes = minutes(self.spacing_minutes, 1.0, 120.0, defaults.spacing_minutes);
        self.after_talk_minutes = minutes(
            self.after_talk_minutes,
            0.0,
            60.0,
            defaults.after_talk_minutes,
        );
        self.welcome_after_minutes = minutes(
            self.welcome_after_minutes,
            2.0,
            240.0,
            defaults.welcome_after_minutes,
        );
        self.break_after_minutes = minutes(
            self.break_after_minutes,
            20.0,
            240.0,
            defaults.break_after_minutes,
        );
        self.goodnight_from = self.goodnight_from.clamp(18 * 60, 24 * 60 - 1);
        self.sleep_after_goodnight_minutes = self
            .sleep_after_goodnight_minutes
            .map(|value| minutes(value, 1.0, 60.0, 2.0));
        self
    }
    /// Quiet hours at this local hour. Equal start and end means none.
    pub fn quiet_at(&self, hour: f64) -> bool {
        let minute = (hour * 60.0).rem_euclid(24.0 * 60.0);
        let (from, until) = (self.quiet_from as f64, self.quiet_until as f64);
        if from < until {
            (from..until).contains(&minute)
        } else if from > until {
            minute >= from || minute < until
        } else {
            false
        }
    }
    /// A goodbye at this local hour is a goodnight.
    pub fn late_at(&self, hour: f64) -> bool {
        let minute = (hour * 60.0).rem_euclid(24.0 * 60.0);
        minute >= self.goodnight_from as f64 || minute < 5.0 * 60.0
    }
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(8193)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= 8192)
        .then(|| serde_json::from_slice(&bytes).ok())
        .flatten()
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub(crate) struct ActSettings {
    pub mind_idle_rhythm: bool,
    pub listen_lean_in: bool,
    pub thinking_beats: bool,
    pub wake_sleep_acts: bool,
    pub posture_drift: bool,
    pub act_sounds: bool,
    pub move_to_music: DanceMode,
    #[serde(skip)]
    pub quirks: Quirks,
    #[serde(skip)]
    pub rules: Rules,
}
impl Default for ActSettings {
    fn default() -> Self {
        Self {
            mind_idle_rhythm: true,
            listen_lean_in: true,
            thinking_beats: true,
            wake_sleep_acts: true,
            posture_drift: false,
            act_sounds: true,
            move_to_music: DanceMode::Sometimes,
            quirks: Quirks::default(),
            rules: Rules::default(),
        }
    }
}
impl ActSettings {
    /// Reads `behaviours.json` and the quirks in `personality.json` beside it.
    pub fn load(path: &Path) -> Self {
        let mut settings: Self = read_json(path)
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        settings.quirks = Quirks::load(&path.with_file_name("personality.json"));
        settings.rules = Rules::load(&path.with_file_name("personality.json"));
        settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_preferences_preserve_defaults() {
        let value: ActSettings = serde_json::from_str(
            r#"{"posture_drift":true,"act_sounds":false,"object_glances":false}"#,
        )
        .unwrap();
        assert!(
            value.mind_idle_rhythm
                && value.listen_lean_in
                && value.thinking_beats
                && value.wake_sleep_acts
                && value.posture_drift
        );
        assert!(!value.act_sounds);
        assert_eq!(value.move_to_music, DanceMode::Sometimes);
    }
    #[test]
    fn music_preference_restores_from_the_shared_behaviour_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("behaviours.json");
        assert_eq!(ActSettings::load(&path).move_to_music, DanceMode::Sometimes);
        std::fs::write(&path, r#"{"move_to_music":false,"act_sounds":true}"#).unwrap();
        let settings = ActSettings::load(&path);
        assert_eq!(settings.move_to_music, DanceMode::Off);
        assert!(settings.act_sounds);
    }
    #[test]
    fn quirks_read_from_the_personality_file_and_default_to_normal() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("behaviours.json");
        assert_eq!(ActSettings::load(&path).quirks, Quirks::default());
        std::fs::write(
            directory.path().join("personality.json"),
            r#"{"quirks":{"curiosity":"nosy","chattiness":"loud","gestures":"big"}}"#,
        )
        .unwrap();
        let quirks = ActSettings::load(&path).quirks;
        assert_eq!(quirks.curiosity, Curiosity::Nosy);
        assert_eq!(quirks.chattiness, Chattiness::Normal);
    }

    #[test]
    fn rules_read_from_personality_stay_in_range_and_wrap_midnight() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("behaviours.json");
        assert_eq!(ActSettings::load(&path).rules, Rules::default());
        std::fs::write(
            directory.path().join("personality.json"),
            r#"{"rules":{"quiet_from":1380,"quiet_until":360,"per_hour":40,"break_after_minutes":5,"sleep_after_goodnight_minutes":null,"looks":false}}"#,
        )
        .unwrap();
        let rules = ActSettings::load(&path).rules;
        assert_eq!((rules.per_hour, rules.break_after_minutes), (12, 20.0));
        assert_eq!(rules.sleep_after_goodnight_minutes, None);
        assert!(!rules.looks && rules.wave_back && rules.spacing_minutes == 10.0);
        assert!(
            rules.quiet_at(23.5)
                && rules.quiet_at(2.0)
                && !rules.quiet_at(22.5)
                && !rules.quiet_at(6.0)
        );
        let same = Rules {
            quiet_from: 600,
            quiet_until: 600,
            ..Rules::default()
        };
        assert!(!same.quiet_at(10.0));
        let default = Rules::default();
        assert!(
            default.late_at(21.0)
                && default.late_at(4.9)
                && !default.late_at(5.0)
                && !default.late_at(20.9)
        );
    }
}
