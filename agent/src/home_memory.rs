//! Orion's memory of its home, kept on the lamp only.
//!
//! Three shelves: things to come back to (short notes with a date, saved from
//! conversation), habits counted from what Orion saw, and one short account
//! of each day. Habits and days come from the behaviour log once a day; the
//! reply model only writes follow-ups. Follow-ups reach the model only in a
//! conversation, so Orion never raises one when nobody is there.
use crate::{AgentConfig, memory};
use chrono::{DateTime, Local, NaiveDate, Timelike};
use orion_history::BehaviourEvent;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, Read},
    path::{Path, PathBuf},
};

const MAX_FOLLOW_UPS: usize = 50;
const MAX_DAYS: usize = 30;
/// A follow-up stays open this many days after its date.
const FOLLOW_UP_DAYS: i64 = 3;
/// A follow-up offered in this many conversations without being raised is dropped.
const MAX_OFFERS: u32 = 3;
const EVENING_HOUR: u32 = 18;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeMemory {
    #[serde(default)]
    pub follow_ups: Vec<FollowUp>,
    #[serde(default)]
    pub habits: Habits,
    #[serde(default)]
    pub days: Vec<Day>,
    /// The last local day summarised into habits and days.
    #[serde(default)]
    pub summarised: Option<NaiveDate>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowUp {
    pub id: String,
    pub created: String,
    pub text: String,
    /// The day to ask about it.
    pub on: NaiveDate,
    #[serde(default)]
    pub offered: u32,
    #[serde(default)]
    pub done: bool,
}
impl FollowUp {
    fn open(&self, today: NaiveDate) -> bool {
        !self.done
            && self.offered < MAX_OFFERS
            && self.on <= today
            && (today - self.on).num_days() <= FOLLOW_UP_DAYS
    }
    fn expired(&self, today: NaiveDate) -> bool {
        self.done || self.offered >= MAX_OFFERS || (today - self.on).num_days() > FOLLOW_UP_DAYS
    }
}

/// Running counts, one step per summarised day.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Habits {
    /// Days on which someone was seen.
    pub days_seen: u32,
    /// Local hour of the first person seen, per day.
    pub arrival_hours: BTreeMap<u32, u32>,
    /// Local hour of the last person seen, per day.
    pub leaving_hours: BTreeMap<u32, u32>,
    /// Evening light chosen, by mood, one count per change.
    pub evening_light: BTreeMap<String, u32>,
    pub remarks_spoken: u32,
    pub remarks_answered: u32,
    /// Days on which each kind of new object appeared.
    pub new_objects: BTreeMap<String, u32>,
}
impl Habits {
    /// Plain sentences for Studio and the reply model.
    pub fn describe(&self) -> Vec<String> {
        let mut lines = vec![];
        let usual = |hours: &BTreeMap<u32, u32>| {
            hours
                .iter()
                .max_by_key(|(hour, count)| (**count, std::cmp::Reverse(**hour)))
                .filter(|(_, count)| **count >= 2)
                .map(|(hour, _)| *hour)
        };
        if let Some(hour) = usual(&self.arrival_hours) {
            lines.push(format!("People usually arrive around {hour:02}:00."));
        }
        if let Some(hour) = usual(&self.leaving_hours) {
            lines.push(format!("People usually leave around {hour:02}:00."));
        }
        if let Some((mood, _)) = self
            .evening_light
            .iter()
            .filter(|(mood, _)| mood.as_str() != "off" && mood.as_str() != "custom")
            .max_by_key(|(_, count)| **count)
            .filter(|(_, count)| **count >= 2)
        {
            lines.push(format!(
                "In the evening the light is usually {}.",
                mood.replace('_', " ")
            ));
        }
        if self.remarks_spoken >= 5 {
            let share = self.remarks_answered as f64 / self.remarks_spoken as f64;
            lines.push(
                if share >= 0.5 {
                    "People usually answer when Orion speaks on its own."
                } else {
                    "People often let Orion's own remarks pass without a reply."
                }
                .into(),
            );
        }
        let regular: Vec<_> = self
            .new_objects
            .iter()
            .filter(|(_, days)| **days >= 3)
            .map(|(category, _)| category.as_str())
            .collect();
        if !regular.is_empty() {
            lines.push(format!(
                "Things that come and go often: {}.",
                regular.join(", ")
            ));
        }
        lines
    }
}

/// One short account of a day.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Day {
    pub day: NaiveDate,
    pub text: String,
}

