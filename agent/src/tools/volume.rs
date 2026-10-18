use serde_json::{Value, json};

pub(super) fn schemas() -> Vec<Value> {
    ["set_volume", "louder", "quieter"].into_iter().map(|name| json!({
        "type":"function", "name":name,
        "description": match name {
            "set_volume" => "Set Orion's master volume to an absolute level, 0–100. Alarms remain audible when muted. After success, confirm briefly using the returned level, for example: Volume 40.",
            "louder" => "Raise Orion's master volume by 10, up to 100. After success, confirm briefly using the returned level, for example: Volume 40.",
            _ => "Lower Orion's master volume by 10, down to 0. Alarms remain audible when muted. After success, confirm briefly using the returned level, for example: Volume 40."
        },
        "inputSchema": if name == "set_volume" { json!({"type":"object","additionalProperties":false,"required":["level"],"properties":{"level":{"type":"integer","minimum":0,"maximum":100}}}) }
            else { json!({"type":"object","additionalProperties":false,"properties":{},"required":[]}) }
    })).collect()
}

pub(super) fn resolve(name: &str, arguments: Value) -> Result<Value, String> {
    let args = arguments.as_object().ok_or("Volume needs an object.")?;
    if name == "set_volume" {
        let level = args.get("level").and_then(Value::as_u64).filter(|level| *level <= 100)
            .filter(|_| args.len() == 1).ok_or("Choose a volume from 0 to 100.")?;
        Ok(json!({"operation":"volume","level":level}))
    } else if matches!(name, "louder" | "quieter") && args.is_empty() {
        Ok(json!({"operation":"volume","action":name}))
    } else { Err("Invalid volume request.".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn volume_tools_send_absolute_or_atomic_relative_operations() {
        assert_eq!(resolve("set_volume", json!({"level":40})).unwrap(), json!({"operation":"volume","level":40}));
        for name in ["louder", "quieter"] {
            assert_eq!(resolve(name, json!({})).unwrap(), json!({"operation":"volume","action":name}));
        }
        for value in [json!({"level":-1}),json!({"level":101}),json!({"level":true}),json!({"level":1.5}),json!({"level":40,"extra":1})] {
            assert!(resolve("set_volume", value).is_err());
        }
        assert!(resolve("louder", json!({"level":40})).is_err());
    }
}
