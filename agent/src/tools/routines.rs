use serde::Deserialize;
use serde_json::{Value, json};

pub fn schemas() -> Vec<Value> {
    let object = |properties: Value, required: Value| json!({"type":"object","additionalProperties":false,"properties":properties,"required":required});
    vec![
        json!({"type":"function","name":"set_mode","description":"Select lamp mode (idle animations, no automatic sleep) or idle mode (character animations, rest after 30 minutes). Only when requested.","inputSchema":object(json!({"mode":{"type":"string","enum":["lamp","idle"]}}),json!(["mode"]))}),
        json!({"type":"function","name":"go_to_sleep","description":"Ask Orion to move to rest after its spoken acknowledgement. Only when requested. Existing alerts remain scheduled.","inputSchema":object(json!({}),json!([]))}),
        json!({"type":"function","name":"set_timer","description":"Create a one-time countdown on Orion. It rings until Hey Orion dismisses it, or for at most five minutes. Confirm the returned duration and label after success.","inputSchema":object(json!({"seconds":{"type":"number","minimum":1,"maximum":604800},"label":{"type":"string","maxLength":80}}),json!(["seconds","label"]))}),
        json!({"type":"function","name":"set_alarm","description":"Create a one-time clock alarm. Call list_alerts first for current local time and timezone; use the UTC offset that applies on the requested date. Supply a future RFC3339 timestamp with explicit UTC offset. Clarify ambiguous AM/PM or date; never silently choose a timezone. Repeats are unsupported.","inputSchema":object(json!({"at":{"type":"string"},"label":{"type":"string","maxLength":80}}),json!(["at","label"]))}),
        json!({"type":"function","name":"list_alerts","description":"Read Orion's current local time and pending, ringing and recent timers and alarms. Use before setting a clock alarm or choosing an alert to cancel.","inputSchema":object(json!({}),json!([]))}),
        json!({"type":"function","name":"cancel_alert","description":"Cancel a specific timer or alarm by its returned ID. List alerts if the ID is unknown; clarify when several match.","inputSchema":object(json!({"id":{"type":"integer","minimum":1}}),json!(["id"]))}),
        json!({"type":"function","name":"stop_alert","description":"Dismiss all alerts ringing now. Pending alerts stay scheduled.","inputSchema":object(json!({}),json!([]))}),
    ]
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
struct Alarm {
    at: String,
    label: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancel {
    id: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| e.to_string())
}
pub fn resolve(name: &str, arguments: Value) -> Result<Value, String> {
    let request = match name {
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
            let at = chrono::DateTime::parse_from_rfc3339(&a.at)
                .map_err(|_| "Use an RFC3339 date and time with explicit UTC offset")?;
            json!({"action":"alarm","due_unix":at.timestamp() as f64,"label":a.label})
        }
        "cancel_alert" => {
            let a: Cancel = parse(arguments)?;
            json!({"action":"cancel","id":a.id})
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
pub fn timezone() -> String {
    std::env::var("TZ")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::fs::read_link("/etc/localtime").ok().and_then(|path| {
                path.to_str()
                    .and_then(|s| s.split_once("zoneinfo/").map(|(_, zone)| zone.to_owned()))
            })
        })
        .or_else(|| {
            std::fs::read_to_string("/etc/timezone")
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .unwrap_or_else(|| chrono::Local::now().format("%Z").to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
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
