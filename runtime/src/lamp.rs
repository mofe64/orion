//! Persistent lamp settings sit below voice and scene feedback in lighting priority.
use crate::{
    Error, Result,
    lighting::{LIGHTING_EFFECT_NAMES, ORION_LIGHT_PIXEL_COUNT, Rgbw8, render_effect},
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LampPatch {
    pub brightness: Option<f64>,
    pub effect: Option<String>,
    pub colors: Option<Vec<[u8; 4]>>,
}
#[derive(Clone)]
pub struct LampProgram {
    brightness: f64,
    effect: String,
    colors: Vec<Rgbw8>,
}
impl Default for LampProgram {
    fn default() -> Self {
        Self::from_color(Rgbw8::new(40, 20, 4, 210))
    }
}
impl LampProgram {
    pub fn from_color(color: Rgbw8) -> Self {
        Self {
            brightness: 1.0,
            effect: "solid".into(),
            colors: vec![color],
        }
    }
    pub fn updated(&self, patch: LampPatch) -> Result<Self> {
        if patch.brightness.is_none() && patch.effect.is_none() && patch.colors.is_none() {
            return Err(Error::InvalidArgument("Empty lamp update".into()));
        }
        let mut next = self.clone();
        if let Some(brightness) = patch.brightness {
            if !brightness.is_finite() || !(0.0..=1.0).contains(&brightness) {
                return Err(Error::InvalidArgument("Brightness must be 0–1".into()));
            }
            next.brightness = brightness;
        }
        if let Some(effect) = patch.effect {
            if effect != "solid" && !LIGHTING_EFFECT_NAMES.contains(&effect.as_str()) {
                return Err(Error::InvalidArgument("Unknown lamp effect".into()));
            }
            next.effect = effect;
        }
        if let Some(colors) = patch.colors {
            if colors.is_empty() || colors.len() > 2 {
                return Err(Error::InvalidArgument(
                    "Lamp needs one or two colors".into(),
                ));
            }
            next.colors = colors
                .into_iter()
                .map(|c| Rgbw8::new(c[0], c[1], c[2], c[3]))
                .collect();
        }
        Ok(next)
    }
    pub fn render(&self, now: f64) -> Result<Vec<Rgbw8>> {
        if self.brightness == 0.0 || self.effect == "off" {
            return Ok(vec![Rgbw8::OFF; ORION_LIGHT_PIXEL_COUNT]);
        }
        let scale = |color: Rgbw8, gain: f64| {
            Rgbw8::new(
                (f64::from(color.red) * gain).round() as u8,
                (f64::from(color.green) * gain).round() as u8,
                (f64::from(color.blue) * gain).round() as u8,
                (f64::from(color.white) * gain).round() as u8,
            )
        };
        let first = self.colors[0];
        let second = *self.colors.last().unwrap();
        if self.effect == "solid" {
            return Ok(vec![
                scale(first.interpolate(second, 0.5)?, self.brightness);
                ORION_LIGHT_PIXEL_COUNT
            ]);
        }
        render_effect(&self.effect, now, 1.0)?
            .into_iter()
            .enumerate()
            .map(|(index, pixel)| {
                let blend = 0.5 + 0.5 * (now * 0.6 + index as f64 * 0.12).sin();
                let gain = (f64::from(pixel.white) / 191.0).min(1.0) * self.brightness;
                Ok(scale(first.interpolate(second, blend)?, gain))
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn patch(value: serde_json::Value) -> LampPatch {
        serde_json::from_value(value).unwrap()
    }
    #[test]
    fn brightness_changes_preserve_palette_and_effect_and_zero_is_off() {
        let lamp = LampProgram::default()
            .updated(patch(
                serde_json::json!({"effect":"thinking_drift","colors":[[40,20,4,210],[255,0,0,0]]}),
            ))
            .unwrap();
        let dim = lamp
            .updated(patch(serde_json::json!({"brightness":0.5})))
            .unwrap();
        assert_eq!(dim.colors, lamp.colors);
        assert_eq!(dim.effect, lamp.effect);
        assert_ne!(dim.render(0.0).unwrap(), dim.render(1.0).unwrap());
        let off = dim
            .updated(patch(serde_json::json!({"brightness":0})))
            .unwrap();
        assert!(off.render(1.0).unwrap().iter().all(|c| *c == Rgbw8::OFF));
        assert!(
            dim.updated(patch(serde_json::json!({"effect":"made_up"})))
                .is_err()
        );
        assert!(
            dim.updated(patch(serde_json::json!({"brightness":-1})))
                .is_err()
        );
    }
}
