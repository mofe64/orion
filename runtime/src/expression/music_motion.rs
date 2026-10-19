use super::{CharacterCoordinator, CharacterState, NextIdleCategory, terminal_phase};
use crate::expression::music::{DanceShape, MusicPulse, plan_with_shape, release};
use crate::expression::settings::DanceMode;
use crate::{MovementPhase, RuntimeCore, RuntimeDriver, RuntimeMode};

#[derive(Debug, Default)]
pub(super) struct MusicMotion {
    pulse: MusicPulse,
    context: bool,
    pub(super) eligible: bool,
    run: Option<u64>,
    resume_phrase: Option<u64>,
    extend_at: f64,
    returning: bool,
    variation: u64,
    dance_until: f64,
    next_dance_at: f64,
    energy: f64,
    bpm: f64,
    shape: DanceShape,
}
impl MusicMotion {
    fn next_phrase(&self) -> u64 {
        (self.pulse.beats / 16.).floor() as u64 + 1
    }
    fn suspend(&mut self) {
        self.resume_phrase = Some(self.next_phrase());
        self.returning = false;
    }
    fn ready(&mut self) -> bool {
        let next = self.next_phrase();
        let phrase = *self.resume_phrase.get_or_insert(next);
        self.pulse.beats >= phrase as f64 * 16.
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
        if !self.music.pulse.fresh() {
            self.music.pulse = MusicPulse::default();
        }
        let mode = self.acts.settings.move_to_music;
        let playable =
            self.music.pulse.audible && self.music.pulse.locked && self.music.pulse.energy >= 0.025;
        let window = mode == DanceMode::Always
            || (mode == DanceMode::Sometimes && now >= self.music.next_dance_at);
        let gaze = self.gaze.status();
        self.music.eligible = self.music.context
            && mode != DanceMode::Off
            && (self.music.run.is_some() || (playable && window && self.music.ready()))
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
            } else if (!self.music.eligible
                || !playable
                || (mode == DanceMode::Sometimes && now >= self.music.dance_until))
                && !self.music.returning
            {
                if mode == DanceMode::Sometimes {
                    self.music.next_dance_at =
                        now + self.rng.range(60., 180. - 60. * self.music.pulse.energy);
                }
                self.release_music(now, core)?;
            }
            if self.music.returning && terminal_phase(core, run).is_some() {
                if self.active_idle_run_id == Some(run) {
                    self.active_idle_run_id = None;
                    self.active_idle_category = None;
                    self.status.active_clip = None;
                }
                self.music.run = None;
                self.music.suspend();
                self.music.eligible = false;
            }
        }

        if !self.music.eligible || !self.music.pulse.audible {
            self.music.resume_phrase = Some(self.music.next_phrase());
        }
        Ok(())
    }

    pub(super) fn release_music<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> crate::Result<bool> {
        let Some(run) = self
            .music
            .run
            .filter(|run| self.active_idle_run_id == Some(*run))
        else {
            return Ok(false);
        };
        if self.music.returning {
            return Ok(true);
        }
        let Some(state) = core.character_commanded_state(run, now) else {
            return Ok(false);
        };
        let Some(anchor) = self.status.active_anchor.clone() else {
            return Ok(false);
        };
        let offsets = state
            .positions
            .iter()
            .map(|(name, q)| (name.clone(), q - anchor[name]))
            .collect();
        let pulse = MusicPulse {
            bpm: self.music.bpm,
            ..self.music.pulse
        };
        let motion = release(pulse, &offsets);
        core.set_character_comfort(core.has_comfort_box());
        let result = core.extend_character_performance(run, motion, anchor, now);
        core.set_character_comfort(self.work_anchor);
        result?;
        self.music.returning = true;
        self.music.resume_phrase = Some(self.music.next_phrase());
        self.status.active_clip = Some("music_release".into());
        Ok(true)
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
                self.music.eligible = false;
            } else {
                if !self.music.returning
                    && self.music.pulse.beats >= self.music.extend_at
                    && core.snapshot().motion.as_ref().is_some_and(|motion| {
                        motion.run_id == run && motion.state == MovementPhase::Executing
                    })
                    && let Some(anchor) = self.status.active_anchor.clone()
                {
                    let (motion, next) = plan_with_shape(
                        self.music.pulse,
                        self.music.variation,
                        self.music.energy,
                        None,
                        self.music.shape,
                    );
                    self.music.energy = self.music.pulse.energy;
                    self.music.bpm = self.music.pulse.bpm;
                    core.set_music_clock(60. / self.music.bpm);
                    core.set_character_comfort(core.has_comfort_box());
                    let result = core.extend_character_performance(run, motion, anchor, now);
                    core.set_character_comfort(self.work_anchor);
                    result?;
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
            return Ok(false);
        }
        // Ordinary idles continue until the next music phrase.
        if !self.music.ready() {
            return Ok(false);
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
        self.music.dance_until = now + self.rng.range(20., 40.);
        let current = core
            .gaze_commanded_state()
            .map(|s| s.positions)
            .unwrap_or_else(|| {
                core.snapshot()
                    .joints
                    .iter()
                    .map(|j| (j.name.clone(), j.position))
                    .collect()
            });
        let start = current
            .iter()
            .map(|(name, q)| (name.clone(), q - anchor[name]))
            .collect();
        self.music.energy = self.music.pulse.energy;
        self.music.bpm = self.music.pulse.bpm;
        self.music.shape = core.music_dance_shape(&anchor)?;
        let (motion, next) = plan_with_shape(
            self.music.pulse,
            self.music.variation,
            self.music.energy,
            Some(&start),
            self.music.shape,
        );
        core.set_music_clock(60. / self.music.bpm);
        // Use the same geometric comfort fitting as desk movements.
        core.set_character_comfort(core.has_comfort_box());
        let result = core.play_generated_anchored_relative(motion, anchor, now);
        core.set_character_comfort(self.work_anchor);
        let run = result?;
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
    fn a_priority_takeover_requires_the_next_phrase_and_silence_never_starts_a_plan() {
        let mut state = MusicMotion::default();
        state.pulse.beats = 5.2;
        state.suspend();
        assert!(!state.ready());
        state.pulse.beats = 7.99;
        assert!(!state.ready());
        state.pulse.beats = 16.;
        assert!(state.ready());
        state.pulse.beats = 17.5;
        state.suspend();
        state.pulse.beats = 31.99;
        assert!(!state.ready());
        state.pulse.beats = 32.;
        assert!(state.ready());
        assert!(!MusicMotion::default().pulse.audible);
    }
}
