use serde::Serialize;

use crate::{Error, Result};

/// Character timing is artistic policy. It deliberately does not contain
/// servo limits; those come from calibration and the STS3215 profile.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct MotionStyle {
    pub name: &'static str,
    pub tempo: f64,
    pub tangent_tension: f64,
    pub joint_lag: f64,
    pub amplitude: f64,
    pub overshoot_scale: f64,
    pub settle_character: f64,
}

impl MotionStyle {
    pub fn named(name: &str) -> Result<Self> {
        MOTION_STYLES
            .iter()
            .copied()
            .find(|style| style.name == name)
            .ok_or_else(|| Error::InvalidArgument(format!("Unknown Orion motion style: {name}")))
    }
}

pub const MOTION_STYLES: [MotionStyle; 8] = [
    MotionStyle {
        name: "living_idle",
        tempo: 0.82,
        tangent_tension: 0.38,
        joint_lag: 0.18,
        amplitude: 0.9,
        overshoot_scale: 0.0,
        settle_character: 0.85,
    },
    MotionStyle {
        name: "attentive",
        tempo: 1.08,
        tangent_tension: 0.58,
        joint_lag: 0.12,
        amplitude: 1.0,
        overshoot_scale: 0.15,
        settle_character: 0.58,
    },
    MotionStyle {
        name: "expressive_turn",
        tempo: 1.0,
        tangent_tension: 0.72,
        joint_lag: 0.22,
        amplitude: 1.0,
        overshoot_scale: 1.0,
        settle_character: 0.62,
    },
    MotionStyle {
        name: "speaking_calm",
        tempo: 0.72,
        tangent_tension: 0.42,
        joint_lag: 0.16,
        amplitude: 0.95,
        overshoot_scale: 0.0,
        settle_character: 0.82,
    },
    MotionStyle {
        name: "speaking_emphatic",
        tempo: 1.12,
        tangent_tension: 0.62,
        joint_lag: 0.12,
        amplitude: 1.0,
        overshoot_scale: 0.18,
        settle_character: 0.62,
    },
    MotionStyle {
        name: "thinking",
        tempo: 0.68,
        tangent_tension: 0.36,
        joint_lag: 0.24,
        amplitude: 0.62,
        overshoot_scale: 0.08,
        settle_character: 0.88,
    },
    MotionStyle {
        name: "quick_reaction",
        tempo: 1.34,
        tangent_tension: 0.7,
        joint_lag: 0.08,
        amplitude: 0.92,
        overshoot_scale: 0.24,
        settle_character: 0.48,
    },
    MotionStyle {
        name: "return_home",
        tempo: 0.74,
        tangent_tension: 0.32,
        joint_lag: 0.2,
        amplitude: 1.0,
        overshoot_scale: 0.0,
        settle_character: 1.0,
    },
];

pub fn motion_styles() -> &'static [MotionStyle] {
    &MOTION_STYLES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_the_complete_character_style_vocabulary() {
        assert_eq!(motion_styles().len(), 8);
        assert_eq!(
            MotionStyle::named("expressive_turn").unwrap().amplitude,
            1.0
        );
        assert!(MotionStyle::named("hardware_limit").is_err());
    }
}
