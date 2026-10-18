use super::{CharacterCoordinator, CharacterState, NextIdleCategory, checked, terminal_phase};
use crate::expression::music::{MusicPulse, performance};
use crate::{MovementPhase, RuntimeCore, RuntimeDriver, RuntimeMode};

#[derive(Debug, Default)]
pub(super) struct MusicMotion {
    pulse: MusicPulse,
    context: bool,
    pub(super) eligible: bool,
    run: Option<u64>,
    resume_bar: Option<u64>,
    extend_at: f64,
    returning: bool,
    variation: u64,
}
impl MusicMotion {
    fn next_bar(&self) -> u64 {
        (self.pulse.beats / 4.).floor() as u64 + 1
    }
    fn suspend(&mut self) {
        self.resume_bar = Some(self.next_bar());
        self.returning = false;
    }
    fn ready(&mut self) -> bool {
        let next = self.next_bar();
        let bar = *self.resume_bar.get_or_insert(next);
        self.pulse.beats >= bar as f64 * 4.
    }
}

impl CharacterCoordinator {
    pub(crate) fn set_music_input(&mut self, pulse: Option<MusicPulse>, context: bool) {
        let beginning = !self.music.pulse.audible || !self.music.context;
        // None means the mixer lock is busy; only a published pulse can report silence.
        if let Some(pulse) = pulse {
            self.music.pulse = pulse;
        }
        self.music.context = context;
        if beginning && context && self.music.pulse.audible {
            self.music.suspend();
        }
    }

    pub(super) fn music_priority<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> crate::Result<()> {
        let gaze = self.gaze.status();
        self.music.eligible = self.music.context
            && (self.music.pulse.streaming || self.music.pulse.audible || self.music.pulse.locked)
            && self.acts.settings.move_to_music
            && self.status.enabled
            && matches!(
                self.status.state,
                CharacterState::HomeIdle | CharacterState::PoseIdle
            )
            && self.starting_run_id.is_none()
            && !self.foreground_pending
            && self.foreground_scene_run_id.is_none()
            && self.attention.is_none()
            && self.work.is_none()
            && !self.work_anchor
            && gaze.run_id.is_none()
            && gaze.track_id.is_none()
            && self.surroundings.reaction.is_none()
            && self.acts.lifecycle.is_none()
            && self.acts.drift.is_none();
        if let Some(run) = self.music.run {
            if self.active_idle_run_id != Some(run) {
                self.music.run = None;
                self.music.suspend();
            } else if !self.music.eligible {
                if core
                    .snapshot()
                    .motion
                    .as_ref()
                    .is_some_and(|motion| motion.run_id == run)
                {
                    checked(core.handle_command("stop", now))?;
                }
                self.active_idle_run_id = None;
                self.active_idle_category = None;
                self.status.active_clip = None;
                self.music.run = None;
                self.music.suspend();
            } else if !self.music.pulse.audible && !self.music.returning {
                if core.snapshot().motion.as_ref().is_some_and(|motion| {
                    motion.run_id == run && motion.state == MovementPhase::Executing
                }) && let Some(anchor) = self.status.active_anchor.clone()
                {
                    let (mut motion, _) = performance(self.music.pulse, self.music.variation);
                    motion.keyframes = vec![motion.keyframes.pop().expect("final return")];
                    core.extend_character_performance(run, motion, anchor, now)?;
                }
                self.music.returning = true;
                self.music.resume_bar = Some(self.music.next_bar());
            }
        }
        if !self.music.eligible || !self.music.pulse.audible {
            self.music.resume_bar = Some(self.music.next_bar());
        }
        Ok(())
    }

    pub(super) fn tick_music<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> crate::Result<bool> {
        if let Some(run) = self.music.run {
            if terminal_phase(core, run).is_some() {
                if self.active_idle_run_id == Some(run) {
                    self.active_idle_run_id = None;
                    self.active_idle_category = None;
                    self.status.active_clip = None;
                }
                self.music.run = None;
                self.music.suspend();
            } else {
                if !self.music.returning
                    && self.music.pulse.beats >= self.music.extend_at
                    && core.snapshot().motion.as_ref().is_some_and(|motion| {
                        motion.run_id == run && motion.state == MovementPhase::Executing
                    })
                    && let Some(anchor) = self.status.active_anchor.clone()
                {
                    let (motion, next) = performance(self.music.pulse, self.music.variation);
                    core.extend_character_performance(run, motion, anchor, now)?;
                    self.music.extend_at = next;
                }
                return Ok(true);
            }
        }
        if !self.music.eligible
            || !self.music.pulse.audible
            || !self.music.pulse.locked
            || self.music.pulse.energy < 0.025
        {
            return Ok(self.music.eligible);
        }
        // Reserve background motion while waiting for the next estimated bar.
        if !self.music.ready() {
            return Ok(true);
        }
        if self.active_idle_run_id.is_some() {
            self.preempt_idle_or_thinking(now, core)?;
        }
        if core.mode() != RuntimeMode::Holding {
            return Ok(false);
        }
        let Some(anchor) = self.status.active_anchor.clone() else {
            return Ok(false);
        };
        self.music.variation = self.music.variation.wrapping_add(1);
        let (motion, next) = performance(self.music.pulse, self.music.variation);
        let run = core.play_generated_anchored_relative(motion, anchor, now)?;
        self.music.run = Some(run);
        self.music.returning = false;
        self.music.extend_at = next;
        self.active_idle_run_id = Some(run);
        self.active_idle_category = Some(NextIdleCategory::Micro);
        self.status.active_clip = Some("music_performance".into());
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_priority_takeover_requires_the_next_bar_and_silence_never_starts_a_plan() {
        let mut state = MusicMotion::default();
        state.pulse.beats = 5.2;
        state.suspend();
        assert!(!state.ready());
        state.pulse.beats = 7.99;
        assert!(!state.ready());
        state.pulse.beats = 8.;
        assert!(state.ready());
        state.pulse.beats = 9.5;
        state.suspend();
        state.pulse.beats = 11.99;
        assert!(!state.ready());
        state.pulse.beats = 12.;
        assert!(state.ready());
        assert!(!MusicMotion::default().pulse.audible);
    }
}
