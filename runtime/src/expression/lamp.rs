use crate::{
    Error, Result,
    lighting::{LIGHTING_EFFECT_NAMES, ORION_LIGHT_PIXEL_COUNT, Rgbw8, render_effect_for_pixels},
};
use serde::Deserialize;
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LampPatch {
    pub on: Option<bool>,
    pub brightness: Option<f64>,
    pub effect: Option<String>,
    pub colors: Option<Vec<[u8; 4]>>,
    pub transition_seconds: Option<f64>,
}
#[derive(Clone)]
pub struct LampProgram {
    revision: u64,
    enabled: bool,
    brightness: f64,
    effect: String,
    effect_started_at: f64,
    colors: Vec<Rgbw8>,
    transition: Option<Transition>,
}
#[derive(Clone)]
struct Transition {
    start: f64,
    seconds: f64,
    brightness: f64,
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
            revision: NEXT_REVISION.fetch_add(1, Ordering::Relaxed),
            enabled: color != Rgbw8::OFF,
            brightness: 1.0,
            effect: "solid".into(),
            effect_started_at: 0.,
            colors: vec![color],
            transition: None,
        }
    }
    pub fn updated(&self, patch: LampPatch) -> Result<Self> {
        self.updated_at(patch, 0.)
    }
    pub fn updated_at(&self, patch: LampPatch, now: f64) -> Result<Self> {
        let seconds = patch.transition_seconds.unwrap_or(0.);
        if !seconds.is_finite() || !(0. ..=3600.).contains(&seconds) {
            return Err(Error::InvalidArgument(
                "Transition must be 0–3600 seconds".into(),
            ));
        }
        let current = self.sample(now)?;
        if patch.on.is_none()
            && patch.brightness.is_none()
            && patch.effect.is_none()
            && patch.colors.is_none()
        {
            return Err(Error::InvalidArgument("Empty lamp update".into()));
        }
        let mut next = if patch.on.is_some()
            && patch.brightness.is_none()
            && patch.effect.is_none()
            && patch.colors.is_none()
        {
            self.clone()
        } else {
            current.clone()
        };
        next.transition = None;
        if let Some(on) = patch.on {
            next.enabled = on;
            if on && !self.enabled {
                next.effect_started_at = now;
            }
            if on && next.effect == "off" {
                next.effect = "solid".into();
            }
            if on && next.brightness == 0. {
                next.brightness = 1.;
            }
        } else if patch.brightness.is_some() || patch.colors.is_some() || patch.effect.is_some() {
            next.enabled = true;
        }
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
            next.effect_started_at = now;
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
        if seconds > 0. {
            next.transition = Some(Transition {
                start: now,
                seconds,
                brightness: current.brightness,
                colors: current.colors,
            });
        }
        next.revision = NEXT_REVISION.fetch_add(1, Ordering::Relaxed);
        Ok(next)
    }
    fn sample(&self, now: f64) -> Result<Self> {
        let mut current = self.clone();
        current.transition = None;
        if let Some(t) = &self.transition {
            let amount = ((now - t.start) / t.seconds).clamp(0., 1.);
            if amount >= 1. {
                return Ok(current);
            }
            current.brightness = t.brightness + (self.brightness - t.brightness) * amount;
            current.colors = (0..t.colors.len().max(self.colors.len()))
                .map(|i| {
                    t.colors[i.min(t.colors.len() - 1)]
                        .interpolate(self.colors[i.min(self.colors.len() - 1)], amount)
                })
                .collect::<Result<Vec<_>>>()?;
        }
        Ok(current)
    }
    pub fn finish(&self) -> Self {
        let mut target = self.clone();
        target.transition = None;
        target
    }
    pub fn freeze(&self, now: f64) -> Result<Self> {
        self.sample(now)
    }
    pub fn status_at(&self, now: f64) -> serde_json::Value {
        self.sample(now).unwrap_or_else(|_| self.clone()).status()
    }

    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({"brightness":self.brightness * 100.0, "effect":self.effect,
            "colors":self.colors.iter().map(|c| [c.red,c.green,c.blue,c.white]).collect::<Vec<_>>()})
    }
    /// Owner controls show the chosen target, not a speech frame or fade sample.
    pub fn owner_status_at(&self, now: f64, takeover: Option<&str>) -> serde_json::Value {
        let mut status = self.status();
        static EPOCH: OnceLock<String> = OnceLock::new();
        status["epoch"] = serde_json::json!(EPOCH.get_or_init(|| {
            format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            )
        }));
        status["effect_elapsed_seconds"] =
            serde_json::json!((now - self.effect_started_at).max(0.));
        status["revision"] = serde_json::json!(self.revision);
        status["on"] = serde_json::json!(
            self.enabled
                && self.brightness > 0.
                && self.effect != "off"
                && self.colors.iter().any(|c| *c != Rgbw8::OFF)
        );
        let palette: Vec<_> = self
            .colors
            .iter()
            .map(|c| [c.red, c.green, c.blue, c.white])
            .collect();
        status["mood"] = serde_json::json!(match palette.as_slice() {
            [[40, 20, 4, 210]] | [[0, 0, 0, 255]] => "warm",
            [[255, 100, 0, 80]] => "ambient",
            [[150, 190, 255, 110], [0, 45, 255, 0]] => "cool",
            [[40, 20, 4, 210], [255, 0, 0, 0]] => "warm_red",
            _ => "custom",
        });
        status["takeover"] = serde_json::json!(takeover);
        status["fade"] = self.transition.as_ref().filter(|t| now < t.start+t.seconds).map(|t| {
            let sample=self.status_at(now);
            serde_json::json!({"remaining_seconds":(t.start+t.seconds-now).max(0.),"brightness":sample["brightness"],"colors":sample["colors"]})
        }).unwrap_or(serde_json::Value::Null);
        status
    }
    pub fn render(&self, now: f64) -> Result<Vec<Rgbw8>> {
        self.render_for_pixels(now, ORION_LIGHT_PIXEL_COUNT)
    }
    pub fn render_for_pixels(&self, now: f64, pixel_count: usize) -> Result<Vec<Rgbw8>> {
        if self.transition.is_some() {
            return self.sample(now)?.render_for_pixels(now, pixel_count);
        }
        if !self.enabled || self.brightness == 0.0 || self.effect == "off" {
            return Ok(vec![Rgbw8::OFF; pixel_count]);
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
                pixel_count
            ]);
        }
        let elapsed = (now - self.effect_started_at).max(0.);
        render_effect_for_pixels(&self.effect, elapsed, 1.0, pixel_count)?
            .into_iter()
            .enumerate()
            .map(|(index, pixel)| {
                let blend = 0.5 + 0.5 * (elapsed * 0.6 + index as f64 * 0.12).sin();
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
    fn selected_pulses_start_bright_even_after_a_long_daemon_uptime() {
        for effect in ["acknowledge_pulse", "settle_glow"] {
            let lamp = LampProgram::default()
                .updated_at(patch(serde_json::json!({"effect":effect})), 7001.25)
                .unwrap();
            assert_eq!(
                lamp.owner_status_at(7001.25, None)["effect_elapsed_seconds"],
                0.
            );
            for count in [24, 40] {
                let first: u32 = lamp
                    .render_for_pixels(7001.25, count)
                    .unwrap()
                    .iter()
                    .map(|p| u32::from(p.white))
                    .sum();
                let later: u32 = lamp
                    .render_for_pixels(7002.25, count)
                    .unwrap()
                    .iter()
                    .map(|p| u32::from(p.white))
                    .sum();
                assert!(
                    first > later + count as u32 * 50,
                    "{effect} must start its own visible pulse"
                );
            }
        }
    }

    #[test]
    fn owner_status_keeps_the_target_during_fades_and_power_preserves_the_palette() {
        let lamp=LampProgram::default().updated_at(patch(serde_json::json!({"brightness":0.4,"colors":[[255,0,0,0]],"effect":"thinking_drift","transition_seconds":10})),0.).unwrap();
        let status = lamp.owner_status_at(5., Some("speech"));
        assert_eq!(status["brightness"], 40.);
        assert_eq!(status["colors"], serde_json::json!([[255, 0, 0, 0]]));
        assert_eq!(status["fade"]["brightness"], 70.);
        assert_eq!(status["fade"]["remaining_seconds"], 5.);
        assert_eq!(status["takeover"], "speech");
        assert!(lamp.owner_status_at(11., None)["fade"].is_null());
        let off = lamp
            .updated_at(patch(serde_json::json!({"on":false})), 11.)
            .unwrap();
        let off_status = off.owner_status_at(11., None);
        assert_eq!(off_status["epoch"], status["epoch"]);
        assert!(off_status["revision"].as_u64().unwrap() > status["revision"].as_u64().unwrap());
        assert_eq!(off.owner_status_at(11., None)["on"], false);
        assert_eq!(off.status()["brightness"], 40.);
        assert!(off.render(11.).unwrap().iter().all(|p| *p == Rgbw8::OFF));
        let on = off
            .updated_at(patch(serde_json::json!({"on":true})), 12.)
            .unwrap();
        assert_eq!(on.owner_status_at(12., None)["on"], true);
        assert_eq!(on.status(), lamp.status());
    }
    #[test]
    fn transitions_interpolate_and_new_commands_start_from_the_current_frame() {
        let start = LampProgram::from_color(Rgbw8::new(0, 0, 0, 0));
        let fade=start.updated_at(patch(serde_json::json!({"brightness":0.5,"colors":[[100,200,40,80]],"transition_seconds":100})),10.).unwrap();
        assert_eq!(fade.status_at(60.)["brightness"], 75.);
        assert_eq!(
            fade.status_at(60.)["colors"][0],
            serde_json::json!([50, 100, 20, 40])
        );
        let cancelled = fade
            .updated_at(patch(serde_json::json!({"brightness":0.2})), 60.)
            .unwrap();
        assert_eq!(cancelled.status_at(200.)["brightness"], 20.);
        assert_eq!(
            cancelled.status_at(200.)["colors"][0],
            serde_json::json!([50, 100, 20, 40])
        );
        assert!(
            fade.updated_at(
                patch(serde_json::json!({"brightness":1,"transition_seconds":3601})),
                0.
            )
            .is_err()
        );
    }
    #[test]
    fn status_exposes_the_manual_palette_effect_and_percent_brightness() {
        let lamp = LampProgram::default()
            .updated(patch(serde_json::json!({
                "brightness":0.35,"effect":"thinking_drift","colors":[[1,2,3,4],[5,6,7,8]]
            })))
            .unwrap();
        assert_eq!(
            lamp.status(),
            serde_json::json!({"brightness":35.0,"effect":"thinking_drift","colors":[[1,2,3,4],[5,6,7,8]]})
        );
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
