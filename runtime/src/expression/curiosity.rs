//! Shared admission policy for autonomous vision gestures, without IO or motion.
use std::collections::{BTreeMap, VecDeque};

const WINDOW_SECONDS: f64 = 600.0;
const MAX_GESTURES: usize = 3;
const MIN_SPACING_SECONDS: f64 = 60.0;
const FIRST_TARGET_COOLDOWN_SECONDS: f64 = 120.0;
const FORGET_SECONDS: f64 = 3600.0;
const MAX_TARGETS: usize = 32;
const FOCUS_SECONDS: f64 = 120.0;
const OBSERVATION_GAP_SECONDS: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CuriosityKind {
    Glance,
    Gesture,
}

/// Callers supply verified runtime state, never a detector's claimed priority.
/// Default closes admission. A setting being enabled is necessary but not enough.
#[derive(Clone, Copy, Debug, Default)]
pub struct CuriosityContext {
    pub enabled: bool,
    pub camera_on: bool,
    pub tracking_on: bool,
    pub powered_and_awake: bool,
    pub character_available: bool,
    pub stopped: bool,
    pub voice_active: bool,
    pub foreground_active: bool,
    pub quiet_hours: bool,
}

impl CuriosityContext {
    fn open(self) -> bool {
        self.enabled
            && self.camera_on
            && self.tracking_on
            && self.powered_and_awake
            && self.character_available
            && !self.stopped
            && !self.voice_active
            && !self.foreground_active
            && !self.quiet_hours
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CuriosityDenied {
    Unavailable,
    Focus,
    Spacing,
    Window,
    Habituation,
    InvalidClock,
    InvalidTarget,
    Capacity,
}

struct Interest {
    last_reaction: f64,
    repetitions: u32,
}

/// One instance is shared by every autonomous vision behaviour for the lifetime
/// of the character. Camera/mode changes must not reset spent budget or interest.
#[derive(Default)]
pub struct CuriosityBudget {
    gestures: VecDeque<f64>,
    targets: BTreeMap<String, Interest>,
    last_now: Option<f64>,
    focus_since: Option<f64>,
    last_focus_observation: Option<f64>,
}

impl CuriosityBudget {
    fn advance(&mut self, now: f64) -> Result<(), CuriosityDenied> {
        if !now.is_finite() || now < 0.0 || self.last_now.is_some_and(|last| now < last) {
            return Err(CuriosityDenied::InvalidClock);
        }
        self.last_now = Some(now);
        Ok(())
    }

    /// A coarse, fresh face-direction observation: true means a present person
    /// is looking away, false means facing Orion, None means unknown/no person.
    /// Missing observations never count as sustained concentration.
    pub fn observe_focus(
        &mut self,
        now: f64,
        looking_away: Option<bool>,
    ) -> Result<(), CuriosityDenied> {
        self.advance(now)?;
        let continuous = self
            .last_focus_observation
            .is_some_and(|last| now - last <= OBSERVATION_GAP_SECONDS);
        if looking_away == Some(true) {
            if !continuous || self.focus_since.is_none() {
                self.focus_since = Some(now);
            }
        } else {
            self.focus_since = None;
        }
        self.last_focus_observation = Some(now);
        Ok(())
    }

    fn focused(&self, now: f64) -> bool {
        self.last_focus_observation
            .is_some_and(|last| now - last <= OBSERVATION_GAP_SECONDS)
            && self
                .focus_since
                .is_some_and(|since| now - since >= FOCUS_SECONDS)
    }

    /// Spend only after a candidate has passed its own freshness, geometry and
    /// motion-preflight checks. Failed execution still consumes the reservation:
    /// retries must not produce a burst. Target keys are shared across behaviours
    /// and describe local tracks/objects, never biometric identities.
    pub fn try_spend(
        &mut self,
        now: f64,
        target: &str,
        kind: CuriosityKind,
        context: CuriosityContext,
    ) -> Result<(), CuriosityDenied> {
        self.advance(now)?;
        if target.is_empty() || target.len() > 96 || target.trim() != target {
            return Err(CuriosityDenied::InvalidTarget);
        }
        if !context.open() {
            return Err(CuriosityDenied::Unavailable);
        }
        if kind != CuriosityKind::Glance && self.focused(now) {
            return Err(CuriosityDenied::Focus);
        }
        while self
            .gestures
            .front()
            .is_some_and(|at| now - at >= WINDOW_SECONDS)
        {
            self.gestures.pop_front();
        }
        if self.gestures.len() >= MAX_GESTURES {
            return Err(CuriosityDenied::Window);
        }
        if self
            .gestures
            .back()
            .is_some_and(|at| now - at < MIN_SPACING_SECONDS)
        {
            return Err(CuriosityDenied::Spacing);
        }
        self.targets
            .retain(|_, interest| now - interest.last_reaction < FORGET_SECONDS);
        if let Some(interest) = self.targets.get(target) {
            let cooldown = FIRST_TARGET_COOLDOWN_SECONDS
                * 2_f64.powi(interest.repetitions.saturating_sub(1).min(4) as i32);
            if now - interest.last_reaction < cooldown {
                return Err(CuriosityDenied::Habituation);
            }
        } else if self.targets.len() >= MAX_TARGETS {
            return Err(CuriosityDenied::Capacity);
        }
        let interest = self.targets.entry(target.into()).or_insert(Interest {
            last_reaction: now,
            repetitions: 0,
        });
        interest.last_reaction = now;
        interest.repetitions = interest.repetitions.saturating_add(1);
        self.gestures.push_back(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available() -> CuriosityContext {
        CuriosityContext {
            enabled: true,
            camera_on: true,
            tracking_on: true,
            powered_and_awake: true,
            character_available: true,
            ..Default::default()
        }
    }
    fn spend(budget: &mut CuriosityBudget, now: f64, target: &str) -> Result<(), CuriosityDenied> {
        budget.try_spend(now, target, CuriosityKind::Gesture, available())
    }

    #[test]
    fn sliding_window_refills_without_a_boundary_burst() {
        let mut budget = CuriosityBudget::default();
        spend(&mut budget, 0.0, "book").unwrap();
        assert_eq!(
            spend(&mut budget, 59.9, "cup"),
            Err(CuriosityDenied::Spacing)
        );
        spend(&mut budget, 60.0, "cup").unwrap();
        spend(&mut budget, 120.0, "plant").unwrap();
        for now in [180.0, 599.9] {
            assert_eq!(
                spend(&mut budget, now, "phone"),
                Err(CuriosityDenied::Window)
            );
        }
        spend(&mut budget, 600.0, "phone").unwrap();
        assert_eq!(
            spend(&mut budget, 601.0, "dog"),
            Err(CuriosityDenied::Window)
        );
        spend(&mut budget, 660.0, "dog").unwrap();
    }

    #[test]
    fn varied_continuous_stimuli_never_exceed_three_per_ten_minutes() {
        let mut budget = CuriosityBudget::default();
        let mut accepted = Vec::new();
        for second in (0..7200).step_by(17) {
            let now = second as f64;
            if spend(&mut budget, now, &format!("object:{second}")).is_ok() {
                accepted.push(now);
            }
            assert!(accepted.iter().filter(|at| now - **at < 600.0).count() <= 3);
            assert!(budget.targets.len() <= MAX_TARGETS);
            assert!(budget.gestures.len() <= MAX_GESTURES);
        }
        assert!(accepted.len() > 20, "quiet time must restore admission");
    }

    #[test]
    fn habituation_is_shared_across_behaviours_and_fades_after_an_hour() {
        let mut budget = CuriosityBudget::default();
        spend(&mut budget, 0.0, "book").unwrap();
        assert_eq!(
            spend(&mut budget, 60.0, "book"),
            Err(CuriosityDenied::Habituation)
        );
        budget
            .try_spend(120.0, "book", CuriosityKind::Glance, available())
            .unwrap();
        assert_eq!(
            spend(&mut budget, 359.0, "book"),
            Err(CuriosityDenied::Habituation)
        );
        spend(&mut budget, 360.0, "book").unwrap();
        assert_eq!(
            spend(&mut budget, 839.0, "book"),
            Err(CuriosityDenied::Habituation)
        );
        spend(&mut budget, 840.0, "book").unwrap();
        spend(&mut budget, 4440.0, "book").unwrap();
        assert_eq!(budget.targets["book"].repetitions, 1);
    }

    #[test]
    fn focus_needs_continuous_observation_and_allows_only_budgeted_glances() {
        let mut budget = CuriosityBudget::default();
        for now in 0..=120 {
            budget.observe_focus(now as f64, Some(true)).unwrap();
        }
        assert_eq!(
            spend(&mut budget, 120.0, "book"),
            Err(CuriosityDenied::Focus)
        );
        budget
            .try_spend(120.0, "book", CuriosityKind::Glance, available())
            .unwrap();
        assert_eq!(
            budget.try_spend(121.0, "cup", CuriosityKind::Glance, available()),
            Err(CuriosityDenied::Spacing)
        );
        budget.observe_focus(122.0, Some(false)).unwrap();
        assert!(!budget.focused(122.0));
        budget.observe_focus(123.0, Some(true)).unwrap();
        budget.observe_focus(250.0, Some(true)).unwrap();
        assert!(!budget.focused(250.0), "a gap cannot imply sustained focus");
        for now in 251..=370 {
            budget.observe_focus(now as f64, Some(true)).unwrap();
        }
        assert!(budget.focused(370.0));
        assert!(!budget.focused(373.0));
        budget.observe_focus(373.0, None).unwrap();
        assert!(!budget.focused(373.0));
    }

    #[test]
    fn privacy_stop_rest_priority_and_settings_close_admission_without_spending() {
        let mut closed = vec![CuriosityContext::default()];
        let open = available();
        closed.extend([
            CuriosityContext {
                enabled: false,
                ..open
            },
            CuriosityContext {
                camera_on: false,
                ..open
            },
            CuriosityContext {
                tracking_on: false,
                ..open
            },
            CuriosityContext {
                powered_and_awake: false,
                ..open
            },
            CuriosityContext {
                character_available: false,
                ..open
            },
            CuriosityContext {
                stopped: true,
                ..open
            },
            CuriosityContext {
                voice_active: true,
                ..open
            },
            CuriosityContext {
                foreground_active: true,
                ..open
            },
            CuriosityContext {
                quiet_hours: true,
                ..open
            },
        ]);
        let mut budget = CuriosityBudget::default();
        for context in closed {
            assert_eq!(
                budget.try_spend(0.0, "book", CuriosityKind::Glance, context),
                Err(CuriosityDenied::Unavailable)
            );
        }
        assert!(budget.targets.is_empty());
        spend(&mut budget, 0.0, "book").unwrap();
        budget
            .try_spend(
                60.0,
                "cup",
                CuriosityKind::Glance,
                CuriosityContext::default(),
            )
            .unwrap_err();
        assert_eq!(
            spend(&mut budget, 60.0, "book"),
            Err(CuriosityDenied::Habituation)
        );
    }

    #[test]
    fn invalid_clocks_and_unbounded_keys_cannot_refill_or_allocate_state() {
        let mut budget = CuriosityBudget::default();
        spend(&mut budget, 100.0, "book").unwrap();
        for now in [f64::NAN, f64::INFINITY, -1.0, 99.0] {
            assert_eq!(
                spend(&mut budget, now, "cup"),
                Err(CuriosityDenied::InvalidClock)
            );
        }
        for target in ["", " padded", &"a".repeat(97)] {
            assert_eq!(
                spend(&mut budget, 100.0, target),
                Err(CuriosityDenied::InvalidTarget)
            );
        }
        assert_eq!(budget.targets.len(), 1);
        assert_eq!(
            spend(&mut budget, 100.0, "cup"),
            Err(CuriosityDenied::Spacing)
        );
    }
}
