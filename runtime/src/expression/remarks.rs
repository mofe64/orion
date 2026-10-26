//! When Orion may speak about what it noticed. Pure policy: no IO, no motion.
//!
//! A performed social reaction offers one remark. Speaking is much rarer than
//! moving: proactive mode spaces remarks far apart, backs off when people
//! ignore them, and never interrupts conversation, music, alerts or quiet hours.
//! Demo mode keeps the same events with short spacing for showing Orion.
use crate::expression::settings::{Chattiness, Quirks, Rules};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemarkEvent {
    PersonArrived,
    PersonLooking,
    NewObject,
    HandRaised,
    PersonLeft,
    /// The room got dark while Orion's light is off or low.
    LightCare,
    /// Someone has been at the desk a long time without a break.
    BreakNudge,
    /// Someone left late in the evening.
    Goodnight,
}
impl RemarkEvent {
    /// A welcome or a look speaks to the person; once per visit is enough.
    fn about_person(self) -> bool {
        matches!(self, Self::PersonArrived | Self::PersonLooking)
    }
    /// Care for the person: said at most once on each visit.
    fn care(self) -> bool {
        matches!(self, Self::LightCare | Self::BreakNudge)
    }
    /// A goodbye is said to someone who is leaving: no reply is expected.
    fn parting(self) -> bool {
        matches!(self, Self::PersonLeft | Self::Goodnight)
    }
    /// Everyday per-event cooldowns, in seconds.
    fn cooldown(self, demo: bool) -> f64 {
        match (self, demo) {
            (_, true) => 45.0,
            (Self::PersonArrived, false) => 1800.0,
            (Self::PersonLooking, false) => 1200.0,
            (Self::NewObject | Self::PersonLeft, false) => 3600.0,
            (Self::HandRaised, false) => f64::INFINITY,
            (Self::LightCare, false) => 3.0 * 3600.0,
            (Self::BreakNudge, false) => 3600.0,
            (Self::Goodnight, false) => 4.0 * 3600.0,
        }
    }
}

