//! Session-scoped, optional local expression. Device ownership stays in oriond.
use crate::Rgbw8;
use serde::Serialize;
use std::collections::VecDeque;

// Both voice states share one palette; wake pulses are brief, thinking breathes.
const VOICE_PALETTE: [Rgbw8; 3] = [
    Rgbw8::new(65, 32, 6, 8),
    Rgbw8::new(8, 45, 38, 8),
    Rgbw8::new(35, 22, 48, 8),
];
const THINKING_BREATH_SECONDS: f64 = 3.0;

#[derive(Default, Serialize)]
pub struct VoiceFeedback {
    session: Option<String>,
    phase: String,
    processing_cued: bool,
    since: f64,
    deadline: f64,
    retired: VecDeque<String>,
    history: VecDeque<(String, String, f64)>,
}
impl VoiceFeedback {
    pub fn owns(&self, id: &str) -> bool {
        self.session.as_deref() == Some(id)
    }
    fn record(&mut self, event: &str, now: f64) {
        if let Some(id) = &self.session {
            self.history.push_back((id.clone(), event.into(), now));
            if self.history.len() > 128 {
                self.history.pop_front();
            }
        }
    }
    pub fn cue_started(&mut self, cue: &str, now: f64) {
        self.record(
            if cue == "voice_wake" {
                "acknowledgment_start"
            } else if cue == "error_muted" {
                "unavailable_cue_start"
            } else {
                "processing_cue_start"
            },
            now,
        );
    }
    pub fn clear(&mut self) {
        if let Some(id) = self.session.take() {
            self.retired.push_back(id);
            if self.retired.len() > 128 {
                self.retired.pop_front();
            }
        }
        self.phase.clear();
    }
    pub fn expire(&mut self, now: f64) -> bool {
        if self.session.is_some() && now >= self.deadline {
            self.record("expired", now);
            self.clear();
            true
        } else {
            false
        }
    }
    pub fn event(
        &mut self,
        id: &str,
        event: &str,
        now: f64,
    ) -> Result<Option<(&'static str, Option<&'static str>)>, &'static str> {
        if id.len() != 32 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid voice session ID");
        }
        if event == "wake" {
            if self.session.is_some() || self.retired.iter().any(|old| old == id) {
                return Ok(None);
            }
            self.session = Some(id.into());
            self.phase = "listening".into();
            self.since = now;
            self.deadline = now + 120.0;
            self.processing_cued = false;
            self.record("wake", now);
            return Ok(Some(("listening", Some("voice_wake"))));
        }
        if self.session.as_deref() != Some(id) {
            return Ok(None);
        }
        match event {
            "endpoint" if self.phase == "listening" => {
                self.phase = "thinking".into();
                self.since = now;
                self.record("endpoint", now);
                self.record("thinking_start", now);
                let cue = if self.processing_cued {
                    None
                } else {
                    Some("voice_processing")
                };
                self.processing_cued = true;
                Ok(Some(("thinking", cue)))
            }
            "followup" if self.phase == "thinking" => {
                self.phase = "listening".into();
                self.since = now - 1.0;
                Ok(Some(("listening", None)))
            }
            "first_chunk" => {
                if !self
                    .history
                    .iter()
                    .any(|(old, event, _)| old == id && event == "first_chunk")
                {
                    self.record("first_chunk", now);
                }
                Ok(None)
            }
            "finish" | "cancel" | "reject" => {
                self.record(event, now);
                self.clear();
                Ok(Some(("neutral", None)))
            }
            "unavailable" if self.phase != "unavailable" => {
                self.phase = "unavailable".into();
                self.since = now;
                self.deadline = now + 0.8;
                self.record("unavailable", now);
                Ok(Some(("neutral", Some("error_muted"))))
            }
            "endpoint" | "followup" | "unavailable" => Ok(None),
            _ => Err("Unknown voice event"),
        }
    }
    pub fn playback_started(&mut self, now: f64) {
        if self.session.is_some() && self.phase != "speaking" {
            self.phase = "speaking".into();
            self.deadline = now + 180.0;
            self.record("playback_start", now);
        }
    }
    pub fn light(&self, now: f64) -> Option<Rgbw8> {
        let elapsed = (now - self.since).max(0.0);
        let (color, gain) = match self.phase.as_str() {
            "listening" if elapsed < 0.9 => {
                let index = (elapsed / 0.3).floor() as usize;
                (
                    VOICE_PALETTE[index.min(2)],
                    (std::f64::consts::PI * (elapsed % 0.3) / 0.3).sin().powi(2),
                )
            }
            "listening" => (
                Rgbw8 {
                    red: 4,
                    green: 3,
                    blue: 0,
                    white: 12,
                },
                1.0,
            ),
            "thinking" => {
                let cycle = elapsed / THINKING_BREATH_SECONDS;
                let index = cycle.floor() as usize % VOICE_PALETTE.len();
                let progress = cycle.fract();
                let fade = progress * progress * (3.0 - 2.0 * progress);
                let color = VOICE_PALETTE[index]
                    .interpolate(VOICE_PALETTE[(index + 1) % VOICE_PALETTE.len()], fade)
                    .expect("bounded palette interpolation");
                // Stay visibly lit at the trough; no flashing or white washout.
                let gain = 0.90 - 0.45 * (cycle * std::f64::consts::TAU).cos();
                (color, gain)
            }
            "unavailable" => (
                Rgbw8 {
                    red: 35,
                    green: 8,
                    blue: 0,
                    white: 3,
                },
                (1.0 - elapsed / 0.8).max(0.0),
            ),
            _ => return None,
        };
        Some(Rgbw8 {
            red: (color.red as f64 * gain) as u8,
            green: (color.green as f64 * gain) as u8,
            blue: (color.blue as f64 * gain) as u8,
            white: (color.white as f64 * gain) as u8,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    #[test]
    fn thinking_breathes_in_all_acknowledgment_colors_without_flashes() {
        let mut feedback = VoiceFeedback::default();
        feedback.event(ID, "wake", 0.0).unwrap();
        feedback.event(ID, "endpoint", 1.0).unwrap();
        let amber = feedback.light(1.0).unwrap();
        let teal = feedback.light(4.0).unwrap();
        let lavender = feedback.light(7.0).unwrap();
        assert!(amber.red > amber.green && amber.green > amber.blue);
        assert!(teal.green > teal.red && teal.blue > teal.red);
        assert!(lavender.blue > lavender.red && lavender.red > lavender.green);
        assert_eq!(amber, feedback.light(10.0).unwrap());
        let mut previous = amber;
        let mut peak = 0;
        for frame in 1..=450 {
            let color = feedback.light(1.0 + frame as f64 * 0.02).unwrap();
            let rgb = [color.red, color.green, color.blue];
            assert!(*rgb.iter().max().unwrap() > color.white);
            assert!(rgb.iter().map(|value| *value as u16).sum::<u16>() > 20);
            peak = peak.max(*rgb.iter().max().unwrap());
            for (before, after) in [
                (previous.red, color.red),
                (previous.green, color.green),
                (previous.blue, color.blue),
                (previous.white, color.white),
            ] {
                assert!(
                    before.abs_diff(after) <= 3,
                    "abrupt color or brightness change"
                );
            }
            previous = color;
        }
        assert!(peak > 55, "breath must rise visibly above its trough");
        feedback.playback_started(11.0);
        assert!(feedback.light(11.0).is_none());
    }

    #[test]
    fn unavailable_plays_error_once_and_releases_feedback() {
        let mut feedback = VoiceFeedback::default();
        feedback.event(ID, "wake", 0.0).unwrap();
        assert_eq!(
            feedback.event(ID, "unavailable", 2.0).unwrap(),
            Some(("neutral", Some("error_muted")))
        );
        assert!(feedback.event(ID, "unavailable", 2.1).unwrap().is_none());
        assert!(feedback.expire(2.9));
        assert!(feedback.light(2.9).is_none());
    }

    #[test]
    fn cues_once_and_stale_events_cannot_replace_a_turn() {
        let mut f = VoiceFeedback::default();
        assert!(f.event(ID, "wake", 0.0).unwrap().is_some());
        assert!(f.event(ID, "wake", 0.1).unwrap().is_none());
        assert!(f.event(ID, "endpoint", 1.0).unwrap().is_some());
        assert!(f.event(ID, "endpoint", 1.1).unwrap().is_none());
        assert!(f.light(20.0).is_some());
        f.playback_started(21.0);
        assert!(f.light(21.0).is_none());
        f.event(ID, "finish", 25.0).unwrap();
        assert!(f.event(ID, "wake", 26.0).unwrap().is_none());
        let next = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        f.event(next, "wake", 27.0).unwrap();
        f.event(ID, "cancel", 28.0).unwrap();
        assert_eq!(f.session.as_deref(), Some(next));
        assert!(f.expire(148.0));
    }
}
