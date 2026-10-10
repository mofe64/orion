use serde::Deserialize;
use serde_json::{Value, json};

fn repeat_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["days","time"],"properties":{"days":{"type":"array","minItems":1,"maxItems":7,"uniqueItems":true,"items":{"type":"string","enum":["mon","tue","wed","thu","fri","sat","sun"]}},"time":{"type":"string","pattern":"^([01][0-9]|2[0-3]):[0-5][0-9]$"}}})
}
pub fn schemas() -> Vec<Value> {
    let object = |properties: Value, required: Value| json!({"type":"object","additionalProperties":false,"properties":properties,"required":required});
    vec![
        json!({"type":"function","name":"start_light_routine","description":"Start a requested wind-down (10–60 minutes) or focus session (5–120 minutes). Wind-down fades to a low warm glow, then rests after foreground activity finishes. Focus selects bright cool lamp light, creates a Focus session timer, then uses warm white and restores the prior mode after ringing. Restarting Orion cancels the light routine; a focus timer still rings on schedule. Starting another routine or changing lighting cancels this routine. Confirm only after success.","inputSchema":object(json!({"routine":{"type":"string","enum":["wind_down","focus"]},"minutes":{"type":"integer","minimum":5,"maximum":120}}),json!(["routine","minutes"]))}),
        json!({"type":"function","name":"stop_light_routine","description":"Stop the running light routine, cancel its focus timer, and restore the prior lamp and mode. Confirm only after success.","inputSchema":object(json!({}),json!([]))}),
        json!({"type":"function","name":"set_reminder","description":"Set a reminder only when requested. Supply text and exactly one trigger: at (RFC3339 with offset), in_seconds, repeat (Pi local weekdays and HH:MM), or when=next_conversation. Read list_alerts for local time first. Confirm only after success.","inputSchema":object(json!({"text":{"type":"string","minLength":1,"maxLength":200},"at":{"type":"string"},"in_seconds":{"type":"number","minimum":1,"maximum":604800},"repeat":repeat_schema(),"when":{"type":"string","enum":["next_conversation"]}}),json!(["text"]))}),
        json!({"type":"function","name":"snooze_alert","description":"Snooze the most recently dismissed alert, only within two minutes of dismissal. minutes defaults to 9; choose 1–60. Confirm only after success.","inputSchema":object(json!({"minutes":{"type":"integer","minimum":1,"maximum":60}}),json!([]))}),
        json!({"type":"function","name":"set_alert_sound","description":"Change the default alarm or timer sound only when requested. A ringing alert keeps its selected sound. Confirm only after success.","inputSchema":object(json!({"kind":{"type":"string","enum":["alarm","timer"]},"sound":{"type":"string","enum":["two_tone","club_alarm","funny_alarm"]}}),json!(["kind","sound"]))}),
        json!({"type":"function","name":"set_mode","description":"Select lamp mode (idle animations, no automatic sleep) or idle mode (character animations, rest after 30 minutes). Only when requested.","inputSchema":object(json!({"mode":{"type":"string","enum":["lamp","idle"]}}),json!(["mode"]))}),
        json!({"type":"function","name":"go_to_sleep","description":"Ask Orion to move to rest after its spoken acknowledgement. Only when requested. Existing alerts remain scheduled.","inputSchema":object(json!({}),json!([]))}),
        json!({"type":"function","name":"set_timer","description":"Create a one-time countdown on Orion. It rings until Hey Orion dismisses it, or for at most five minutes. Confirm the returned duration and label after success.","inputSchema":object(json!({"seconds":{"type":"number","minimum":1,"maximum":604800},"label":{"type":"string","maxLength":80}}),json!(["seconds","label"]))}),
        json!({"type":"function","name":"set_alarm","description":"Create a clock alarm, once at an explicit time or recurring on Pi local weekdays. Call list_alerts first for current local time and timezone; use the UTC offset that applies on the requested date. If the IANA timezone is unknown, use the reported local offset for today and clarify the offset for a future date when it cannot be determined. Supply a future RFC3339 timestamp with explicit UTC offset. Clarify ambiguous AM/PM or date; never silently choose a timezone. Supply exactly one of at or repeat. briefing=true gives a morning briefing after dismissal.","inputSchema":object(json!({"at":{"type":"string"},"repeat":repeat_schema(),"briefing":{"type":"boolean"},"sunrise_minutes":{"type":"integer","minimum":5,"maximum":60},"label":{"type":"string","maxLength":80}}),json!(["label"]))}),
        json!({"type":"function","name":"list_alerts","description":"Read Orion's current local time and pending, ringing, announcing and recent timers, alarms and reminders. Use before setting a clock alarm or choosing an alert to cancel.","inputSchema":object(json!({}),json!([]))}),
        json!({"type":"function","name":"cancel_alert","description":"Cancel one occurrence by its returned ID by default. series=true also cancels all future occurrences of its recurring alarm or reminder. For a recurring alert, ask whether the user means this occurrence or the whole series when unclear. List alerts if the ID is unknown; clarify when several match.","inputSchema":object(json!({"id":{"type":"integer","minimum":1},"series":{"type":"boolean","default":false}}),json!(["id"]))}),
        json!({"type":"function","name":"stop_alert","description":"Dismiss all ringing alarms and timers. Pending reminders and other scheduled alerts stay scheduled.","inputSchema":object(json!({}),json!([]))}),
    ]
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sound {
    kind: String,
    sound: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mode {
    mode: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Timer {
    seconds: f64,
    label: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LightRoutine {
    routine: String,
    minutes: u16,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Alarm {
    at: Option<String>,
    label: String,
    repeat: Option<Repeat>,
    #[serde(default)]
    briefing: bool,
    sunrise_minutes: Option<u8>,
}
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Repeat {
    days: Vec<String>,
    time: String,
}
impl Repeat {
    fn validate(&self) -> Result<(), String> {
        let unique: std::collections::HashSet<_> = self.days.iter().collect();
        if self.days.is_empty()
            || unique.len() != self.days.len()
            || self
                .days
                .iter()
                .any(|d| !["mon", "tue", "wed", "thu", "fri", "sat", "sun"].contains(&d.as_str()))
            || self.time.len() != 5
            || chrono::NaiveTime::parse_from_str(&self.time, "%H:%M").is_err()
        {
            return Err("Repeat requires unique weekdays and HH:MM time".into());
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reminder {
    text: String,
    at: Option<String>,
    in_seconds: Option<f64>,
    repeat: Option<Repeat>,
    when: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Snooze {
    minutes: Option<u8>,
}
fn timestamp(at: &str) -> Result<f64, String> {
    chrono::DateTime::parse_from_rfc3339(at)
        .map(|at| at.timestamp() as f64)
        .map_err(|_| "Use an RFC3339 date and time with explicit UTC offset".into())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancel {
    id: u64,
    #[serde(default)]
    series: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| e.to_string())
}
pub fn resolve(name: &str, arguments: Value) -> Result<Value, String> {
    let request = match name {
        "start_light_routine" => {
            let a: LightRoutine = parse(arguments)?;
            let bounds = match a.routine.as_str() {
                "wind_down" => 10..=60,
                "focus" => 5..=120,
                _ => return Err("Unknown light routine".into()),
            };
            if !bounds.contains(&a.minutes) {
                return Err("Wind-down needs 10–60 minutes; focus needs 5–120 minutes".into());
            }
            json!({"action":"start_light_routine","routine":a.routine,"minutes":a.minutes})
        }
        "stop_light_routine" => {
            let _: Empty = parse(arguments)?;
            json!({"action":"stop_light_routine"})
        }
        "snooze_alert" => {
            let a: Snooze = parse(arguments)?;
            if a.minutes.is_some_and(|m| !(1..=60).contains(&m)) {
                return Err("Snooze must be 1–60 minutes".into());
            }
            json!({"action":"snooze","minutes":a.minutes})
        }
        "set_reminder" => {
            let a: Reminder = parse(arguments)?;
            crate::reminders::validate(&a.text)?;
            if usize::from(a.at.is_some())
                + usize::from(a.in_seconds.is_some())
                + usize::from(a.repeat.is_some())
                + usize::from(a.when.is_some())
                != 1
            {
                return Err("Choose exactly one reminder trigger".into());
            }
            if let Some(when) = a.when {
                if when != "next_conversation" {
                    return Err("when must be next_conversation".into());
                }
                return Ok(json!({"next_conversation":a.text}));
            }
            if a.in_seconds
                .is_some_and(|s| !s.is_finite() || !(1.0..=604800.).contains(&s))
            {
                return Err("Reminder duration must be one second to seven days".into());
            }
            if let Some(rule) = &a.repeat {
                rule.validate()?;
            }
            json!({"action":"reminder","label":a.text,"due_unix":a.at.as_deref().map(timestamp).transpose()?,"seconds":a.in_seconds,"repeat":a.repeat})
        }
        "set_alert_sound" => {
            let a: Sound = parse(arguments)?;
            if !["alarm", "timer"].contains(&a.kind.as_str())
                || !["two_tone", "club_alarm", "funny_alarm"].contains(&a.sound.as_str())
            {
                return Err("Choose a published alarm or timer sound".into());
            }
            json!({"action":"set_sound","kind":a.kind,"sound":a.sound})
        }
        "get_lighting" => {
            let _: Empty = parse(arguments)?;
            return Ok(json!({"operation":"lamp_status"}));
        }
        "set_mode" => {
            let a: Mode = parse(arguments)?;
            if !["lamp", "idle"].contains(&a.mode.as_str()) {
                return Err("Mode must be lamp or idle".into());
            }
            json!({"action":"set_mode","mode":a.mode})
        }
        "set_timer" => {
            let a: Timer = parse(arguments)?;
            if !a.seconds.is_finite() || !(1.0..=604800.).contains(&a.seconds) {
                return Err("Timer must be one second to seven days".into());
            }
            json!({"action":"timer","seconds":a.seconds,"label":a.label})
        }
        "set_alarm" => {
            let a: Alarm = parse(arguments)?;
            if a.sunrise_minutes.is_some_and(|m| !(5..=60).contains(&m)) {
                return Err("Sunrise must be 5–60 minutes".into());
            }
            if a.at.is_some() == a.repeat.is_some() {
                return Err("Choose exactly one alarm time or repeat".into());
            }
            if let Some(rule) = &a.repeat {
                rule.validate()?;
            }
            if a.label.chars().count() > 80 || a.label.chars().any(char::is_control) {
                return Err("Use a label of at most 80 printable characters".into());
            }
            json!({"action":"alarm","due_unix":a.at.as_deref().map(timestamp).transpose()?,"repeat":a.repeat,"label":a.label,"briefing":a.briefing,"sunrise_minutes":a.sunrise_minutes})
        }
        "cancel_alert" => {
            let a: Cancel = parse(arguments)?;
            json!({"action":"cancel","id":a.id,"series":a.series})
        }
        "list_alerts" | "stop_alert" | "go_to_sleep" => {
            let _: Empty = parse(arguments)?;
            if name == "go_to_sleep" {
                return Ok(json!({"operation":"sleep"}));
            }
            json!({"action":if name=="list_alerts" {"list"} else {"stop"}})
        }
        _ => return Err("Tool is not permitted".into()),
    };
    Ok(json!({"operation":"routines","request":request}))
}
/// The Pi's IANA zone lets the agent account for future daylight-saving changes.
/// Clock offsets remain explicit in every scheduled alarm.
pub fn timezone() -> Option<String> {
    resolve_timezone(
        std::env::var("TZ").ok(),
        || {
            std::fs::read_link("/etc/localtime").ok().and_then(|path| {
                path.to_str()
                    .and_then(|s| s.split_once("zoneinfo/").map(|(_, zone)| zone.to_owned()))
            })
        },
        || {
            std::fs::read_to_string("/etc/timezone")
                .ok()
                .map(|s| s.trim().to_owned())
        },
    )
}
fn is_iana_name(value: &str) -> bool {
    value == "UTC"
        || (value.contains('/')
            && value.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && part
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+'))
            }))
}
fn resolve_timezone(
    tz: Option<String>,
    link: impl FnOnce() -> Option<String>,
    file: impl FnOnce() -> Option<String>,
) -> Option<String> {
    tz.filter(|value| is_iana_name(value))
        .or_else(|| link().filter(|value| is_iana_name(value)))
        .or_else(|| file().filter(|value| is_iana_name(value)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn light_routine_tool_discloses_restart_cancellation() {
        let tool = schemas()
            .into_iter()
            .find(|t| t["name"] == "start_light_routine")
            .unwrap();
        let description = tool["description"].as_str().unwrap();
        assert!(description.contains("Restarting Orion cancels the light routine"));
        assert!(description.contains("focus timer still rings on schedule"));
    }
    #[test]
    fn cancel_requires_explicit_series_intent() {
        assert_eq!(
            resolve("cancel_alert", json!({"id":1})).unwrap()["request"]["series"],
            false
        );
        assert_eq!(
            resolve("cancel_alert", json!({"id":1,"series":true})).unwrap()["request"]["series"],
            true
        );
        assert!(resolve("cancel_alert", json!({"id":1,"series":"yes"})).is_err());
        let tool = schemas()
            .into_iter()
            .find(|t| t["name"] == "cancel_alert")
            .unwrap();
        assert!(
            tool["description"]
                .as_str()
                .unwrap()
                .contains("ask whether")
        );
    }
    #[test]
    fn posix_tz_values_fall_through_to_the_system_iana_name() {
        for tz in [
            "CET-1CEST",
            ":/etc/localtime",
            "/etc/localtime",
            "",
            "../London",
            "Europe//London",
        ] {
            assert_eq!(
                resolve_timezone(Some(tz.into()), || Some("Europe/London".into()), || None),
                Some("Europe/London".into())
            );
        }
        for tz in [
            "Europe/London",
            "America/Argentina/Buenos_Aires",
            "Etc/GMT+1",
            "UTC",
        ] {
            assert_eq!(
                resolve_timezone(Some(tz.into()), || panic!("Valid TZ must win"), || None),
                Some(tz.into())
            );
        }
        assert_eq!(resolve_timezone(None, || None, || None), None);
        assert_eq!(
            resolve_timezone(Some("CET-1CEST".into()), || None, || None),
            None
        );
    }
    #[test]
    fn light_routine_and_sunrise_arguments_are_strict_and_bounded() {
        for (routine, valid, invalid) in [("wind_down", 10, 9), ("focus", 120, 121)] {
            assert!(
                resolve(
                    "start_light_routine",
                    json!({"routine":routine,"minutes":valid})
                )
                .is_ok()
            );
            assert!(
                resolve(
                    "start_light_routine",
                    json!({"routine":routine,"minutes":invalid})
                )
                .is_err()
            );
        }
        assert!(
            resolve(
                "start_light_routine",
                json!({"routine":"other","minutes":20})
            )
            .is_err()
        );
        assert!(resolve("stop_light_routine", json!({"id":1})).is_err());
        assert!(
            resolve(
                "set_alarm",
                json!({"label":"wake","at":"2030-01-02T09:00:00Z","sunrise_minutes":4})
            )
            .is_err()
        );
        assert_eq!(
            resolve(
                "set_alarm",
                json!({"label":"wake","at":"2030-01-02T09:00:00Z","sunrise_minutes":30})
            )
            .unwrap()["request"]["sunrise_minutes"],
            30
        );
        for tool in schemas() {
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        }
    }
    #[test]
    fn reminder_has_exactly_one_valid_trigger_and_repeat_is_strict() {
        for args in [
            json!({"text":"bins"}),
            json!({"text":"bins","at":"2030-01-01T09:00:00Z","in_seconds":10}),
            json!({"text":"bins","when":"later"}),
            json!({"text":"bins","repeat":{"days":["mon","mon"],"time":"09:00"}}),
            json!({"text":"bins","repeat":{"days":["mon"],"time":"9:00"}}),
        ] {
            assert!(resolve("set_reminder", args).is_err());
        }
        assert_eq!(
            resolve(
                "set_reminder",
                json!({"text":"call Mum","when":"next_conversation"})
            )
            .unwrap()["next_conversation"],
            "call Mum"
        );
        assert!(
            resolve(
                "set_alarm",
                json!({"label":"gym","repeat":{"days":["mon"],"time":"09:00"}})
            )
            .is_ok()
        );
        assert!(resolve("snooze_alert", json!({"minutes":61})).is_err());
    }
    #[test]
    fn sound_matches_runtime_contract_and_rejects_paths_and_extra_fields() {
        assert_eq!(
            resolve(
                "set_alert_sound",
                json!({"kind":"alarm","sound":"club_alarm"})
            )
            .unwrap(),
            json!({"operation":"routines","request":{"action":"set_sound","kind":"alarm","sound":"club_alarm"}})
        );
        for args in [
            json!({"kind":"alarm","sound":"/tmp/a.wav"}),
            json!({"kind":"reminder","sound":"two_tone"}),
            json!({"kind":"timer","sound":"two_tone","path":"a"}),
        ] {
            assert!(resolve("set_alert_sound", args).is_err());
        }
    }
    #[test]
    fn validates_time_and_blocks_extra_authority() {
        assert!(resolve("go_to_sleep", json!({"session_id":"other"})).is_err());
        assert!(resolve("set_mode", json!({"mode":"disable"})).is_err());
        assert!(resolve("set_timer", json!({"seconds":0,"label":"tea"})).is_err());
        assert!(
            resolve(
                "set_alarm",
                json!({"at":"2030-01-02T09:00:00","label":"wake"})
            )
            .is_err()
        );
        assert_eq!(
            resolve(
                "set_alarm",
                json!({"at":"2030-01-02T09:00:00+01:00","label":"wake"})
            )
            .unwrap()["request"]["due_unix"],
            json!(1893571200.0)
        );
    }
}