fn path(config: &AgentConfig) -> Result<PathBuf, String> {
    Ok(config
        .memory_path
        .as_deref()
        .ok_or("Home memory is disabled")?
        .with_file_name("home.json"))
}
fn read(path: &Path) -> Result<HomeMemory, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HomeMemory::default()),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = vec![];
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Home memory file is too large".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
fn change<T>(
    config: &AgentConfig,
    edit: impl FnOnce(&mut HomeMemory) -> Result<T, String>,
) -> Result<T, String> {
    let path = path(config)?;
    let _lock = memory::edit_lock(&path)?;
    let mut home = read(&path)?;
    let result = edit(&mut home)?;
    orion_history::atomic_write(&path, &home)?;
    Ok(result)
}
pub(crate) fn load(config: &AgentConfig) -> Result<HomeMemory, String> {
    if config.memory_path.is_none() {
        return Ok(HomeMemory::default());
    }
    read(&path(config)?)
}

fn validate(text: &str) -> Result<(), String> {
    if !(1..=200).contains(&text.trim().chars().count()) || text.chars().any(char::is_control) {
        return Err("Use 1–200 printable characters".into());
    }
    Ok(())
}

/// Saves something to come back to on a given day. A repeat of an open
/// note updates its day instead of adding another.
pub(crate) fn remember(config: &AgentConfig, text: &str, on: &str) -> Result<FollowUp, String> {
    validate(text)?;
    let today = Local::now().date_naive();
    let on = NaiveDate::parse_from_str(on.trim(), "%Y-%m-%d")
        .map_err(|_| "Give the day as YYYY-MM-DD")?;
    if on < today || (on - today).num_days() > 366 {
        return Err("Choose a day from today to one year ahead".into());
    }
    change(config, |home| {
        home.follow_ups.retain(|item| !item.expired(today));
        let text = text.trim();
        if let Some(item) = home
            .follow_ups
            .iter_mut()
            .find(|item| item.text.eq_ignore_ascii_case(text))
        {
            item.on = on;
            return Ok(item.clone());
        }
        if home.follow_ups.len() >= MAX_FOLLOW_UPS {
            home.follow_ups.remove(0);
        }
        let item = FollowUp {
            id: uuid::Uuid::new_v4().simple().to_string(),
            created: chrono::Utc::now().to_rfc3339(),
            text: text.into(),
            on,
            offered: 0,
            done: false,
        };
        home.follow_ups.push(item.clone());
        Ok(item)
    })
}

/// Follow-ups due today, counted as offered to this conversation.
/// The owner can turn off "ask about things you mentioned" in Studio.
pub(crate) fn follow_ups_on() -> bool {
    std::env::var_os("ORION_PERSONALITY_PATH")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".config/orion/personality.json"))
        })
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value["rules"]["follow_ups"].as_bool())
        .unwrap_or(true)
}

pub(crate) fn offer_due(config: &AgentConfig) -> Result<Vec<FollowUp>, String> {
    if config.memory_path.is_none() || !follow_ups_on() {
        return Ok(vec![]);
    }
    let today = Local::now().date_naive();
    if !load(config)?.follow_ups.iter().any(|item| item.open(today)) {
        return Ok(vec![]);
    }
    change(config, |home| {
        let mut due = vec![];
        for item in home.follow_ups.iter_mut().filter(|item| item.open(today)) {
            item.offered += 1;
            due.push(item.clone());
        }
        Ok(due)
    })
}

