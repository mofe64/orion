//! One path for what Orion does on its own initiative.
//!
//! Every behaviour reads the same [`Situation`](crate::expression::situation::Situation),
//! asks [`admit`](CharacterCoordinator::admit) for room in the shared curiosity budget,
//! and performs through [`perform`](CharacterCoordinator::perform). Words follow
//! the movement through the remark policy, which reads the same situation.
use super::reactions::{Reaction, ReactionKind, Stage};
use super::*;
use crate::expression::curiosity::{CuriosityContext, CuriosityDenied, CuriosityKind};
use crate::expression::remarks::RemarkEvent;

/// One reaction Orion wants to perform.
pub(super) struct Act {
    pub kind: ReactionKind,
    /// The budget key: one person, one object or one kind of event.
    pub key: String,
    pub motion: MotionDefinition,
    pub anchor: JointPositions,
    /// The posture the reaction returns to.
    pub centre: JointPositions,
    pub track: u64,
    pub category: Option<String>,
    pub object: Option<(String, f64, f64, f64)>,
    /// The reaction settles by this many seconds after it starts.
    pub seconds: f64,
    pub behaviour: &'static str,
    pub target: Option<String>,
    pub target_category: Option<String>,
    pub clip: &'static str,
    /// The remark this reaction offers: event, detail and object ID.
    pub remark: Option<(RemarkEvent, Option<String>, Option<String>)>,
}

impl CharacterCoordinator {
    /// No reaction runs and Orion is free to move on its own.
    pub(super) fn may_start_reaction<D: RuntimeDriver>(&self, core: &RuntimeCore<D>) -> bool {
        self.surroundings.reaction.is_none() && self.reaction_ready(core)
    }

    /// Admission against the shared budget. Callers check availability first.
    pub(super) fn admit(
        &mut self,
        now: f64,
        key: &str,
        kind: CuriosityKind,
    ) -> std::result::Result<(), CuriosityDenied> {
        let context = CuriosityContext {
            enabled: true,
            camera_on: true,
            tracking_on: true,
            powered_and_awake: true,
            character_available: true,
            ..Default::default()
        };
        self.surroundings.budget.try_spend(now, key, kind, context)
    }

    /// Records a reaction Orion chose not to start, for Studio history.
    pub(super) fn skip(
        &self,
        behaviour: &str,
        target: Option<String>,
        category: Option<String>,
        reason: &str,
        now: f64,
    ) {
        if let Some(log) = &self.behaviour_log {
            log.skipped(behaviour, target, category, reason, now);
        }
    }

    /// A reaction found its moment taken: another reaction runs or Orion is busy.
    pub(super) fn busy_reason(&self) -> &'static str {
        if self.surroundings.reaction.is_some() {
            "reaction_running"
        } else {
            "busy"
        }
    }

    /// Admits and starts a reaction. Returns false when the budget declined.
    pub(super) fn perform<D: RuntimeDriver>(
        &mut self,
        act: Act,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<bool> {
        if let Err(denied) = self.admit(now, &act.key, CuriosityKind::Gesture) {
            self.skip(
                act.behaviour,
                act.target,
                act.target_category,
                denied.code(),
                now,
            );
            return Ok(false);
        }
        self.start_act(act, now, core)?;
        Ok(true)
    }

    /// Starts a reaction without the curiosity budget. Care for the person,
    /// such as a break nudge, is not curiosity and comes while people work.
    pub(super) fn start_act<D: RuntimeDriver>(
        &mut self,
        act: Act,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        self.preempt_idle_or_thinking(now, core)?;
        let run = core.play_generated_anchored_relative(act.motion, act.anchor, now)?;
        self.surroundings.reaction = Some(Reaction {
            kind: act.kind,
            category: act.category,
            track: act.track,
            started: now,
            deadline: now + act.seconds,
            run: Some(run),
            stage: Stage::Notice,
            centre: act.centre,
            object: act.object,
        });
        if let Some(log) = &self.behaviour_log {
            log.motion(act.behaviour, run, act.target, act.target_category, now);
        }
        self.status.active_clip = Some(act.clip.into());
        self.reset_timers(now);
        if let Some((event, detail, object)) = act.remark {
            self.offer_remark(event, detail.as_deref(), object.as_deref(), now);
        }
        Ok(())
    }
}
