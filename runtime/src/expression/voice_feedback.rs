use crate::Rgbw8;
use serde::Serialize;
use std::collections::VecDeque;

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
    confirmed: bool,
    verifying_wake: bool,
    processing_cued: bool,
    acknowledgment_since: Option<f64>,
    since: f64,
    deadline: f64,
    retired: VecDeque<String>,
    history: VecDeque<(String, String, f64)>,
}
impl VoiceFeedback {
    /// Confirmation requires an explicit prefix verification or an endpointed
    /// wake. A raw candidate alone cannot extend the inactivity deadline.
    pub fn confirm(&mut self, id: &str, now: f64) -> bool {
        if !self.owns(id)
            || self.confirmed
            || !(self.phase == "thinking" || (self.phase == "listening" && self.verifying_wake))
        {
            return false;
        }
        self.confirmed = true;
        self.acknowledgment_since = Some(now);
        self.record("confirmed", now);
        true
    }

    pub fn confirmed_activity(&self) -> bool {
        self.session.is_some() && self.confirmed && self.phase != "unavailable"
    }

    pub fn reaction(&self) -> &'static str {
        if !self.confirmed {
            return "neutral";
        }
        match self.phase.as_str() {
            "listening" | "window" => "listening",
            "thinking" => "thinking",
            _ => "neutral",
        }
    }

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
        self.confirmed = false;
        self.verifying_wake = false;
        self.acknowledgment_since = None;
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
        if matches!(event, "wake" | "continue") {
            if self.session.is_some() || self.retired.iter().any(|old| old == id) {
                return Ok(None);
            }
            self.session = Some(id.into());
            self.confirmed = event == "continue";
            self.phase = "listening".into();
            self.since = if event == "continue" { now - 1.0 } else { now };
            self.deadline = now + 120.0;
            self.processing_cued = false;
            self.acknowledgment_since = None;
            self.record(event, now);
            return Ok(self.confirmed.then_some(("listening", None)));
        }
        if self.session.as_deref() != Some(id) {
            return Ok(None);
        }
        match event {
            "verify" if self.phase == "listening" && !self.confirmed => {
                self.verifying_wake = true;
                self.record("wake_verification", now);
                Ok(None)
            }
            "processing" if matches!(self.phase.as_str(), "speaking" | "thinking") => {
                self.phase = "thinking".into();
                self.since = now;
                self.deadline = now + 180.0;
                self.record("tool_processing", now);
                Ok(Some(("thinking", None)))
            }
            "guard" if self.phase == "speaking" => {
                self.phase = "guard".into();
                self.deadline = now + 3.0;
                self.record("guard", now);
                Ok(Some(("neutral", None)))
            }
            "window" if self.phase == "guard" => {
                self.phase = "window".into();
                self.since = now;
                self.deadline = now + 5.0;
                self.record("window", now);
                Ok(Some(("listening", None)))
            }
            "endpoint" if self.phase == "listening" => {
                self.phase = "thinking".into();
                self.since = now;
                self.record("endpoint", now);
                if !self.confirmed {
                    return Ok(None);
                }
                self.record("thinking_start", now);
                let cue = if self.processing_cued || self.acknowledging(now) {
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
                let visible = self.confirmed;
                self.record(event, now);
                self.clear();
                Ok(visible.then_some(("neutral", None)))
            }
            "unavailable" if self.phase != "unavailable" => {
                self.phase = "unavailable".into();
                self.since = now;
                self.deadline = now + 0.8;
                self.record("unavailable", now);
                Ok(self.confirmed.then_some(("neutral", Some("error_muted"))))
            }
            "endpoint" | "followup" | "unavailable" | "guard" | "window" | "verify" => Ok(None),
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
    pub fn acknowledging(&self, now: f64) -> bool {
        self.acknowledgment_since.is_some_and(|at| now - at < 0.9)
    }
    pub fn light(&self, now: f64) -> Option<Rgbw8> {
        if !self.confirmed {
            return None;
        }
        let elapsed = (now - self.since).max(0.0);
        let (color, gain) =
            if matches!(self.phase.as_str(), "listening" | "thinking") && self.acknowledging(now) {
                // The acknowledgement owns its own clock, so endpoint/followup cannot
                // replace or restart its original three-color pattern.
                let elapsed = (now - self.acknowledgment_since.unwrap()).max(0.0);
                let index = (elapsed / 0.3).floor() as usize;
                (
                    VOICE_PALETTE[index.min(2)],
                    (std::f64::consts::PI * (elapsed % 0.3) / 0.3).sin().powi(2),
                )
            } else {
                match self.phase.as_str() {
                    "window" => (
                        VOICE_PALETTE[1],
                        0.55 - 0.30 * (elapsed * std::f64::consts::TAU / 1.4).cos(),
                    ),
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
                }
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
    fn unconfirmed_candidates_never_produce_feedback_including_endpoint_and_failure() {
        for ending in ["reject", "cancel", "unavailable"] {
            let mut feedback = VoiceFeedback::default();
            for (event, at) in [
                ("wake", 0.0),
                ("verify", 0.2),
                ("endpoint", 2.0),
                (ending, 3.0),
            ] {
                assert!(feedback.event(ID, event, at).unwrap().is_none(), "{event}");
                assert!(feedback.light(at + 0.15).is_none(), "{event}");
                assert_eq!(feedback.reaction(), "neutral");
            }
        }
    }

    #[test]
    fn confirmation_replays_original_ack_pattern_once_despite_endpoint_and_followup() {
        for verification in ["verify", "endpoint"] {
            let mut f = VoiceFeedback::default();
            f.event(ID, "wake", 0.0).unwrap();
            f.event(ID, verification, 0.2).unwrap();
            assert!(f.light(2.0).is_none());
            assert!(f.confirm(ID, 10.0));
            // The full-transcript path may already be thinking; neither transition
            // is allowed to cut short the acknowledgement or play over its chime.
            assert_eq!(
                f.event(ID, "endpoint", 10.02)
                    .unwrap()
                    .and_then(|(_, cue)| cue),
                None
            );
            f.event(ID, "followup", 10.04).unwrap();
            assert!(!f.confirm(ID, 10.1));
            for (offset, expected) in [
                (0.15, VOICE_PALETTE[0]),
                (0.45, VOICE_PALETTE[1]),
                (0.75, VOICE_PALETTE[2]),
            ] {
                let actual = f.light(10.0 + offset).unwrap();
                for (a, b) in [
                    (actual.red, expected.red),
                    (actual.green, expected.green),
                    (actual.blue, expected.blue),
                    (actual.white, expected.white),
                ] {
                    assert!(a.abs_diff(b) <= 1);
                }
            }
            assert!(!f.acknowledging(10.91));
            assert_eq!(f.light(10.91), Some(Rgbw8::new(4, 3, 0, 12)));
        }
    }

    #[test]
    fn prefix_confirmation_requires_verification_and_keeps_listening() {
        let mut feedback = VoiceFeedback::default();
        feedback.event(ID, "wake", 1.0).unwrap();
        assert!(!feedback.confirm(ID, 1.1));
        feedback.event(ID, "verify", 1.2).unwrap();
        assert!(feedback.confirm(ID, 2.0));
        assert_eq!(feedback.reaction(), "listening");
        assert!(!feedback.confirm(ID, 2.1));
        feedback.event(ID, "endpoint", 5.0).unwrap();
        assert_eq!(feedback.reaction(), "thinking");
        feedback.event(ID, "cancel", 5.1).unwrap();
        assert!(!feedback.confirm(ID, 5.2));
    }
    #[test]
    fn intermediate_speech_returns_to_silent_processing_animation() {
        let mut f = VoiceFeedback::default();
        f.event(ID, "wake", 0.0).unwrap();
        f.event(ID, "verify", 0.05).unwrap();
        assert!(f.confirm(ID, 0.1));
        f.event(ID, "endpoint", 1.0).unwrap();
        f.playback_started(2.0);
        assert_eq!(
            f.event(ID, "processing", 3.0).unwrap(),
            Some(("thinking", None))
        );
        assert!(f.light(3.0).is_some());
        assert_ne!(f.light(3.0), f.light(4.0));
        f.playback_started(5.0);
        assert_eq!(f.event(ID, "guard", 6.0).unwrap(), Some(("neutral", None)));
    }
    #[test]
    fn conversation_invitation_is_silent_teal_bounded_and_session_scoped() {
        let mut f = VoiceFeedback::default();
        f.event(ID, "wake", 0.0).unwrap();
        f.event(ID, "verify", 0.05).unwrap();
        assert!(f.confirm(ID, 0.1));
        f.playback_started(1.0);
        assert_eq!(f.event(ID, "guard", 2.0).unwrap(), Some(("neutral", None)));
        assert!(f.light(2.2).is_none());
        assert_eq!(
            f.event(ID, "window", 2.8).unwrap(),
            Some(("listening", None))
        );
        let low = f.light(2.8).unwrap();
        let high = f.light(3.5).unwrap();
        assert!(high.green > high.red && high.blue > high.red);
        assert!(high.green > low.green);
        assert!(f.event(ID, "window", 3.0).unwrap().is_none());
        assert!(f.expire(7.8));
        assert!(f.light(7.8).is_none());
        let next = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        assert_eq!(
            f.event(next, "continue", 8.0).unwrap(),
            Some(("listening", None))
        );
        f.event(ID, "cancel", 8.1).unwrap();
        assert!(f.owns(next));
        assert!(f.event(next, "endpoint", 9.0).unwrap().is_some());
    }
    #[test]
    fn thinking_breathes_in_all_acknowledgment_colors_without_flashes() {
        let mut feedback = VoiceFeedback::default();
        feedback.event(ID, "wake", 0.0).unwrap();
        feedback.event(ID, "verify", 0.05).unwrap();
        assert!(feedback.confirm(ID, 0.1));
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
        feedback.event(ID, "verify", 0.05).unwrap();
        assert!(feedback.confirm(ID, 0.1));
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
        assert!(f.event(ID, "wake", 0.0).unwrap().is_none());
        f.event(ID, "verify", 0.05).unwrap();
        assert!(f.confirm(ID, 0.1));
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