/// Everyday objects that sit on a desk all day are not news.
const ORDINARY_OBJECTS: [&str; 14] = [
    "laptop",
    "keyboard",
    "mouse",
    "cell phone",
    "tv",
    "chair",
    "dining table",
    "cup",
    "bottle",
    "remote",
    "couch",
    "bed",
    "potted plant",
    "book",
];
const DEMO_SPACING_SECONDS: f64 = 30.0;
const DEMO_AFTER_CONVERSATION_SECONDS: f64 = 20.0;
/// An object must stay this long before an everyday remark about it.
const OBJECT_DWELL_SECONDS: f64 = 12.0;
/// A request that waits longer than this after its due time is dropped.
const STALE_SECONDS: f64 = 8.0;
/// A remark without a reply in this time counts as ignored.
const REPLY_SECONDS: f64 = 45.0;
const MAX_IGNORED: u32 = 2;
const MAX_QUIET_SECONDS: f64 = 8.0 * 3600.0;
#[derive(Clone, Copy, Debug, Default)]
pub struct RemarkContext {
    pub proactive: bool,
    pub demo: bool,
    /// A voice turn, reply playback, music, alert, light routine or scene.
    pub busy: bool,
    /// The person has looked away from Orion for a long time.
    pub focused: bool,
    pub hour: f64,
    /// Local calendar day, for once-a-day object memory.
    pub day: i64,
    /// When the current visit started; see [`VisitTracker`](super::situation::VisitTracker).
    pub visit: Option<f64>,
}
impl From<&super::situation::Situation> for RemarkContext {
    fn from(situation: &super::situation::Situation) -> Self {
        Self {
            proactive: situation.liveliness == super::situation::Liveliness::Lively,
            demo: situation.liveliness.demo(),
            busy: situation.busy,
            focused: situation.focused,
            hour: situation.hour,
            day: situation.day,
            visit: situation.visit,
        }
    }
}
impl RemarkContext {
    fn enabled(self) -> bool {
        self.proactive || self.demo
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PendingRemark {
    pub id: u64,
    pub event: RemarkEvent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub demo: bool,
    /// The reply model writes the line in this lamp's style.
    pub quirks: Quirks,
    #[serde(skip)]
    due: f64,
    #[serde(skip)]
    object: Option<String>,
}

#[derive(Debug, Default)]
pub struct RemarkPolicy {
    next_id: u64,
    pending: Option<PendingRemark>,
    spoken: VecDeque<f64>,
    by_event: BTreeMap<RemarkEvent, f64>,
    by_category: BTreeMap<String, f64>,
    seen_today: (i64, BTreeSet<String>),
    ignored: u32,
    awaiting_reply: Option<f64>,
    quiet_until: f64,
    last_conversation: Option<f64>,
    /// The visit in which Orion last spoke to or with the people.
    spoken_visit: Option<f64>,
    /// Care remarks already said on one visit.
    cared: (Option<f64>, BTreeSet<RemarkEvent>),
    started: bool,
    context: RemarkContext,
    quirks: Quirks,
    rules: Rules,
}

impl RemarkPolicy {
    /// A reserved Orion speaks half as often as a normal one; a chatty one
    /// twice as often, up to six times an hour.
    pub fn set_quirks(&mut self, quirks: Quirks) {
        self.quirks = quirks;
    }
    /// The owner's rules for a Lively Orion.
    pub fn set_rules(&mut self, rules: Rules) {
        self.rules = rules;
    }
    fn hourly_limit(&self) -> usize {
        let base = self.rules.per_hour as usize;
        match self.quirks.chattiness {
            Chattiness::Reserved => (base / 2).max(1),
            Chattiness::Normal => base,
            Chattiness::Chatty => base + base.div_ceil(2),
        }
    }
    /// The owner switched this kind of remark off.
    fn switched_off(&self, event: RemarkEvent) -> bool {
        let rules = &self.rules;
        match event {
            RemarkEvent::PersonLooking => !rules.looks,
            RemarkEvent::NewObject => !rules.new_things,
            RemarkEvent::LightCare => !rules.light_care,
            RemarkEvent::BreakNudge => !rules.break_nudge,
            RemarkEvent::Goodnight => !rules.goodnight,
            RemarkEvent::PersonArrived | RemarkEvent::HandRaised | RemarkEvent::PersonLeft => false,
        }
    }
    pub fn set_context(&mut self, context: RemarkContext, now: f64) {
        if !self.started {
            // After a restart or update, people already here are not new.
            self.started = true;
            self.spoken_visit = context.visit;
        }
        if context.demo && !self.context.demo {
            // Each demo take starts fresh.
            self.by_event.clear();
            self.by_category.clear();
            self.spoken.clear();
            self.spoken_visit = None;
            self.cared = (None, BTreeSet::new());
        }
        self.context = context;
        if !context.enabled() || context.busy {
            self.pending = None;
        }
        if context.busy {
            self.last_conversation = Some(now);
        }
        if self
            .awaiting_reply
            .is_some_and(|at| now - at >= REPLY_SECONDS)
        {
            self.awaiting_reply = None;
            if !self.context.demo {
                self.ignored = (self.ignored + 1).min(MAX_IGNORED);
            }
        }
    }
    pub fn demo(&self) -> bool {
        self.context.demo
    }
    pub fn enabled(&self) -> bool {
        self.context.enabled()
    }
    /// Orion already spoke to or with the people on this visit.
    fn visit_spoken(&self) -> bool {
        self.context.visit.is_some() && self.spoken_visit == self.context.visit
    }

    /// A confirmed voice turn is engagement: a reply resets the back-off.
    /// Returns whether the turn answered a remark.
    pub fn engaged(&mut self, now: f64) -> bool {
        let answered = self.awaiting_reply.is_some();
        self.ignored = 0;
        self.awaiting_reply = None;
        self.last_conversation = Some(now);
        self.pending = None;
        self.spoken_visit = self.context.visit;
        answered
    }

    pub fn quiet(&mut self, seconds: f64, now: f64) -> Result<f64, &'static str> {
        if !seconds.is_finite() || !(0.0..=MAX_QUIET_SECONDS).contains(&seconds) {
            return Err("Quiet time must be 0 to 8 hours");
        }
        self.quiet_until = now + seconds;
        self.pending = None;
        Ok(self.quiet_until)
    }

    fn spacing(&self) -> f64 {
        if self.context.demo {
            DEMO_SPACING_SECONDS
        } else {
            let base = self.rules.spacing_minutes * 60.0;
            let spacing = match self.quirks.chattiness {
                Chattiness::Reserved => 2.0 * base,
                Chattiness::Normal => base,
                Chattiness::Chatty => base / 2.0,
            };
            spacing * 2_f64.powi(self.ignored as i32)
        }
    }

    /// Admission for one performed reaction. Returns whether a remark is queued.
    pub fn offer(
        &mut self,
        event: RemarkEvent,
        detail: Option<&str>,
        object: Option<&str>,
        now: f64,
    ) -> bool {
        self.consider(event, detail, object, now).is_ok()
    }

    /// Like [`offer`](Self::offer), with the reason a remark was not queued.
    pub fn consider(
        &mut self,
        event: RemarkEvent,
        detail: Option<&str>,
        object: Option<&str>,
        now: f64,
    ) -> Result<(), &'static str> {
        while self.spoken.front().is_some_and(|at| now - at >= 3600.0) {
            self.spoken.pop_front();
        }
        if let Some(reason) = self.refusal(event, now) {
            return Err(reason);
        }
        let context = self.context;
        let demo = context.demo;
        let detail = detail.map(str::trim).filter(|text| !text.is_empty());
        if event == RemarkEvent::NewObject {
            let Some(category) = detail else {
                return Err("no_detail");
            };
            if demo {
                if self
                    .by_category
                    .get(category)
                    .is_some_and(|at| now - at < 120.0)
                {
                    return Err("cooldown");
                }
            } else {
                if self.seen_today.0 != context.day {
                    self.seen_today = (context.day, BTreeSet::new());
                }
                if ORDINARY_OBJECTS.contains(&category) {
                    return Err("ordinary_object");
                }
                if self.seen_today.1.contains(category) {
                    return Err("seen_today");
                }
            }
        }
        self.next_id += 1;
        self.pending = Some(PendingRemark {
            id: self.next_id,
            event,
            detail: detail.map(|text| text.chars().take(32).collect()),
            demo,
            quirks: self.quirks,
            due: if event == RemarkEvent::NewObject && !demo {
                now + OBJECT_DWELL_SECONDS
            } else {
                now
            },
            object: object.map(str::to_owned),
        });
        Ok(())
    }

