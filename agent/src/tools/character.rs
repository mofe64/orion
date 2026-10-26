use serde_json::{Value, json};

#[cfg(test)]
const NAMES: [&str; 5] = [
    "follow_me",
    "stop_following",
    "set_proactive_mode",
    "set_demo_mode",
    "pause_remarks",
];

pub(super) fn schemas() -> Vec<Value> {
    let empty = json!({"type":"object","additionalProperties":false,"properties":{},"required":[]});
    let on = json!({"type":"object","additionalProperties":false,"required":["on"],"properties":{"on":{"type":"boolean"}}});
    vec![
        json!({"type":"function","name":"follow_me","description":"Start following the user's hand or face with Orion's head when asked, for example: follow my hand, follow my face, let's play. Confirm only after success.","inputSchema":{"type":"object","additionalProperties":false,"required":["target"],"properties":{"target":{"type":"string","enum":["hand","face"]}}}}),
        json!({"type":"function","name":"stop_following","description":"Stop following the user's hand or face when asked and return to the earlier camera behaviour. Confirm only after success.","inputSchema":empty}),
        json!({"type":"function","name":"set_proactive_mode","description":"Turn proactive mode on or off when asked. In proactive mode Orion sometimes comments on what it notices, such as someone arriving or a new object. Confirm only after success.","inputSchema":on}),
        json!({"type":"function","name":"set_demo_mode","description":"Turn demo mode on or off when asked. Demo mode makes Orion react and comment much more often, with bigger movements, for showing Orion to people. Confirm only after success.","inputSchema":on}),
        json!({"type":"function","name":"pause_remarks","description":"Stop Orion's unprompted comments for a while when the user asks Orion to be quiet, stop commenting or leave them alone. Default 120 minutes. Questions still get answers. Confirm briefly after success.","inputSchema":{"type":"object","additionalProperties":false,"properties":{"minutes":{"type":"integer","minimum":5,"maximum":480}},"required":[]}}),
    ]
}

pub(super) fn resolve(name: &str, arguments: Value) -> Result<Value, String> {
    let args = arguments.as_object().ok_or("Invalid character request.")?;
    let switch = || {
        args.get("on")
            .and_then(Value::as_bool)
            .filter(|_| args.len() == 1)
            .ok_or("Say on or off.")
    };
    match name {
        "follow_me" => {
            let target = args
                .get("target")
                .and_then(Value::as_str)
                .filter(|_| args.len() == 1)
                .ok_or("Choose hand or face.")?;
            let mode = match target {
                "hand" => "hands",
                "face" => "faces",
                _ => return Err("Choose hand or face.".into()),
            };
            Ok(json!({"operation":"tracking","mode":mode}))
        }
        "stop_following" if args.is_empty() => Ok(json!({"operation":"tracking","mode":"resume"})),
        "set_proactive_mode" => {
            Ok(json!({"operation":"character_behaviours","proactive_speech":switch()?}))
        }
        "set_demo_mode" => Ok(json!({"operation":"character_behaviours","demo_mode":switch()?})),
        "pause_remarks" => {
            let minutes = match args.get("minutes") {
                None => 120,
                Some(value) => value
                    .as_u64()
                    .filter(|minutes| (5..=480).contains(minutes))
                    .ok_or("Choose 5 to 480 minutes.")?,
            };
            if args.keys().any(|key| key != "minutes") {
                return Err("Invalid quiet request.".into());
            }
            Ok(json!({"operation":"remarks","quiet_minutes":minutes}))
        }
        _ => Err("Invalid character request.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_tools_send_bounded_operations() {
        assert_eq!(
            resolve("follow_me", json!({"target":"hand"})).unwrap(),
            json!({"operation":"tracking","mode":"hands"})
        );
        assert_eq!(
            resolve("follow_me", json!({"target":"face"})).unwrap(),
            json!({"operation":"tracking","mode":"faces"})
        );
        assert_eq!(
            resolve("stop_following", json!({})).unwrap(),
            json!({"operation":"tracking","mode":"resume"})
        );
        assert_eq!(
            resolve("set_demo_mode", json!({"on":true})).unwrap(),
            json!({"operation":"character_behaviours","demo_mode":true})
        );
        assert_eq!(
            resolve("set_proactive_mode", json!({"on":false})).unwrap(),
            json!({"operation":"character_behaviours","proactive_speech":false})
        );
        assert_eq!(
            resolve("pause_remarks", json!({})).unwrap(),
            json!({"operation":"remarks","quiet_minutes":120})
        );
        for (name, value) in [
            ("follow_me", json!({"target":"cat"})),
            ("follow_me", json!({})),
            ("stop_following", json!({"target":"hand"})),
            ("set_demo_mode", json!({"on":"yes"})),
            ("set_demo_mode", json!({"on":true,"extra":1})),
            ("pause_remarks", json!({"minutes":2})),
            ("pause_remarks", json!({"minutes":60,"extra":1})),
        ] {
            assert!(resolve(name, value).is_err(), "{name}");
        }
        assert_eq!(schemas().len(), NAMES.len());
    }
}
