//! State-led acts use the character's existing motion ownership and compiler.
use super::*;
use crate::expression::{mind::AttentionTarget, settings::ActSettings};

#[derive(Debug)]
pub(super) struct Acts {
    pub settings: ActSettings,
    pub lifecycle: Option<u64>,
    pub drift: Option<(u64, String)>,
    pub drifted: bool,
    drift_at: f64,
    pub allowed: bool,
    pub rest_due: Option<f64>,
    pub thinking_at: f64,
    next_beat: f64,
    last_beat: Option<bool>,
    listening_attempted: bool,
}
impl Default for Acts {
    fn default() -> Self {
        Self {
            settings: ActSettings::default(),
            lifecycle: None,
            drift: None,
            drifted: false,
            drift_at: f64::INFINITY,
            allowed: false,
            rest_due: None,
            thinking_at: 0.,
            next_beat: 0.,
            last_beat: None,
            listening_attempted: false,
        }
    }
}

impl CharacterCoordinator {
    pub(crate) fn set_act_context(&mut self, allowed: bool, rest_due: Option<f64>) {
        self.acts.allowed = allowed;
        self.acts.rest_due = rest_due;
    }
    pub(crate) fn set_act_settings(&mut self, settings: ActSettings) {
        self.acts.settings = settings;
    }
    pub(crate) fn act_sounds_enabled(&self) -> bool {
        self.acts.settings.act_sounds
    }
    pub(crate) fn wake_sleep_acts_enabled<D: RuntimeDriver>(&self, core: &RuntimeCore<D>) -> bool {
        self.acts.settings.wake_sleep_acts && core.motions().motion("wake_stretch").is_ok()
    }
    pub(crate) fn drifted(&self) -> bool {
        self.acts.drifted || self.acts.drift.is_some()
    }
    pub(crate) fn finish_lifecycle_act(&mut self) {
        self.acts.lifecycle = None;
        self.status.active_clip = None;
    }
    pub(crate) fn play_lifecycle_act<D: RuntimeDriver>(
        &mut self,
        clip: &str,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<u64> {
        self.preempt_idle_or_thinking(now, core)?;
        let anchor = self
            .status
            .active_anchor
            .clone()
            .ok_or_else(|| Error::InvalidState("No act anchor".into()))?;
        let run = core.play_anchored_relative(clip, anchor, now)?;
        self.acts.lifecycle = Some(run);
        self.status.active_clip = Some(clip.into());
        if let Some(log) = &self.behaviour_log {
            log.motion(clip, run, None, None, now);
        }
        Ok(run)
    }
    pub(crate) fn return_home_before_rest<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<u64> {
        self.preempt_for_foreground(now, core)?;
        if core.mode() == RuntimeMode::Moving {
            checked(core.handle_command("stop", now))?;
        }
        let home = core.poses().pose("home")?.clone();
        let definition = posture_motion();
        core.validate_curiosity_motion(&definition, &home, None)?;
        let run = core.play_generated_anchored_relative(definition, home, now)?;
        self.acts.lifecycle = Some(run);
        self.status.active_clip = Some("posture_home".into());
        if let Some(log) = &self.behaviour_log {
            log.motion("posture_home", run, Some("home".into()), None, now);
        }
        Ok(run)
    }
    pub(crate) fn commit_rest_home<D: RuntimeDriver>(&mut self, core: &RuntimeCore<D>) {
        self.capture_anchor(core);
        self.acts.drifted = false;
        self.finish_lifecycle_act();
    }
    pub(super) fn schedule_drift(&mut self, now: f64) {
        self.acts.drift_at = now + self.rng.range(360., 900.);
    }
    pub(super) fn cancel_acts<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        let run = self
            .acts
            .lifecycle
            .take()
            .or_else(|| self.acts.drift.take().map(|d| d.0));
        if run.is_some_and(|id| {
            core.snapshot()
                .motion
                .as_ref()
                .is_some_and(|m| m.run_id == id)
        }) {
            checked(core.handle_command("stop", now))?;
        }
        Ok(())
    }
    fn drift_ready<D: RuntimeDriver>(&self, core: &RuntimeCore<D>) -> bool {
        self.acts.settings.posture_drift
            && self.acts.allowed
            && self.status.enabled
            && !self.gaze.is_stopped()
            && !self.hands.blocks_gaze()
            && !self.work_anchor
            && self.work.is_none()
            && self.attention.is_none()
            && self.starting_run_id.is_none()
            && !self.foreground_pending
            && self.foreground_scene_run_id.is_none()
            && self.speech_motion_run_id.is_none()
            && self.status.hands_failure.is_none()
            && self.surroundings.reaction.is_none()
            && matches!(
                self.status.state,
                CharacterState::HomeIdle | CharacterState::PoseIdle
            )
            && core.mode() == RuntimeMode::Holding
    }
    pub(super) fn tick_posture<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<bool> {
        if let Some((run, name)) = self.acts.drift.clone() {
            if !self.acts.allowed
                || !self.acts.settings.posture_drift
                || self.hands.blocks_gaze()
                || self.work_anchor
                || self.gaze.is_stopped()
            {
                self.cancel_acts(now, core)?;
                self.schedule_drift(now);
                return Ok(false);
            }
            if let Some(phase) = terminal_phase(core, run) {
                if phase == MovementPhase::Completed {
                    self.capture_anchor(core);
                    self.acts.drifted = name != "home";
                    self.status.state = self.idle_state();
                }
                self.acts.drift = None;
                self.status.active_clip = None;
                self.schedule_drift(now);
                self.reset_timers(now);
            }
            return Ok(true);
        }
        if now < self.acts.drift_at || !self.drift_ready(core) {
            return Ok(false);
        }
        let name = if self.mind.label == "sleepy" {
            "home_low"
        } else if matches!(self.mind.attention.target, AttentionTarget::Person { .. }) {
            "home_attentive"
        } else if self.rng.index(2) == 0 {
            "home"
        } else {
            "home_window"
        };
        let mut target = match core.poses().pose(name) {
            Ok(pose) => pose.clone(),
            Err(_) => {
                self.schedule_drift(now);
                return Ok(false);
            }
        };
        if name == "home_attentive"
            && let Some(bearing) = self.gaze.current_bearing()
        {
            let (_, facing) = self.gaze.attention_motion(Some(bearing), &target, core)?;
            for joint in ["base_yaw_joint", "head_roll_joint", "head_pitch_joint"] {
                *target.get_mut(joint).unwrap() = facing[joint];
            }
        }
        let definition = posture_motion();
        let result = core
            .validate_curiosity_motion(&definition, &target, None)
            .and_then(|()| core.play_generated_anchored_relative(definition, target, now));
        self.schedule_drift(now);
        match result {
            Ok(run) => {
                self.gaze.clear();
                self.acts.drift = Some((run, name.into()));
                self.acts.drifted = true;
                self.status.active_clip = Some("posture_drift".into());
                if let Some(log) = &self.behaviour_log {
                    log.motion("posture_drift", run, Some(name.into()), None, now);
                }
                Ok(true)
            }
            Err(error) => {
                self.status.last_rejection = Some(error.to_string());
                Ok(false)
            }
        }
    }
    pub(super) fn tick_listening<D: RuntimeDriver>(&mut self, now: f64, core: &mut RuntimeCore<D>) {
        if self.acts.listening_attempted
            || !self.acts.settings.listen_lean_in
            || self.hands.blocks_gaze()
            || self.work_anchor
            || core.mode() != RuntimeMode::Holding
        {
            return;
        }
        self.acts.listening_attempted = true;
        if let (Ok(mut motion), Some(anchor)) = (
            core.motions().motion("listen_lean_in").cloned(),
            self.status.active_anchor.clone(),
        ) {
            motion.keyframes[0].hold_seconds = 180.;
            if let Ok(run) = core.play_generated_anchored_relative(motion, anchor, now) {
                self.thinking_run = Some(run);
                self.status.active_clip = Some("listen_lean_in".into());
            }
        }
    }
    pub(super) fn begin_attention_state(&mut self, thinking: bool, now: f64) {
        self.acts.listening_attempted = false;
        if thinking {
            self.acts.thinking_at = now;
            self.acts.next_beat = now + 3.;
        }
    }
    pub(super) fn release_listening<D: RuntimeDriver>(
        &mut self,
        reaction: &str,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> bool {
        if self.status.state != CharacterState::Listening
            || !matches!(reaction, "thinking" | "neutral")
        {
            return false;
        }
        let Some(run) = self.thinking_run else {
            return false;
        };
        let Some(anchor) = self.status.active_anchor.clone() else {
            return false;
        };
        if core
            .extend_character_performance(run, speech_settle_motion(), anchor, now)
            .is_err()
        {
            return false;
        }
        self.begin_attention_state(reaction == "thinking", now);
        if let Some(attention) = self.attention.as_mut() {
            attention.expires_at = now + if reaction == "neutral" { 15. } else { 120. };
        }
        if reaction == "thinking" {
            self.status.state = CharacterState::Thinking;
        } else {
            self.thinking_run = None;
            self.speech_motion_run_id = Some(run);
            self.status.state = CharacterState::Settling;
        }
        self.status.active_clip = Some("listen_release".into());
        true
    }
    pub(super) fn tick_thinking_beats<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> bool {
        if !self.acts.settings.thinking_beats || core.motions().motion("think_beat_a").is_err() {
            return false;
        }
        if now < self.acts.thinking_at + 3. {
            return false;
        }
        if now < self.acts.next_beat {
            return true;
        }
        let choice = self
            .acts
            .last_beat
            .map(|last| !last)
            .unwrap_or_else(|| self.rng.index(2) == 0);
        let clip = if choice {
            "think_beat_a"
        } else {
            "think_beat_b"
        };
        self.acts.next_beat = now + self.rng.range(3., 5.);
        if self.hands.blocks_gaze() || self.work_anchor {
            return true;
        }
        if let (Ok(definition), Some(anchor)) = (
            core.motions().motion(clip).cloned(),
            self.status.active_anchor.clone(),
        ) {
            let run = if let Some(run) = self.thinking_run.filter(|run| {
                core.snapshot()
                    .motion
                    .as_ref()
                    .is_some_and(|m| m.run_id == *run && m.state == MovementPhase::Executing)
            }) {
                core.extend_character_performance(run, definition, anchor, now)
            } else if core.mode() == RuntimeMode::Holding {
                core.play_generated_anchored_relative(definition, anchor, now)
            } else {
                return true;
            };
            if let Ok(run) = run {
                self.thinking_run = Some(run);
                self.acts.last_beat = Some(choice);
                self.status.active_clip = Some(clip.into());
            }
        }
        true
    }
    pub(super) fn idle_factors(&self) -> (f64, f64, f64) {
        if !self.acts.settings.mind_idle_rhythm || self.hands.blocks_gaze() || self.work_anchor {
            return (1., 1., 1.);
        }
        let energy = self.mind.energy.clamp(0., 1.);
        (1.6 - energy, 0.85 + 0.3 * energy, 0.9 + 0.2 * energy)
    }
    pub(crate) fn mind_light_intensity(&self) -> f64 {
        if !self.acts.settings.mind_idle_rhythm || self.hands.blocks_gaze() || self.work_anchor {
            1.
        } else {
            0.6 + 0.4 * self.mind.energy.clamp(0., 1.)
        }
    }
    pub(crate) fn background_frame<D: RuntimeDriver>(
        &self,
        effect: &str,
        now: f64,
        pixels: usize,
        intensity: f64,
        core: &RuntimeCore<D>,
    ) -> Result<Vec<crate::Rgbw8>> {
        let idle = matches!(
            self.status.state,
            CharacterState::HomeIdle | CharacterState::PoseIdle
        );
        let gain = if idle {
            self.mind_light_intensity()
        } else {
            1.
        };
        let exhale = self.sleep_light_gain(core);
        let mut frame = crate::devices::lighting::render_effect_for_pixels(
            effect,
            now,
            (intensity * gain * exhale).clamp(0., 1.),
            pixels,
        )?;
        if idle && self.mind.energy < 0.4 && self.idle_factors() != (1., 1., 1.) {
            for color in &mut frame {
                color.red = color.red.saturating_add((color.white as f64 * 0.15) as u8);
                color.blue = (color.blue as f64 * 0.55) as u8;
                color.white = (color.white as f64 * 0.8) as u8;
            }
        }
        Ok(frame)
    }
    pub(crate) fn sleep_light_gain<D: RuntimeDriver>(&self, core: &RuntimeCore<D>) -> f64 {
        if self.status.active_clip.as_deref() == Some("sleep_settle") {
            core.snapshot()
                .motion
                .as_ref()
                .map_or(1., |motion| 1. - 0.65 * motion.progress.clamp(0., 1.))
        } else {
            1.
        }
    }
    pub(super) fn sleepy_idle<D: RuntimeDriver>(&self, now: f64, core: &RuntimeCore<D>) -> bool {
        self.acts
            .rest_due
            .is_some_and(|at| at > now && at - now <= 120.)
            && self.mind.energy < 0.3
            && self.last_idle.as_deref() != Some("yawn")
            && !self.work_anchor
            && !self.hands.blocks_gaze()
            && core.motions().motion("yawn").is_ok()
    }
}
fn posture_motion() -> MotionDefinition {
    MotionDefinition {
        name: "posture_drift".into(),
        description: "Move slowly to the next held posture.".into(),
        space: MotionSpace::AnchorRelative,
        style: MotionStyle::named("return_home").unwrap(),
        return_to_anchor: true,
        keyframes: vec![MotionKeyframe {
            pose_name: None,
            target: JointPositions::new(),
            duration_seconds: 3.5,
            arrival: KeyframeArrival::Settle,
            hold_seconds: 0.,
            marker: Some("posture_arrived".into()),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expression::{
        gaze::tests::{FollowingDriver, following},
        rest::{RestCoordinator, RestState},
        voice_feedback::VoiceFeedback,
    };
    fn fixture() -> (RuntimeCore<FollowingDriver>, CharacterCoordinator) {
        let (mut core, anchor, _) = following();
        core.configure_comfort_box(
            crate::HardwareVersion::V2
                .profile()
                .comfort_box
                .clone()
                .unwrap(),
        )
        .unwrap();
        let mut character = CharacterCoordinator::new(7);
        character.status.enabled = true;
        character.status.state = CharacterState::HomeIdle;
        character.status.active_anchor = Some(anchor);
        character.set_gaze_enabled(true);
        character.reset_timers(0.);
        (core, character)
    }
    fn advance(
        core: &mut RuntimeCore<FollowingDriver>,
        character: &mut CharacterCoordinator,
        from: f64,
        to: f64,
    ) {
        for step in 1..=((to - from) * 50.).round() as usize {
            let now = from + step as f64 * 0.02;
            core.tick(now).unwrap();
            character
                .tick(now, core, false, None, false, None, None)
                .unwrap();
        }
    }
    #[test]
    fn rhythm_scales_both_intervals_head_amplitude_and_tempo() {
        let (core, mut low) = fixture();
        let (_, mut high) = fixture();
        low.mind.energy = 0.;
        high.mind.energy = 1.;
        low.reset_timers(10.);
        high.reset_timers(10.);
        for (a, b) in [
            (low.next_micro_at, high.next_micro_at),
            (low.next_large_at, high.next_large_at),
        ] {
            assert!(((a - 10.) / (b - 10.) - 1.6 / 0.6).abs() < 1e-9);
        }
        let anchor = core.poses().pose("home").unwrap();
        let limits = core.driver().joint_limits().unwrap();
        let source = core.motions().motion("idle_breathe").unwrap();
        let a = low
            .vary_idle(source.clone(), anchor, &limits, false)
            .unwrap();
        let b = high
            .vary_idle(source.clone(), anchor, &limits, false)
            .unwrap();
        let joint = "head_pitch_joint";
        assert!(
            (b.keyframes[0].target[joint] / source.keyframes[0].target[joint]
                - a.keyframes[0].target[joint] / source.keyframes[0].target[joint]
                - 0.3)
                .abs()
                < 1e-9
        );
        assert!(
            (source.keyframes[0].duration_seconds / b.keyframes[0].duration_seconds
                - source.keyframes[0].duration_seconds / a.keyframes[0].duration_seconds
                - 0.2)
                .abs()
                < 1e-9
        );
        let dim = low
            .background_frame("warm_idle_breathe", 1., 24, 1., &core)
            .unwrap();
        let bright = high
            .background_frame("warm_idle_breathe", 1., 24, 1., &core)
            .unwrap();
        assert!(dim[0].white < bright[0].white);
        assert!(
            dim[0].red as f64 / (dim[0].white as f64 + 1.)
                > bright[0].red as f64 / (bright[0].white as f64 + 1.)
        );
        low.work_anchor = true;
        assert_eq!(low.idle_factors(), (1., 1., 1.));
        assert_eq!(low.mind_light_intensity(), 1.);
        assert_eq!(
            low.background_frame("warm_idle_breathe", 1., 24, 1., &core)
                .unwrap(),
            bright
        );
    }
    #[test]
    fn drift_requires_idle_admission_and_commits_only_measured_arrival() {
        let (mut core, mut character) = fixture();
        character.acts.settings.posture_drift = true;
        character.acts.allowed = true;
        assert!(character.drift_ready(&core));
        character.acts.allowed = false;
        assert!(!character.drift_ready(&core));
        character.acts.allowed = true;
        character.work_anchor = true;
        assert!(!character.drift_ready(&core));
        character.work_anchor = false;
        character.status.state = CharacterState::Listening;
        assert!(!character.drift_ready(&core));
        character.status.state = CharacterState::HomeIdle;
        character.forget_gaze_for_external_stop(crate::expression::gaze::GazeStopCommand::Stop);
        assert!(!character.drift_ready(&core));
        character.resume_gaze(&core).unwrap();
        character.acts.drift_at = 0.;
        character.mind.label = "sleepy".into();
        let old = character.status.active_anchor.clone();
        assert!(character.tick_posture(0., &mut core).unwrap());
        assert_eq!(character.status.active_anchor, old);
        advance(&mut core, &mut character, 0., 6.);
        assert!(character.acts.drift.is_none());
        assert_ne!(character.status.active_anchor, old);
        assert!(character.drifted());
        let low = core.poses().pose("home_low").unwrap();
        for (joint, value) in character.status.active_anchor.as_ref().unwrap() {
            assert!((value - low[joint]).abs() < 1e-9);
        }
        character.acts.rest_due = Some(100.);
        character.mind.energy = 0.2;
        assert!(character.sleepy_idle(5., &core));
        assert!(!character.sleepy_idle(101., &core));
    }
    #[test]
    fn hands_and_rest_pending_block_drift_and_energy_variation() {
        let (mut core, mut character) = fixture();
        character.acts.settings.posture_drift = true;
        character.acts.allowed = true;
        let observation =
            serde_json::from_value(serde_json::json!({"capture_boottime_ns":1_000_000_000u64,
            "camera_on":true,"tracking_mode":"hands","settings":{},"objects":[],"faces":[],
            "looking_away":null,"hand_count":0,"hand_point":null}))
            .unwrap();
        character
            .observe_surroundings(observation, 0., &mut core)
            .unwrap();
        assert!(character.hands.blocks_gaze());
        assert!(!character.drift_ready(&core));
        assert_eq!(character.idle_factors(), (1., 1., 1.));
        character.acts.allowed = false;
        assert!(!character.drift_ready(&core));
    }
    fn speech() -> SpeechAnalysis {
        SpeechAnalysis {
            rms_20ms: vec![0.2; 200],
            quiet_regions: vec![],
            phrase_peaks: vec![40],
            cues: vec![],
            duration_seconds: 4.,
            streaming: true,
        }
    }
    #[test]
    fn listening_holds_releases_and_speech_takes_the_same_run() {
        for processing in [false, true] {
            let (mut core, mut character) = fixture();
            character.set_reaction("listening", 0., &mut core).unwrap();
            character
                .tick(0., &mut core, false, None, false, None, None)
                .unwrap();
            let run = character.thinking_run.unwrap();
            advance(&mut core, &mut character, 0., 3.);
            assert_eq!(
                character.status.active_clip.as_deref(),
                Some("listen_lean_in")
            );
            if processing {
                character.set_reaction("thinking", 3., &mut core).unwrap();
                assert_eq!(character.thinking_run, Some(run));
                assert_eq!(
                    character.status.active_clip.as_deref(),
                    Some("listen_release")
                );
            }
            character.note_speech_started(3.02);
            character
                .tick(3.02, &mut core, false, None, true, Some(&speech()), Some(0))
                .unwrap();
            assert_eq!(character.speech_motion_run_id, Some(run));
            assert!(character.thinking_run.is_none());
            assert_eq!(character.status.state, CharacterState::Speaking);
        }
    }
    #[test]
    fn thinking_beats_wait_alternate_and_preserve_continuation() {
        let (mut core, mut character) = fixture();
        character.set_reaction("thinking", 0., &mut core).unwrap();
        character
            .tick(0., &mut core, false, None, false, None, None)
            .unwrap();
        let run = character.thinking_run.unwrap();
        advance(&mut core, &mut character, 0., 2.98);
        assert!(
            !character
                .status
                .active_clip
                .as_deref()
                .is_some_and(|s| s.starts_with("think_beat"))
        );
        advance(&mut core, &mut character, 2.98, 3.02);
        assert_eq!(character.thinking_run, Some(run));
        let first = character.acts.last_beat.unwrap();
        let due = character.acts.next_beat;
        assert!((3. ..=5.).contains(&(due - 3.)));
        advance(&mut core, &mut character, 3.02, 8.02);
        assert_eq!(character.acts.last_beat, Some(!first));
        character.note_speech_started(8.04);
        character
            .tick(8.04, &mut core, false, None, true, Some(&speech()), Some(0))
            .unwrap();
        assert_eq!(character.status.state, CharacterState::Speaking);
        assert!(character.thinking_run.is_none());
    }
    #[test]
    fn sleep_and_wake_acts_continue_after_success_cancellation_or_timeout() {
        for failure in 0..3 {
            let (mut core, mut character) = fixture();
            let mut rest = RestCoordinator::new(1800.);
            let mut feedback = VoiceFeedback::default();
            rest.started();
            rest.tick(0., &mut core, &mut character, &feedback, false);
            rest.begin_rest(0., &mut core, &mut character).unwrap();
            assert_eq!(rest.status(0.).state, RestState::PreparingRest);
            assert!(core.snapshot().torque_enabled);
            if failure == 1 {
                checked(core.handle_command("stop", 0.1)).unwrap();
            }
            if failure == 2 {
                core.driver().offset_feedback(0.4, 0.);
            }
            for step in 1..=700 {
                let now = step as f64 * 0.02;
                core.tick(now).unwrap();
                character
                    .tick(now, &mut core, false, None, false, None, None)
                    .unwrap();
                rest.tick(now, &mut core, &mut character, &feedback, false);
                if rest.status(now).state == RestState::GoingToRest {
                    core.driver().offset_feedback(0., 0.);
                }
                if !core.snapshot().torque_enabled {
                    assert_eq!(rest.status(now).state, RestState::Resting);
                    assert_eq!(
                        core.snapshot().last_motion.as_ref().unwrap().state,
                        MovementPhase::Completed
                    );
                }
            }
            assert_eq!(rest.status(14.).state, RestState::Resting);
            let id = "a".repeat(32);
            feedback.event(&id, "wake", 14.).unwrap();
            feedback.event(&id, "endpoint", 14.).unwrap();
            assert!(feedback.confirm(&id, 14.));
            rest.confirmed(&id, 14.);
            let mut stretched = false;
            for step in 701..=1500 {
                let now = step as f64 * 0.02;
                core.tick(now).unwrap();
                character
                    .tick(now, &mut core, false, None, false, None, None)
                    .unwrap();
                rest.tick(now, &mut core, &mut character, &feedback, false);
                if character.status.active_clip.as_deref() == Some("wake_stretch") {
                    stretched = true;
                    assert_eq!(rest.status(now).state, RestState::Waking);
                    if failure == 1 {
                        checked(core.handle_command("stop", now)).unwrap();
                    }
                    if failure == 2 {
                        core.driver().offset_feedback(0.4, 0.);
                    }
                } else {
                    core.driver().offset_feedback(0., 0.);
                }
            }
            assert!(stretched);
            assert_eq!(rest.status(30.).state, RestState::Awake);
            assert!(core.snapshot().torque_enabled);
        }
    }
    #[test]
    fn drift_returns_home_before_sleep_and_journals_both_outcomes() {
        let (mut core, mut character) = fixture();
        let dir = tempfile::tempdir().unwrap();
        let writer = orion_history::BehaviourWriter::start(dir.path().into()).unwrap();
        character.behaviour_log = Some(crate::expression::behaviour::BehaviourLog::new(&writer));
        character.acts.allowed = true;
        character.acts.settings.posture_drift = true;
        character.acts.drift_at = 0.;
        character.mind.label = "sleepy".into();
        character.tick_posture(0., &mut core).unwrap();
        let log = character.behaviour_log.clone().unwrap();
        advance(&mut core, &mut character, 0., 6.);
        log.observe_motion(core.snapshot(), 6.);
        let mut rest = RestCoordinator::new(1.);
        let feedback = VoiceFeedback::default();
        rest.started();
        rest.tick(6., &mut core, &mut character, &feedback, false);
        rest.tick(7., &mut core, &mut character, &feedback, false);
        assert_eq!(
            character.status.active_clip.as_deref(),
            Some("posture_home")
        );
        for step in 351..=1500 {
            let now = step as f64 * 0.02;
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
            rest.tick(now, &mut core, &mut character, &feedback, false);
            log.observe_motion(core.snapshot(), now);
            if character.status.active_clip.as_deref() == Some("sleep_settle") {
                for (joint, value) in character.status.active_anchor.as_ref().unwrap() {
                    assert!((value - core.poses().pose("home").unwrap()[joint]).abs() < 1e-9);
                }
            }
        }
        assert_eq!(rest.status(30.).state, RestState::Resting);
        drop(writer);
        let history = orion_history::Store::new(dir.path().into())
            .behaviours(None, None)
            .unwrap();
        let events = history["items"].as_array().unwrap();
        for name in ["posture_drift", "posture_home"] {
            for phase in ["started", "completed"] {
                assert!(
                    events
                        .iter()
                        .any(|e| e["behaviour"] == name && e["phase"] == phase)
                );
            }
        }
    }

    #[test]
    fn interrupted_drift_keeps_the_anchor_and_records_cancellation() {
        let (mut core, mut character) = fixture();
        let directory = tempfile::tempdir().unwrap();
        let writer = orion_history::BehaviourWriter::start(directory.path().into()).unwrap();
        let log = crate::expression::behaviour::BehaviourLog::new(&writer);
        character.behaviour_log = Some(log.clone());
        character.acts.settings.posture_drift = true;
        character.acts.allowed = true;
        character.acts.drift_at = 0.;
        character.mind.label = "sleepy".into();
        let anchor = character.status.active_anchor.clone();
        character.tick_posture(0., &mut core).unwrap();
        core.tick(0.5).unwrap();
        character.preempt_idle_or_thinking(0.5, &mut core).unwrap();
        log.observe_motion(core.snapshot(), 0.5);
        assert_eq!(character.status.active_anchor, anchor);
        drop(writer);
        let history = orion_history::Store::new(directory.path().into())
            .behaviours(None, None)
            .unwrap();
        assert!(
            history["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["behaviour"] == "posture_drift"
                    && event["phase"] == "interrupted")
        );
    }
}
