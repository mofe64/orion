use serde::Deserialize;
use serde_json::{Value, json};

pub fn schemas() -> Vec<Value> {
    let target =
        json!({"entity":{"type":"string"},"name":{"type":"string"},"area":{"type":"string"}});
    let object = |properties: Value, required: Value| json!({"type":"object","additionalProperties":false,"properties":properties,"required":required});
    let mut control = target.clone();
    control.as_object_mut().unwrap().extend(json!({
        "action":{"type":"string","enum":["on","off","toggle","open","close","stop","position","temperature","play","pause","activate","run"]},
        "brightness":{"type":"number","minimum":0,"maximum":100},
        "colour":{"type":"string"},"temperature":{"type":"number","description":"Climate temperature in Home Assistant units, or light colour temperature in Kelvin (1000–40000)."},
        "position":{"type":"number","minimum":0,"maximum":100}
    }).as_object().unwrap().clone());
    vec![
        json!({"type":"function","name":"home_list_devices","description":"List household entities exposed to Orion, with names, rooms and current state.","inputSchema":object(json!({}),json!([]))}),
        json!({"type":"function","name":"home_state","description":"Read one exposed household entity by entity ID or name and optional room. Ask which one when several match.","inputSchema":object(target,json!([]))}),
        json!({"type":"function","name":"home_control","description":"Control one exposed household device by entity ID or name and optional room. Supports lights, switches, fans, climate, covers and media. Locks, alarms, garage doors, gates, scenes and scripts are refused. Brightness and position use percent. Colour is a colour name. Confirm only after success.","inputSchema":object(control,json!(["action"]))}),
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    entity: Option<String>,
    name: Option<String>,
    area: Option<String>,
    action: Option<String>,
    brightness: Option<f64>,
    colour: Option<String>,
    temperature: Option<f64>,
    position: Option<f64>,
}
pub fn resolve(name: &str, arguments: Value) -> Result<Value, String> {
    if name == "home_list_devices" {
        if arguments != json!({}) {
            return Err("Device listing takes no fields".into());
        }
        return Ok(json!({"operation":"home","request":{"action":"list"}}));
    }
    let request: Request =
        serde_json::from_value(arguments.clone()).map_err(|_| "Invalid home request")?;
    if request.entity.is_some() == request.name.is_some() {
        return Err("Choose one entity ID or device name, with an optional room".into());
    }
    for value in [
        &request.entity,
        &request.name,
        &request.area,
        &request.colour,
    ]
    .into_iter()
    .flatten()
    {
        if value.trim().is_empty() || value.len() > 200 || value.chars().any(char::is_control) {
            return Err("Invalid home device name or setting".into());
        }
    }
    for (value, low, high) in [
        (request.brightness, 0., 100.),
        (request.position, 0., 100.),
        (request.temperature, -50., 40000.),
    ] {
        if value.is_some_and(|value| !value.is_finite() || value < low || value > high) {
            return Err("Invalid home device setting".into());
        }
    }
    let mut body = arguments;
    if name == "home_control" {
        let action = request.action.ok_or("Choose a home action")?;
        if ![
            "on",
            "off",
            "toggle",
            "open",
            "close",
            "stop",
            "position",
            "temperature",
            "play",
            "pause",
            "activate",
            "run",
        ]
        .contains(&action.as_str())
        {
            return Err("Unsupported home action".into());
        }
        body["command"] = action.into();
        body["action"] = "control".into();
    } else if name == "home_state" {
        if request.action.is_some()
            || request.brightness.is_some()
            || request.colour.is_some()
            || request.temperature.is_some()
            || request.position.is_some()
        {
            return Err("State reads take only a device and room".into());
        }
        body["action"] = "state".into();
    } else {
        return Err("Unknown home tool".into());
    }
    Ok(json!({"operation":"home","request":body}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_through_robot_operation_and_rejects_ambiguous_input() {
        assert_eq!(
            resolve(
                "home_control",
                json!({"name":"Desk","area":"Study","action":"on","brightness":50})
            )
            .unwrap(),
            json!({"operation":"home","request":{"action":"control","command":"on","name":"Desk","area":"Study","brightness":50}})
        );
        assert!(resolve("home_state", json!({"entity":"light.desk","name":"Desk"})).is_err());
        assert!(
            resolve(
                "home_control",
                json!({"name":"Desk","action":"on","brightness":101})
            )
            .is_err()
        );
        assert!(resolve("home_state", json!({"name":"Desk","action":"on"})).is_err());
        assert_eq!(
            resolve("home_list_devices", json!({})).unwrap()["request"]["action"],
            "list"
        );
    }
}