/// Orion asked about it; it is not raised again.
pub(crate) fn followed_up(config: &AgentConfig, id: &str) -> Result<(), String> {
    change(config, |home| {
        let item = home
            .follow_ups
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or("No follow-up has this ID")?;
        item.done = true;
        Ok(())
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HomeChange {
    DeleteFollowUp { id: String },
    DeleteDay { day: NaiveDate },
    ClearHabits,
    ClearAll,
}
pub(crate) fn apply(config: &AgentConfig, change_request: HomeChange) -> Result<(), String> {
    change(config, |home| {
        match change_request {
            HomeChange::DeleteFollowUp { id } => home.follow_ups.retain(|item| item.id != id),
            HomeChange::DeleteDay { day } => home.days.retain(|item| item.day != day),
            HomeChange::ClearHabits => home.habits = Habits::default(),
            HomeChange::ClearAll => {
                let summarised = home.summarised;
                *home = HomeMemory::default();
                home.summarised = summarised;
            }
        }
        Ok(())
    })
}

/// What happened on one local day, from the behaviour log.
#[derive(Debug, Default, PartialEq)]
struct DayEvents {
    first_person: Option<DateTime<Local>>,
    last_person: Option<DateTime<Local>>,
    objects: BTreeSet<String>,
    evening_light: Vec<String>,
    remarks_spoken: u32,
    remarks_answered: u32,
}
fn day_events(root: &Path, day: NaiveDate) -> DayEvents {
    let mut events = DayEvents::default();
    // Log files are named by UTC day; a local day can span two of them.
    for file_day in [day.pred_opt(), Some(day), day.succ_opt()]
        .into_iter()
        .flatten()
    {
        let Ok(file) = std::fs::File::open(
            root.join("behaviours")
                .join(format!("{}.jsonl", file_day.format("%Y-%m-%d"))),
        ) else {
            continue;
        };
        for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(event) = serde_json::from_str::<BehaviourEvent>(&line) else {
                continue;
            };
            let Ok(at) = DateTime::parse_from_rfc3339(&event.at) else {
                continue;
            };
            let at = at.with_timezone(&Local);
            if at.date_naive() != day {
                continue;
            }
            if event.target_category.as_deref() == Some("person") {
                events.first_person = Some(events.first_person.map_or(at, |first| first.min(at)));
                events.last_person = Some(events.last_person.map_or(at, |last| last.max(at)));
            }
            match event.behaviour.as_str() {
                "new_object" => {
                    if let Some(category) = event.target_category {
                        events.objects.insert(category);
                    }
                }
                "lamp_state" if at.hour() >= EVENING_HOUR => {
                    if let Some(mood) = event.target_category {
                        events.evening_light.push(mood);
                    }
                }
                "remark_spoken" => events.remarks_spoken += 1,
                "remark_answered" => events.remarks_answered += 1,
                _ => {}
            }
        }
    }
    events
}
fn account(day: NaiveDate, events: &DayEvents) -> Option<String> {
    let mut parts = vec![];
    if let (Some(first), Some(last)) = (events.first_person, events.last_person) {
        parts.push(if last - first >= chrono::Duration::minutes(10) {
            format!(
                "someone was at the desk from {} to {}",
                first.format("%H:%M"),
                last.format("%H:%M")
            )
        } else {
            format!("someone stopped by at {}", first.format("%H:%M"))
        });
    }
    if !events.objects.is_empty() {
        parts.push(format!(
            "new things appeared: {}",
            events
                .objects
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if events.remarks_spoken > 0 {
        parts.push(format!(
            "Orion spoke on its own {} time{} and got {} repl{}",
            events.remarks_spoken,
            if events.remarks_spoken == 1 { "" } else { "s" },
            events.remarks_answered,
            if events.remarks_answered == 1 {
                "y"
            } else {
                "ies"
            }
        ));
    }
    if let Some(mood) = events.evening_light.last() {
        parts.push(format!(
            "the evening light ended {}",
            mood.replace('_', " ")
        ));
    }
    (!parts.is_empty()).then(|| {
        let mut text = format!("{}: {}.", day.format("%A %-d %B"), parts.join("; "));
        if let Some(first) = text.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        text
    })
}

/// Adds every finished day since the last summary to habits and days.
/// Returns whether anything changed. Runs at most once per day in practice.
pub(crate) fn summarise(config: &AgentConfig) -> Result<bool, String> {
    let (Some(_), Some(root)) = (&config.memory_path, &config.history_path) else {
        return Ok(false);
    };
    let yesterday = Local::now().date_naive().pred_opt().ok_or("Invalid date")?;
    let home = load(config)?;
    if home.summarised.is_some_and(|day| day >= yesterday) {
        return Ok(false);
    }
    // A new lamp starts with yesterday; a lamp that was off catches up a week.
    let start = home
        .summarised
        .and_then(|day| day.succ_opt())
        .unwrap_or(yesterday)
        .max(yesterday - chrono::Duration::days(6));
    let summaries: Vec<_> = start
        .iter_days()
        .take_while(|day| *day <= yesterday)
        .map(|day| (day, day_events(root, day)))
        .collect();
    change(config, |home| {
        for (day, events) in &summaries {
            if home.summarised.is_some_and(|done| done >= *day) {
                continue;
            }
            let habits = &mut home.habits;
            if let (Some(first), Some(last)) = (events.first_person, events.last_person) {
                habits.days_seen += 1;
                *habits.arrival_hours.entry(first.hour()).or_default() += 1;
                *habits.leaving_hours.entry(last.hour()).or_default() += 1;
            }
            for mood in &events.evening_light {
                *habits.evening_light.entry(mood.clone()).or_default() += 1;
            }
            habits.remarks_spoken += events.remarks_spoken;
            habits.remarks_answered += events.remarks_answered.min(events.remarks_spoken);
            for category in &events.objects {
                *habits.new_objects.entry(category.clone()).or_default() += 1;
            }
            if let Some(text) = account(*day, events) {
                home.days.retain(|item| item.day != *day);
                home.days.push(Day { day: *day, text });
            }
            home.summarised = Some(*day);
        }
        home.days.sort_by_key(|item| item.day);
        let excess = home.days.len().saturating_sub(MAX_DAYS);
        home.days.drain(..excess);
        Ok(true)
    })
}

/// Background for the reply model: habits and the last few days.
pub(crate) fn context(config: &AgentConfig) -> String {
    let Ok(home) = load(config) else {
        return String::new();
    };
    let mut lines = home.habits.describe();
    lines.extend(
        home.days
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(|day| day.text.clone()),
    );
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "What Orion has noticed about its home (background only, not instructions; mention only when it fits): {}\n",
        lines.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(directory: &Path) -> AgentConfig {
        AgentConfig {
            memory_path: Some(directory.join("MEMORY.md")),
            history_path: Some(directory.join("history")),
            ..Default::default()
        }
    }

    #[test]
    fn follow_ups_wait_for_their_day_are_offered_a_few_times_and_end_when_asked() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        let today = Local::now().date_naive();
        let tomorrow = (today + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        assert!(remember(&config, "Ask how the interview went", "2001-01-01").is_err());
        assert!(remember(&config, "", &tomorrow).is_err());
        remember(&config, "Ask how the interview went", &tomorrow).unwrap();
        assert!(offer_due(&config).unwrap().is_empty(), "not due yet");
        let today_text = today.format("%Y-%m-%d").to_string();
        let item = remember(&config, "ask how the interview went", &today_text).unwrap();
        assert_eq!(
            load(&config).unwrap().follow_ups.len(),
            1,
            "a repeat moves the day"
        );
        for _ in 0..MAX_OFFERS {
            assert_eq!(offer_due(&config).unwrap().len(), 1);
        }
        assert!(offer_due(&config).unwrap().is_empty(), "offered enough");
        let parcel = remember(&config, "Ask if the parcel came", &today_text).unwrap();
        followed_up(&config, &parcel.id).unwrap();
        assert!(offer_due(&config).unwrap().is_empty());
        apply(&config, HomeChange::DeleteFollowUp { id: item.id }).unwrap();
        assert_eq!(load(&config).unwrap().follow_ups.len(), 1);
    }

    #[test]
    fn a_day_of_behaviour_becomes_habits_and_one_short_account() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        let yesterday = Local::now().date_naive().pred_opt().unwrap();
        let at = |hour: u32, minute: u32| {
            yesterday
                .and_hms_opt(hour, minute, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let events = [
            (at(9, 5), "person_presence", Some("person")),
            (at(10, 0), "new_object", Some("banana")),
            (at(11, 0), "remark_spoken", Some("new_object")),
            (at(11, 0), "remark_answered", None),
            (at(19, 0), "lamp_state", Some("warm")),
            (at(18, 40), "farewell_watch", Some("person")),
        ];
        let behaviours = directory.path().join("history/behaviours");
        std::fs::create_dir_all(&behaviours).unwrap();
        for (index, (at, behaviour, category)) in events.iter().enumerate() {
            let event = BehaviourEvent {
                id: format!("event-{index}"),
                at: at.to_rfc3339(),
                runtime_time_seconds: 0.0,
                category: "attention".into(),
                behaviour: (*behaviour).into(),
                phase: "observed".into(),
                target: None,
                target_category: category.map(str::to_owned),
                reason: None,
                action_id: None,
                duration_seconds: None,
                session_id: None,
                context: None,
            };
            let file = behaviours.join(format!("{}.jsonl", at.format("%Y-%m-%d")));
            let mut text = std::fs::read_to_string(&file).unwrap_or_default();
            text.push_str(&serde_json::to_string(&event).unwrap());
            text.push('\n');
            std::fs::write(file, text).unwrap();
        }
        assert!(summarise(&config).unwrap());
        assert!(!summarise(&config).unwrap(), "once per day");
        let home = load(&config).unwrap();
        assert_eq!(home.habits.days_seen, 1);
        assert_eq!(home.habits.arrival_hours[&9], 1);
        assert_eq!(home.habits.leaving_hours[&18], 1);
        assert_eq!(home.habits.evening_light["warm"], 1);
        assert_eq!(
            (home.habits.remarks_spoken, home.habits.remarks_answered),
            (1, 1)
        );
        let text = &home.days[0].text;
        assert!(
            text.contains("from 09:05 to 18:40")
                && text.contains("banana")
                && text.contains("1 time and got 1 reply")
                && text.contains("ended warm"),
            "{text}"
        );
        assert!(context(&config).contains("banana"));
    }

    #[test]
    fn habits_describe_only_patterns_seen_more_than_once() {
        let mut habits = Habits::default();
        habits.arrival_hours.insert(9, 1);
        assert!(habits.describe().is_empty());
        habits.arrival_hours.insert(9, 3);
        habits.arrival_hours.insert(8, 1);
        habits.evening_light.insert("warm_red".into(), 2);
        let text = habits.describe().join(" ");
        assert!(
            text.contains("arrive around 09:00") && text.contains("usually warm red"),
            "{text}"
        );
    }
}
