use crate::error::{OrionRuntimeError, Result};
use serde::Serialize;

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

/// Motion Style represents settings that influence how Orion performs a movement.
/// They influcne the size, timing and shape of the movement.
/// For example, Orion can perform a gesture slowly and gently, or more quickly and emphatically.
/// The style provides numerical settings that the motion and trajectory code use to produce those differences.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct MotionStyle {
    /// The name of the motion style.
    pub name: &'static str,
    /// Changes travel timing. Higher values shorten the requested travel time; lower values lengthen it.
    pub tempo: f64,
    /// Influences how strongly movement flows through internal keyframes by scaling their calculated velocity and acceleration.
    pub tangent_tension: f64,
    /// Adjusts that flow differently across joints, giving them different movement character. It does not insert a literal time delay.
    pub joint_lag: f64,
    /// Multiplies anchor-relative offsets, making relative gestures larger or smaller. Absolute pose targets are unchanged.
    pub amplitude: f64,
    /// Influences acceleration at internal flowing keyframes. Despite the name, it does not directly specify an overshoot angle or percentage.
    pub overshoot_scale: f64,
    /// Adjusts travel duration for keyframes that settle. Higher values give those arrivals more time, with other settings held equal.
    pub settle_character: f64,
}

impl MotionStyle {
    /// Searches the style presets by name and returns a copy of the matching MotionStyle. If the name is unknown, it returns an error.
    pub fn named(name: &str) -> Result<Self> {
        MOTION_STYLES
            .iter()
            .copied()
            .find(|style| style.name == name)
            .ok_or_else(|| {
                OrionRuntimeError::InvalidArgument(format!("Unknown Orion motion style: {name}"))
            })
    }
}

/// Returns a borrowed view of the whole preset list, allowing callers to discover which styles are available.
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
