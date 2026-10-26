//! One picture of what is happening around Orion, built once per camera update.
//!
//! Every behaviour that decides whether Orion should act reads this picture
//! instead of asking its own questions about people, time, light and work.
use serde::Serialize;

/// How much Orion does on its own. Quiet only reacts with movement, Lively
/// also speaks, and Demo exaggerates both for showing Orion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Liveliness {
    #[default]
    Quiet,
    Lively,
    Demo,
}
impl Liveliness {
    pub fn from_settings(proactive: bool, demo: bool) -> Self {
        if demo {
            Self::Demo
        } else if proactive {
            Self::Lively
        } else {
            Self::Quiet
        }
    }
    pub fn demo(self) -> bool {
        self == Self::Demo
    }
    pub fn speaks(self) -> bool {
        self != Self::Quiet
    }
    /// Nobody in view for this long ends a visit.
    pub fn visit_away_seconds(self) -> f64 {
        if self.demo() { 30.0 } else { 600.0 }
    }
    /// An arrival is noticed only after the room was empty for this long.
    pub fn arrival_empty_seconds(self) -> f64 {
        if self.demo() { 30.0 } else { 600.0 }
    }
    /// A farewell is shown only after a person stayed this long.
    pub fn farewell_stay_seconds(self) -> f64 {
        if self.demo() { 20.0 } else { 600.0 }
    }
}

/// Orion's own light, as the owner set it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LampView {
    pub on: bool,
    /// Percent, 0 to 100.
    pub brightness: f64,
    pub mood: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Situation {
    /// Runtime time of the camera update.
    pub at: f64,
    /// Faces in view.
    pub people: usize,
    /// A face in view looks at Orion.
    pub looking: bool,
    /// The person has looked away from Orion for a long time, as when working.
    pub focused: bool,
    /// When the current visit started. None when nobody is around.
    pub visit: Option<f64>,
    /// Since when nobody is in view.
    pub away_since: Option<f64>,
    /// Local hour with minutes as a fraction.
    pub hour: f64,
    /// Local calendar day.
    pub day: i64,
    /// A voice turn, reply, music, alert, light routine or scene is running.
    pub busy: bool,
    pub lamp: LampView,
    /// Room light from the camera, in lux, when the camera reports it.
    pub room_lux: Option<f64>,
    pub liveliness: Liveliness,
}

/// Conditions only the command layer can see.
#[derive(Clone, Debug, Default)]
pub struct Conditions {
    pub busy: bool,
    pub hour: f64,
    pub day: i64,
    pub lamp: LampView,
}

/// A visit starts when a person comes into view and ends when nobody was in
/// view for a while. At start Orion assumes the current visit is already
/// running, so a restart does not make people already here new.
#[derive(Clone, Debug, Default)]
pub struct VisitTracker {
    visit: Option<f64>,
    away_since: Option<f64>,
    started: bool,
}
impl VisitTracker {
    pub fn observe(&mut self, present: bool, away_seconds: f64, now: f64) {
        if !self.started {
            self.started = true;
            self.visit = Some(now);
        }
        if present {
            self.away_since = None;
            self.visit.get_or_insert(now);
        } else if now - *self.away_since.get_or_insert(now) >= away_seconds {
            self.visit = None;
        }
    }
    pub fn visit(&self) -> Option<f64> {
        self.visit
    }
    pub fn away_since(&self) -> Option<f64> {
        self.away_since
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liveliness_follows_the_two_settings() {
        assert_eq!(Liveliness::from_settings(false, false), Liveliness::Quiet);
        assert_eq!(Liveliness::from_settings(true, false), Liveliness::Lively);
        assert_eq!(Liveliness::from_settings(false, true), Liveliness::Demo);
        assert_eq!(Liveliness::from_settings(true, true), Liveliness::Demo);
        assert!(!Liveliness::Quiet.speaks() && Liveliness::Lively.speaks());
    }

    #[test]
    fn a_visit_survives_short_absences_and_ends_after_a_long_one() {
        let mut visits = VisitTracker::default();
        visits.observe(false, 600.0, 0.0);
        let first = visits.visit();
        assert_eq!(first, Some(0.0), "a restart keeps the current visit");
        visits.observe(true, 600.0, 10.0);
        visits.observe(false, 600.0, 20.0);
        visits.observe(true, 600.0, 500.0);
        assert_eq!(visits.visit(), first);
        visits.observe(false, 600.0, 510.0);
        visits.observe(false, 600.0, 1110.0);
        assert_eq!(visits.visit(), None);
        assert_eq!(visits.away_since(), Some(510.0));
        visits.observe(true, 600.0, 1200.0);
        assert_eq!(visits.visit(), Some(1200.0));
    }
}