    /// Why a remark about this event cannot be queued now, without queuing it.
    pub fn refusal(&self, event: RemarkEvent, now: f64) -> Option<&'static str> {
        let context = self.context;
        let demo = context.demo;
        let gap = if demo {
            DEMO_AFTER_CONVERSATION_SECONDS
        } else {
            self.rules.after_talk_minutes * 60.0
        };
        let hourly = self.spoken.iter().filter(|at| now - **at < 3600.0).count();
        if !context.enabled() {
            Some("quiet_mode")
        } else if context.busy {
            Some("busy")
        } else if self.switched_off(event) {
            Some("switched_off")
        } else if self.pending.is_some() {
            Some("pending")
        } else if now < self.quiet_until {
            Some("asked_quiet")
        } else if event == RemarkEvent::HandRaised && !demo {
            Some("demo_only")
        } else if event.about_person() && self.visit_spoken() {
            Some("once_per_visit")
        } else if event.care()
            && context.visit.is_some()
            && self.cared.0 == context.visit
            && self.cared.1.contains(&event)
        {
            Some("once_per_visit")
        } else if !demo && context.focused && !event.care() {
            Some("focus")
        } else if !demo && self.rules.quiet_at(context.hour) && event != RemarkEvent::Goodnight {
            Some("night")
        } else if event != RemarkEvent::Goodnight
            && self.last_conversation.is_some_and(|at| now - at < gap)
        {
            Some("after_conversation")
        } else if event != RemarkEvent::Goodnight
            && self
                .spoken
                .back()
                .is_some_and(|at| now - at < self.spacing())
        {
            Some(if self.ignored > 0 {
                "ignored"
            } else {
                "spacing"
            })
        } else if self
            .by_event
            .get(&event)
            .is_some_and(|at| now - at < event.cooldown(demo))
        {
            Some("cooldown")
        } else if !demo && event != RemarkEvent::Goodnight && hourly >= self.hourly_limit() {
            Some("hourly_limit")
        } else {
            None
        }
    }

    /// The remark the listener may start now. An object remark waits for its
    /// dwell time and is dropped if the object left. Old requests are dropped.
    pub fn ready(&mut self, now: f64, present: impl Fn(&str) -> bool) -> Option<&PendingRemark> {
        let remark = self.pending.as_ref()?;
        if now < remark.due {
            return None;
        }
        if now - remark.due > STALE_SECONDS
            || !self.context.enabled()
            || self.context.busy
            || now < self.quiet_until
            || remark.object.as_deref().is_some_and(|id| !present(id))
        {
            self.pending = None;
            return None;
        }
        self.pending.as_ref()
    }

    /// The listener opened its speaking session for this remark.
    pub fn take(&mut self, id: u64, now: f64) -> Result<PendingRemark, &'static str> {
        if self.pending.as_ref().is_none_or(|remark| remark.id != id) {
            return Err("No current remark has this ID");
        }
        if self.pending.as_ref().unwrap().due > now {
            return Err("The remark is not due yet");
        }
        let remark = self.pending.take().unwrap();
        self.spoken.push_back(now);
        self.by_event.insert(remark.event, now);
        if remark.event.about_person() {
            self.spoken_visit = self.context.visit;
        }
        if remark.event.care() {
            if self.cared.0 != self.context.visit {
                self.cared = (self.context.visit, BTreeSet::new());
            }
            self.cared.1.insert(remark.event);
        }
        if let Some(category) = &remark.detail
            && remark.event == RemarkEvent::NewObject
        {
            self.by_category.insert(category.clone(), now);
            if !remark.demo {
                self.seen_today.1.insert(category.clone());
            }
        }
        if !remark.event.parting() {
            self.awaiting_reply = Some(now);
        }
        Ok(remark)
    }

    pub fn status(&self, now: f64) -> serde_json::Value {
        serde_json::json!({
            "proactive": self.context.proactive,
            "demo": self.context.demo,
            "quiet_seconds": (self.quiet_until - now).max(0.0),
            "ignored": self.ignored,
            "visit_spoken": self.visit_spoken(),
            "pending": self.pending,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(demo: bool) -> RemarkPolicy {
        let mut policy = RemarkPolicy::default();
        policy.set_context(
            RemarkContext {
                proactive: true,
                demo,
                hour: 12.0,
                day: 1,
                visit: Some(0.0),
                ..Default::default()
            },
            0.0,
        );
        policy.spoken_visit = None;
        policy
    }
    /// A new visit starts at `now`; reply tracking is left as it was.
    fn new_visit(policy: &mut RemarkPolicy, now: f64) {
        policy.context.visit = Some(now);
    }
    fn speak(policy: &mut RemarkPolicy, now: f64) -> RemarkEvent {
        let id = policy.ready(now, |_| true).unwrap().id;
        policy.take(id, now).unwrap().event
    }

    #[test]
    fn everyday_remarks_are_rare_spaced_and_limited_per_event() {
        let mut policy = policy(false);
        assert!(policy.offer(RemarkEvent::PersonArrived, None, None, 1000.0));
        assert_eq!(speak(&mut policy, 1000.0), RemarkEvent::PersonArrived);
        // Ten minutes between any two remarks.
        new_visit(&mut policy, 1500.0);
        assert!(!policy.offer(RemarkEvent::PersonLooking, None, None, 1500.0));
        assert!(policy.offer(RemarkEvent::PersonLooking, None, None, 1600.0));
        speak(&mut policy, 1600.0);
        // The same event waits its own cooldown.
        new_visit(&mut policy, 2300.0);
        assert!(!policy.offer(RemarkEvent::PersonArrived, None, None, 2300.0));
        assert!(policy.offer(RemarkEvent::PersonArrived, None, None, 2800.0));
        // Hand play belongs to demo mode only.
        let mut policy = self::policy(false);
        assert!(!policy.offer(RemarkEvent::HandRaised, None, None, 1000.0));
    }

    #[test]
    fn everyday_objects_wait_dwell_skip_ordinary_things_and_speak_once_a_day() {
        let mut policy = policy(false);
        assert!(!policy.offer(
            RemarkEvent::NewObject,
            Some("laptop"),
            Some("object:1"),
            1000.0
        ));
        assert!(policy.offer(
            RemarkEvent::NewObject,
            Some("banana"),
            Some("object:2"),
            1000.0
        ));
        assert!(policy.ready(1005.0, |_| true).is_none());
        // Gone before the dwell ended: dropped.
        assert!(policy.ready(1012.0, |_| false).is_none());
        assert!(policy.offer(
            RemarkEvent::NewObject,
            Some("banana"),
            Some("object:3"),
            1013.0
        ));
        let id = policy.ready(1025.0, |id| id == "object:3").unwrap().id;
        policy.take(id, 1025.0).unwrap();
        assert!(!policy.offer(
            RemarkEvent::NewObject,
            Some("banana"),
            Some("object:4"),
            9000.0
        ));
        policy.context.day = 2;
        assert!(policy.offer(
            RemarkEvent::NewObject,
            Some("banana"),
            Some("object:4"),
            9000.0
        ));
    }

    #[test]
    fn ignored_remarks_back_off_and_a_reply_resets() {
        let mut policy = policy(false);
        let context = policy.context;
        assert!(policy.offer(RemarkEvent::PersonArrived, None, None, 1000.0));
        speak(&mut policy, 1000.0);
        policy.set_context(context, 1050.0);
        assert_eq!(policy.ignored, 1);
        new_visit(&mut policy, 2100.0);
        assert!(!policy.offer(RemarkEvent::PersonLooking, None, None, 2100.0));
        assert!(policy.offer(RemarkEvent::PersonLooking, None, None, 2200.0));
        speak(&mut policy, 2200.0);
        policy.engaged(2210.0);
        assert_eq!(policy.ignored, 0);
        // Conversation gap after the reply.
        assert!(!policy.offer(RemarkEvent::NewObject, Some("banana"), None, 2400.0));
    }

    #[test]
    fn busy_quiet_focus_and_night_close_admission() {
        let mut policy = policy(false);
        let mut context = policy.context;
        context.focused = true;
        policy.set_context(context, 1000.0);
        assert!(!policy.offer(RemarkEvent::PersonArrived, None, None, 1000.0));
        context.focused = false;
        context.hour = 23.0;
        policy.set_context(context, 1000.0);
        assert!(!policy.offer(RemarkEvent::PersonArrived, None, None, 1000.0));
        context.hour = 12.0;
        policy.set_context(context, 1000.0);
        policy.quiet(600.0, 1000.0).unwrap();
        assert!(!policy.offer(RemarkEvent::PersonArrived, None, None, 1500.0));
        assert!(policy.offer(RemarkEvent::PersonArrived, None, None, 1700.0));
        context.busy = true;
        policy.set_context(context, 1701.0);
        assert!(policy.ready(1701.0, |_| true).is_none());
        assert!(policy.quiet(-1.0, 0.0).is_err());
        let mut off = RemarkPolicy::default();
        assert!(!off.offer(RemarkEvent::PersonArrived, None, None, 1000.0));
    }

    #[test]
    fn the_owners_rules_set_quiet_hours_and_switch_remarks_off() {
        let mut policy = policy(false);
        policy.set_rules(Rules {
            quiet_from: 20 * 60,
            quiet_until: 9 * 60,
            looks: false,
            ..Rules::default()
        });
        let mut context = policy.context;
        context.hour = 8.5;
        policy.set_context(context, 1000.0);
        assert_eq!(
            policy.refusal(RemarkEvent::PersonArrived, 1000.0),
            Some("night")
        );
        context.hour = 12.0;
        policy.set_context(context, 1000.0);
        assert_eq!(
            policy.refusal(RemarkEvent::PersonLooking, 1000.0),
            Some("switched_off")
        );
        assert_eq!(policy.refusal(RemarkEvent::PersonArrived, 1000.0), None);
    }

    #[test]
    fn demo_speaks_often_about_everything_and_drops_stale_requests() {
        let mut policy = policy(true);
        assert!(policy.offer(
            RemarkEvent::NewObject,
            Some("cup"),
            Some("object:1"),
            1000.0
        ));
        assert_eq!(speak(&mut policy, 1000.0), RemarkEvent::NewObject);
        assert!(!policy.offer(RemarkEvent::HandRaised, None, None, 1020.0));
        assert!(policy.offer(RemarkEvent::HandRaised, None, None, 1031.0));
        assert!(policy.ready(1040.0, |_| true).is_none());
        new_visit(&mut policy, 1041.0);
        assert!(policy.offer(RemarkEvent::PersonLooking, None, None, 1041.0));
        assert!(policy.take(99, 1041.0).is_err());
        speak(&mut policy, 1041.0);
        // Turning demo mode on again clears the memory of a take.
        let mut context = policy.context;
        context.demo = false;
        policy.set_context(context, 1042.0);
        context.demo = true;
        policy.set_context(context, 1043.0);
        assert!(policy.offer(RemarkEvent::NewObject, Some("cup"), None, 1043.0));
    }

    #[test]
    fn orion_speaks_about_a_person_once_per_visit() {
        use crate::expression::situation::{Liveliness, VisitTracker};
        for demo in [false, true] {
            let away = Liveliness::from_settings(true, demo).visit_away_seconds(&Rules::default());
            let mut policy = policy(demo);
            let mut visits = VisitTracker::default();
            let mut context = policy.context;
            let mut observe = |policy: &mut RemarkPolicy, present: bool, now: f64| {
                visits.observe(present, away, now);
                context.visit = visits.visit();
                policy.set_context(context, now);
            };
            observe(&mut policy, false, 0.0);
            observe(&mut policy, false, away + 1.0);
            observe(&mut policy, true, 1000.0);
            assert!(policy.offer(RemarkEvent::PersonArrived, None, None, 1000.0));
            speak(&mut policy, 1000.0);
            // Still here an hour later: no more welcomes or looks.
            for now in [1100.0, 2500.0, 4700.0] {
                observe(&mut policy, true, now);
                assert!(
                    !policy.offer(RemarkEvent::PersonLooking, None, None, now),
                    "{demo}"
                );
            }
            // Objects are still news.
            assert!(policy.offer(RemarkEvent::NewObject, Some("banana"), None, 4700.0));
            policy.pending = None;
            // A short step away is the same visit.
            observe(&mut policy, false, 4710.0);
            observe(&mut policy, true, 4710.0 + away - 1.0);
            assert!(!policy.offer(RemarkEvent::PersonLooking, None, None, 4710.0 + away));
            // Away long enough: a new visit.
            observe(&mut policy, false, 5000.0);
            observe(&mut policy, false, 5000.0 + away);
            observe(&mut policy, true, 5000.0 + away);
            assert!(policy.offer(RemarkEvent::PersonArrived, None, None, 5000.0 + away));
        }
        // A conversation counts as speaking with the person.
        let mut policy = policy(true);
        policy.engaged(1000.0);
        assert!(!policy.offer(RemarkEvent::PersonLooking, None, None, 1100.0));
    }

    #[test]
    fn a_restart_does_not_welcome_people_who_are_already_here() {
        let mut policy = RemarkPolicy::default();
        let context = RemarkContext {
            proactive: true,
            hour: 12.0,
            visit: Some(1000.0),
            ..Default::default()
        };
        policy.set_context(context, 1000.0);
        assert!(!policy.offer(RemarkEvent::PersonLooking, None, None, 1000.0));
    }

    #[test]
    fn chattiness_quirk_changes_how_often_lively_orion_speaks() {
        for (chattiness, gap, limit) in [
            (Chattiness::Reserved, 1200.0, 2),
            (Chattiness::Normal, 600.0, 4),
            (Chattiness::Chatty, 300.0, 6),
        ] {
            let mut policy = policy(false);
            policy.set_quirks(Quirks {
                chattiness,
                ..Default::default()
            });
            let mut spoken = 0;
            for step in 0..12 {
                let now = 1000.0 + step as f64 * gap / 2.0;
                if policy.offer(
                    RemarkEvent::NewObject,
                    Some(&format!("thing{step}")),
                    None,
                    now,
                ) {
                    let id = policy.ready(now + 12.0, |_| true).unwrap().id;
                    policy.take(id, now + 12.0).unwrap();
                    policy.awaiting_reply = None;
                    spoken += 1;
                }
            }
            assert!(spoken > 0 && spoken <= 6, "{chattiness:?}");
            assert!(policy.hourly_limit() == limit);
            assert!((policy.spacing() - gap).abs() < 1e-9, "{chattiness:?}");
        }
    }

    #[test]
    fn care_remarks_come_once_a_visit_even_while_people_work() {
        let mut policy = policy(false);
        policy.context.focused = true;
        assert_eq!(
            policy.consider(RemarkEvent::PersonLooking, None, None, 1000.0),
            Err("focus")
        );
        assert!(policy.offer(RemarkEvent::BreakNudge, None, None, 1000.0));
        speak(&mut policy, 1000.0);
        assert!(policy.offer(RemarkEvent::LightCare, Some("light off"), None, 2000.0));
        speak(&mut policy, 2000.0);
        assert_eq!(
            policy.consider(RemarkEvent::BreakNudge, None, None, 9000.0),
            Err("once_per_visit")
        );
        new_visit(&mut policy, 9000.0);
        assert!(policy.offer(RemarkEvent::BreakNudge, None, None, 9000.0));
    }

    #[test]
    fn goodnight_is_said_late_right_after_talking_and_expects_no_reply() {
        let mut policy = policy(false);
        policy.context.hour = 23.5;
        assert_eq!(
            policy.consider(RemarkEvent::PersonLeft, None, None, 1000.0),
            Err("night")
        );
        policy.engaged(1000.0);
        assert!(policy.offer(RemarkEvent::Goodnight, None, None, 1010.0));
        speak(&mut policy, 1010.0);
        let context = policy.context;
        policy.set_context(context, 1100.0);
        assert_eq!(policy.ignored, 0, "nobody is there to answer a goodnight");
        assert_eq!(
            policy.consider(RemarkEvent::Goodnight, None, None, 1200.0),
            Err("cooldown")
        );
    }

    #[test]
    fn a_refused_remark_says_why() {
        let mut policy = policy(false);
        assert_eq!(
            policy.consider(RemarkEvent::NewObject, Some("laptop"), None, 1000.0),
            Err("ordinary_object")
        );
        assert!(policy.offer(RemarkEvent::PersonArrived, None, None, 1000.0));
        speak(&mut policy, 1000.0);
        assert_eq!(
            policy.consider(RemarkEvent::PersonLooking, None, None, 1100.0),
            Err("once_per_visit")
        );
        assert_eq!(
            policy.consider(RemarkEvent::NewObject, Some("banana"), None, 1100.0),
            Err("spacing")
        );
        policy.context.hour = 23.0;
        assert_eq!(
            policy.consider(RemarkEvent::NewObject, Some("banana"), None, 5000.0),
            Err("night")
        );
    }
}
