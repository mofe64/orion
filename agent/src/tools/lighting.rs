use serde::Deserialize;
use serde_json::{Value, json};

pub const EFFECTS: &[&str] = &[
    "solid",
    "warm_idle_breathe",
    "attentive_focus",
    "thinking_drift",
    "speaking_energy",
    "acknowledge_pulse",
    "curious_sweep",
    "delight_spark",
    "settle_glow",
    "off",
];
pub const COLORS: &[&str] = &[
    "warm_white",
    "cool_white",
    "red",
    "amber",
    "orange",
    "green",
    "teal",
    "blue",
    "purple",
    "pink",
];
pub const MOODS: &[&str] = &["ambient", "warm", "cool", "warm_red"];
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    brightness: Option<f64>,
    mood: Option<String>,
    effect: Option<String>,
    colors: Option<Vec<String>>,
}
fn color(name: &str) -> Result<[u8; 4], String> {
    Ok(match name {
        "warm_white" => [40, 20, 4, 210],
        "cool_white" => [150, 190, 255, 110],
        "red" => [255, 0, 0, 0],
        "amber" => [255, 110, 0, 0],
        "orange" => [255, 50, 0, 0],
        "green" => [0, 220, 35, 0],
        "teal" => [0, 180, 150, 0],
        "blue" => [0, 45, 255, 0],
        "purple" => [145, 0, 220, 0],
        "pink" => [255, 25, 105, 0],
        _ => return Err("Unknown lighting color".into()),
    })
}
pub(crate) fn resolve(value: Value) -> Result<Value, String> {
    let request: Request = serde_json::from_value(value).map_err(|e| e.to_string())?;
    if request
        .brightness
        .is_some_and(|b| !b.is_finite() || !(0.0..=100.0).contains(&b))
    {
        return Err("Brightness must be 0–100 percent".into());
    }
    if request.mood.is_some() && request.colors.is_some() {
        return Err("Choose a mood or explicit colors, not both".into());
    }
    let mut effect = request.effect;
    if effect
        .as_ref()
        .is_some_and(|s| !EFFECTS.contains(&s.as_str()))
    {
        return Err("Unknown lighting effect".into());
    }
    let mut colors = request.colors;
    if let Some(mood) = request.mood {
        let palette = match mood.as_str() {
            "ambient" => vec!["warm_white", "amber"],
            "warm" => vec!["warm_white"],
            "cool" => vec!["cool_white", "blue"],
            "warm_red" => vec!["warm_white", "red"],
            _ => return Err("Unknown lighting mood".into()),
        };
        colors = Some(palette.into_iter().map(str::to_owned).collect());
    }
    if colors.is_none() && effect.as_ref().is_some_and(|s| s != "off" && s != "solid") {
        let accents = &COLORS[2..];
        let index = uuid::Uuid::new_v4().as_u128() as usize % accents.len();
        colors = Some(vec!["warm_white".into(), accents[index].into()]);
    }
    if colors.is_some() && effect.is_none() {
        effect = Some("solid".into());
    }
    let rgbw = colors
        .as_ref()
        .map(|names| {
            if names.is_empty() || names.len() > 2 {
                return Err("Choose one or two colors".into());
            }
            names
                .iter()
                .map(|name| color(name))
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?;
    if request.brightness.is_none() && effect.is_none() && rgbw.is_none() {
        return Err("Specify brightness, mood, effect, or colors".into());
    }
    Ok(json!({"brightness":request.brightness.map(|b| b / 100.0),"effect":effect,"colors":rgbw}))
}
pub(crate) fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "brightness":{"type":"number","minimum":0,"maximum":100,"description":"Absolute brightness percent; alone preserves current colors and effect."},
        "mood":{"type":"string","enum":MOODS,"description":"ambient=warm white+amber; warm=warm white; cool=cool white+blue; warm_red=warm white+red."},
        "effect":{"type":"string","enum":EFFECTS,"description":"Omit for a steady color. Animated effects default to warm white plus a random accent; do not ask for colors."},
        "colors":{"type":"array","items":{"type":"string","enum":COLORS},"minItems":1,"maxItems":2,"description":"Optional explicit palette instead of mood."}
    }})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_catalog_and_preserves_brightness_only_changes() {
        assert_eq!(
            resolve(json!({"brightness":30})).unwrap(),
            json!({"brightness":0.3,"effect":null,"colors":null})
        );
        let animated = resolve(json!({"effect":"thinking_drift"})).unwrap();
        assert_eq!(animated["colors"][0], json!(color("warm_white").unwrap()));
        assert_eq!(animated["colors"].as_array().unwrap().len(), 2);
        assert!(resolve(json!({"colors":["unknown"]})).is_err());
        assert!(resolve(json!({"brightness":101})).is_err());
        assert!(resolve(json!({"mood":"warm","colors":["red"]})).is_err());
        assert_eq!(
            resolve(json!({"mood":"warm_red"})).unwrap()["colors"][1],
            json!([255, 0, 0, 0])
        );
    }
}
