use serde::Deserialize;
use serde_json::{Value, json};

pub(super) fn schema() -> Value {
    json!({"type":"function","name":"aim_light",
        "description":"Point Orion's light at a desk object only when the user explicitly asks. Call before speaking. Map everyday names to COCO categories: notebook to book, mug to cup, plant to potted plant. Returns aimed, clamped (possibly no movement), not_found, or ambiguous with candidates. aimed means the light landed on the object, even when lean_limited says Orion leaned less than it wanted; do not mention clamping or limits then. Ask which object when ambiguous; never claim exact aim after clamped. Unsupported objects need clarification; this tool never sends a photo.",
        "inputSchema":{"type":"object","additionalProperties":false,"required":["target"],
            "properties":{"target":{"type":"string"}}}})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    target: String,
}

pub(super) fn resolve(arguments: Value) -> Result<Value, String> {
    let args: Arguments = serde_json::from_value(arguments).map_err(|e| e.to_string())?;
    if args.target.len() > 64 {
        return Err("Target name is too long".into());
    }
    let target = args.target.trim().to_ascii_lowercase();
    let category = match target.as_str() {
        "notebook" | "textbook" => "book",
        "mug" | "coffee mug" => "cup",
        "plant" | "pot plant" | "houseplant" => "potted plant",
        "phone" | "mobile" | "mobile phone" | "smartphone" => "cell phone",
        "sofa" => "couch",
        "television" | "monitor" => "tv",
        "desk" | "table" => "dining table",
        other => other,
    };
    let categories: Vec<String> =
        serde_json::from_str(include_str!("../../../vision/orion_vision/coco.json"))
            .map_err(|e| e.to_string())?;
    if !categories.iter().any(|name| name == category) {
        return Err(
            "Object is outside the supported COCO categories; ask for a supported target".into(),
        );
    }
    Ok(json!({"operation":"aim_light","category":category}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn everyday_names_share_the_detectors_category_list() {
        for (target, expected) in [
            ("notebook", "book"),
            (" MUG ", "cup"),
            ("plant", "potted plant"),
            ("phone", "cell phone"),
            ("book", "book"),
            ("teddy bear", "teddy bear"),
        ] {
            assert_eq!(
                resolve(json!({"target":target})).unwrap()["category"],
                expected
            );
        }
        for args in [
            json!({"target":"red sketch"}),
            json!({"target":"book","session_id":"injected"}),
            json!({"target":42}),
        ] {
            assert!(resolve(args).is_err());
        }
    }
}
