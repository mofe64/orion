use serde::Serialize;

use crate::daemon::RuntimeCore;
use crate::driver::RuntimeDriver;
use crate::motion::{
    KeyframeArrival, MotionDefinition, MotionKeyframe, MotionLibrary, MotionSpace,
};
use crate::pose::JointPositions;
use crate::speech::SpeechAnalysis;
use crate::state::{MovementPhase, RuntimeMode};
use crate::style::MotionStyle;
use crate::{Error, Result};

const MICRO_IDLE_MIN_SECONDS: f64 = 8.0;
const MICRO_IDLE_MAX_SECONDS: f64 = 20.0;
const LARGE_IDLE_MIN_SECONDS: f64 = 35.0;
const LARGE_IDLE_MAX_SECONDS: f64 = 75.0;
const SPEECH_END_LEAD_SECONDS: f64 = 0.12;
const SPEECH_FINAL_SETTLE_SECONDS: f64 = 0.55;
const SPEECH_GESTURE_DURATION_SCALE: f64 = 1.35;
const SPEECH_PEAK_LOOKAHEAD_FRAMES: usize = 75;
const SPEECH_BODY_BEAT_INTERVAL_DRAWINGS: usize = 3;

const MICRO_IDLES: [&str; 4] = [
    "idle_breathe",
    "idle_head_curiosity",
    "idle_micro_glance",
    "idle_shoulder_adjust",
];
const LARGE_IDLES: [&str; 3] = ["idle_weight_shift", "idle_soft_head_shake", "idle_breathe"];

#[derive(Clone, Debug)]
struct PlannedSpeechDrawing {
    clip: String,
    head_target: JointPositions,
    body_target: JointPositions,
    duration_seconds: f64,
    body_beat: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterState {
    Off,
    Starting,
    HomeIdle,
    PoseIdle,
    Listening,
    Thinking,
    Speaking,
    ForegroundScene,
    Settling,
    ShuttingDown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NextIdleCategory {
    Micro,
    Large,
}

#[derive(Clone, Debug, Serialize)]
pub struct CharacterStatus {
    pub enabled: bool,
    pub state: CharacterState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_anchor: Option<JointPositions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_clip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_idle_category: Option<NextIdleCategory>,
}

#[derive(Debug)]
struct Attention {
    previous: JointPositions,
    run_id: Option<u64>,
    returning: bool,
    expires_at: f64,
}

#[derive(Debug)]
pub struct CharacterCoordinator {
    status: CharacterStatus,
    rng: SeededRandom,
    next_micro_at: f64,
    next_large_at: f64,
    last_idle: Option<String>,
    active_idle_run_id: Option<u64>,
    active_idle_category: Option<NextIdleCategory>,
    starting_run_id: Option<u64>,
    foreground_pending: bool,
    foreground_scene_run_id: Option<u64>,
    thinking_run: Option<u64>,
    speech_motion_run_id: Option<u64>,
    speech_motion_started: bool,
    last_speech_clip: Option<String>,
    speech_planned_until: f64,
    speech_gesture_index: usize,
    speech_last_body_beat: Option<usize>,
    speech_last_tilt: f64,
    speech_last_turn: i8,
    speech_previous_body: JointPositions,
    attention: Option<Attention>,
}

impl CharacterCoordinator {
    pub fn new(seed: u64) -> Self {
        Self {
            status: CharacterStatus {
                enabled: false,
                state: CharacterState::Off,
                active_anchor: None,
                active_clip: None,
                next_idle_category: None,
            },
            rng: SeededRandom::new(seed),
            next_micro_at: f64::INFINITY,
            next_large_at: f64::INFINITY,
            last_idle: None,
            active_idle_run_id: None,
            active_idle_category: None,
            starting_run_id: None,
            foreground_pending: false,
            foreground_scene_run_id: None,
            thinking_run: None,
            speech_motion_run_id: None,
            speech_motion_started: false,
            last_speech_clip: None,
            speech_planned_until: 0.0,
            speech_gesture_index: 0,
            speech_last_body_beat: None,
            speech_last_tilt: 0.0,
            speech_last_turn: 0,
            speech_previous_body: JointPositions::new(),
            attention: None,
        }
    }

    pub fn clear_attention(&mut self) {
        self.attention = None;
    }

    /// Confirmed, coarse speaker attention through the ordinary motion executor.
    pub fn attend<D: RuntimeDriver>(
        &mut self,
        side: &str,
        confidence: f64,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<CharacterStatus> {
        if !matches!(side, "left" | "right")
            || !confidence.is_finite()
            || !(0.75..=1.0).contains(&confidence)
        {
            return Err(Error::InvalidArgument(
                "Attention requires left/right and confidence in [0.75, 1].".into(),
            ));
        }
        if !self.status.enabled
            || self.starting_run_id.is_some()
            || self.foreground_pending
            || self.foreground_scene_run_id.is_some()
            || self.speech_motion_run_id.is_some()
            || !matches!(
                self.status.state,
                CharacterState::HomeIdle
                    | CharacterState::PoseIdle
                    | CharacterState::Listening
                    | CharacterState::Thinking
            )
            || (core.mode() == RuntimeMode::Moving
                && self.active_idle_run_id.is_none()
                && self.thinking_run.is_none())
        {
            return Err(Error::InvalidState(
                "Attention requires an available powered character.".into(),
            ));
        }
        if self.attention.is_some() {
            return Ok(self.status.clone()); // One facing decision per conversation.
        }
        let previous = self
            .status
            .active_anchor
            .clone()
            .ok_or_else(|| Error::InvalidState("Attention needs an anchor.".into()))?;
        let target_yaw = if side == "left" { -0.35 } else { 0.35 };
        if (core
            .snapshot()
            .joints
            .iter()
            .find(|joint| joint.name == "base_yaw_joint")
            .ok_or_else(|| Error::Runtime("Missing base feedback".into()))?
            .position
            - target_yaw)
            .abs()
            > 0.65
        {
            return Err(Error::InvalidState(
                "Attention would require a broad turn from this pose.".into(),
            ));
        }
        self.preempt_idle(now, core)?;
        let response = checked(core.handle_command(&format!("play attention_{side}"), now))?;
        let run_id = response
            .get("run_id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| Error::Runtime("Attention has no run ID.".into()))?;
        self.attention = Some(Attention {
            previous,
            run_id: Some(run_id),
            returning: false,
            expires_at: now + 120.0,
        });
        self.status.state = CharacterState::Listening;
        self.status.active_clip = Some(format!("attention_{side}"));
        self.reset_timers(now);
        Ok(self.status.clone())
    }

    pub fn status(&self) -> &CharacterStatus {
        &self.status
    }

    /// Select the lowest-priority character light for the current held state.
    pub fn background_lighting_effect<D: RuntimeDriver>(
        &self,
        core: &RuntimeCore<D>,
    ) -> Option<String> {
        if !self.status.enabled {
            return None;
        }
        match self.status.state {
            CharacterState::Listening => return Some("attentive_focus".into()),
            CharacterState::Thinking => return Some("thinking_drift".into()),
            CharacterState::Starting | CharacterState::Settling => {
                return Some("settle_glow".into());
            }
            CharacterState::Off
            | CharacterState::Speaking
            | CharacterState::ForegroundScene
            | CharacterState::ShuttingDown => return None,
            CharacterState::HomeIdle | CharacterState::PoseIdle => {}
        }
        self.status
            .active_anchor
            .as_ref()
            .and_then(|anchor| closest_pose_default_lighting(core, anchor))
            .or_else(|| Some("warm_idle_breathe".into()))
    }

    pub fn start<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<CharacterStatus> {
        if self.status.enabled {
            return Err(Error::InvalidState(
                "Character mode is already enabled.".into(),
            ));
        }
        if core.mode() == RuntimeMode::Observe {
            checked(core.handle_command("configure", now))?;
        }
        if core.mode() == RuntimeMode::Configured {
            checked(core.handle_command("enable", now))?;
        }
        if core.mode() != RuntimeMode::Holding {
            return Err(Error::InvalidState(
                "Character mode requires configured holding torque.".into(),
            ));
        }
        let response = checked(core.handle_command("goto home 1.600000", now))?;
        self.starting_run_id = response.get("run_id").and_then(serde_json::Value::as_u64);
        self.status.enabled = true;
        self.status.state = CharacterState::Starting;
        self.status.active_clip = Some("return_home".into());
        self.reset_timers(now);
        Ok(self.status.clone())
    }

    pub fn stop<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<CharacterStatus> {
        if !self.status.enabled {
            return Err(Error::InvalidState("Character mode is not enabled.".into()));
        }
        self.clear_attention();
        self.preempt_idle(now, core)?;
        if core.mode() == RuntimeMode::Moving {
            checked(core.handle_command("stop", now))?;
        }
        if core.mode() == RuntimeMode::Holding {
            let response = checked(core.handle_command("play return_home", now))?;
            self.starting_run_id = response.get("run_id").and_then(serde_json::Value::as_u64);
            self.status.state = CharacterState::ShuttingDown;
            self.status.active_clip = Some("return_home".into());
        } else {
            self.finish_stop();
        }
        Ok(self.status.clone())
    }

    /// Leave autonomous character behavior and use the existing rest trajectory.
    pub fn rest<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<serde_json::Value> {
        self.clear_attention();
        self.preempt_idle(now, core)?;
        if core.mode() == RuntimeMode::Moving {
            checked(core.handle_command("stop", now))?;
        }
        self.finish_stop();
        if core.mode() == RuntimeMode::Observe {
            checked(core.handle_command("configure", now))?;
        }
        if core.mode() == RuntimeMode::Configured {
            checked(core.handle_command("enable", now))?;
        }
        checked(core.handle_command("goto rest 3.0", now))
    }

    pub fn set_reaction<D: RuntimeDriver>(
        &mut self,
        reaction: &str,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<CharacterStatus> {
        if !self.status.enabled {
            return Err(Error::InvalidState(
                "Enable character mode before setting character state.".into(),
            ));
        }
        if self.starting_run_id.is_some()
            || self.foreground_pending
            || self.foreground_scene_run_id.is_some()
        {
            return Err(Error::InvalidState(
                "Character transition has priority over a reaction.".into(),
            ));
        }
        self.preempt_idle(now, core)?;
        self.reset_timers(now);
        if let Some(attention) = self.attention.as_mut() {
            attention.expires_at = now + if reaction == "neutral" { 15.0 } else { 120.0 };
        }
        // Playback acknowledgement can arrive before the physical settle ends.
        // Keep speech ownership until tick has retired its movement; otherwise
        // a later idle can overwrite the only terminal record of that run.
        if matches!(
            self.status.state,
            CharacterState::Speaking | CharacterState::Settling
        ) && matches!(reaction, "neutral" | "listening" | "thinking")
        {
            return Ok(self.status.clone());
        }
        match reaction {
            "neutral" => self.status.state = self.idle_state(),
            "listening" => self.status.state = CharacterState::Listening,
            "thinking" => self.status.state = CharacterState::Thinking,
            _ => {
                return Err(Error::InvalidArgument(
                    "Character state must be neutral, listening, or thinking.".into(),
                ));
            }
        }
        Ok(self.status.clone())
    }

    pub fn preempt_idle<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        if (self.active_idle_run_id.is_some() || self.thinking_run.is_some())
            && core.mode() == RuntimeMode::Moving
        {
            checked(core.handle_command("stop", now))?;
        }
        self.thinking_run = None;
        if self.active_idle_run_id.take().is_some() {
            self.status.active_clip = None;
        }
        self.active_idle_category = None;
        Ok(())
    }

    pub fn note_foreground_started(&mut self, now: f64) {
        self.clear_attention();
        self.starting_run_id = None;
        if !self.status.enabled {
            return;
        }
        self.foreground_pending = true;
        self.status.state = CharacterState::ForegroundScene;
        self.status.active_clip = None;
        self.reset_timers(now);
    }

    pub fn note_speech_started(&mut self, now: f64) {
        if !self.status.enabled || self.starting_run_id.is_some() {
            return;
        }
        self.status.state = CharacterState::Speaking;
        self.status.active_clip = None;
        self.speech_motion_started = false;
        self.reset_timers(now);
        self.speech_planned_until = 0.0;
        self.speech_gesture_index = 0;
        self.speech_last_body_beat = None;
        self.speech_last_tilt = 0.0;
        self.speech_last_turn = 0;
        self.speech_previous_body.clear();
    }

    pub fn note_foreground_scene_started(&mut self, now: f64, run_id: u64) {
        self.clear_attention();
        self.starting_run_id = None;
        if !self.status.enabled {
            return;
        }
        self.foreground_scene_run_id = Some(run_id);
        self.status.state = CharacterState::ForegroundScene;
        self.status.active_clip = None;
        self.reset_timers(now);
    }

    pub fn tick<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
        scene_active: bool,
        last_scene_result: Option<(u64, bool)>,
        speech_active: bool,
        speech_analysis: Option<&SpeechAnalysis>,
        speech_frame: Option<usize>,
    ) -> Result<()> {
        if !self.status.enabled {
            return Ok(());
        }
        if let Some(run_id) = self.starting_run_id {
            if let Some(phase) = terminal_phase(core, run_id) {
                self.starting_run_id = None;
                if self.status.state == CharacterState::ShuttingDown {
                    self.finish_stop();
                    return Ok(());
                }
                if phase != MovementPhase::Completed {
                    self.finish_stop();
                    return Ok(());
                }
                self.capture_anchor(core);
                self.status.state = CharacterState::HomeIdle;
                self.status.active_clip = None;
                self.reset_timers(now);
            }
            return Ok(());
        }

        if scene_active {
            self.status.state = CharacterState::ForegroundScene;
            return Ok(());
        }
        if let Some(run_id) = self.foreground_scene_run_id {
            let Some((terminal_run_id, completed)) = last_scene_result else {
                return Ok(());
            };
            if terminal_run_id != run_id {
                return Ok(());
            }
            if completed {
                self.capture_anchor(core);
            }
            self.foreground_scene_run_id = None;
            self.status.state = self.idle_state();
            self.status.active_clip = None;
            self.reset_timers(now);
        }
        if self
            .status
            .active_anchor
            .as_ref()
            .is_some_and(|anchor| closest_pose_is_shutdown_only(core, anchor))
        {
            self.finish_stop();
            return Ok(());
        }
        if let Some(mut attention) = self.attention.take() {
            if let Some(run_id) = attention.run_id {
                if let Some(phase) = terminal_phase(core, run_id) {
                    attention.run_id = None;
                    self.status.active_clip = None;
                    if phase == MovementPhase::Completed {
                        if attention.returning {
                            self.status.active_anchor = Some(attention.previous.clone());
                            self.status.state = self.idle_state();
                            self.reset_timers(now);
                        } else {
                            self.capture_anchor(core);
                            self.attention = Some(attention);
                        }
                    }
                } else if speech_active {
                    checked(core.handle_command("stop", now))?;
                    self.status.active_clip = None;
                } else {
                    self.attention = Some(attention);
                    return Ok(());
                }
            } else if !speech_active
                && now >= attention.expires_at
                && core.mode() == RuntimeMode::Holding
            {
                attention.run_id = Some(core.play_generated_anchored_relative(
                    attention_return_motion(),
                    attention.previous.clone(),
                    now,
                )?);
                attention.returning = true;
                self.status.active_clip = Some("attention_return".into());
                self.status.state = CharacterState::Settling;
                self.attention = Some(attention);
                return Ok(());
            } else {
                self.attention = Some(attention);
            }
        }
        if speech_active {
            if self.status.state != CharacterState::Speaking {
                if self.status.active_anchor.is_none() {
                    self.capture_anchor(core);
                }
                self.status.state = CharacterState::Speaking;
                self.speech_motion_started = false;
            }
            self.tick_speaking(now, core, speech_analysis, speech_frame);
            return Ok(());
        }

        if self.status.state == CharacterState::Speaking {
            if let Some(run_id) = self.speech_motion_run_id {
                if terminal_phase(core, run_id).is_none() {
                    if core.mode() == RuntimeMode::Moving {
                        let _ = checked(core.handle_command("stop", now));
                    }
                    self.speech_motion_run_id = None;
                    if core.mode() == RuntimeMode::Holding
                        && let Some(anchor) = self.status.active_anchor.clone()
                        && let Ok(settle_run_id) = core.play_generated_anchored_relative(
                            speech_settle_motion(),
                            anchor,
                            now,
                        )
                    {
                        self.speech_motion_run_id = Some(settle_run_id);
                        self.status.active_clip = Some("speak_settle".into());
                        self.status.state = CharacterState::Settling;
                        return Ok(());
                    }
                }
                self.speech_motion_run_id = None;
            }
            self.status.active_clip = None;
            self.status.state = self.idle_state();
            self.reset_timers(now);
        }
        if self.status.state == CharacterState::Settling {
            if let Some(run_id) = self.speech_motion_run_id {
                if terminal_phase(core, run_id).is_none() {
                    return Ok(());
                }
                self.speech_motion_run_id = None;
            }
            self.status.active_clip = None;
            self.status.state = self.idle_state();
            self.reset_timers(now);
        }

        if self.status.state == CharacterState::Thinking {
            if self
                .thinking_run
                .is_some_and(|run| terminal_phase(core, run).is_none())
            {
                return Ok(());
            }
            self.thinking_run = None;
            if core.mode() == RuntimeMode::Holding {
                if let Some(anchor) = self.status.active_anchor.clone() {
                    self.thinking_run = core
                        .play_generated_anchored_relative(thinking_motion(), anchor, now)
                        .ok();
                }
            }
            return Ok(());
        }
        if let Some(run_id) = self.active_idle_run_id {
            if let Some(phase) = terminal_phase(core, run_id) {
                self.active_idle_run_id = None;
                let category = self.active_idle_category.take().ok_or_else(|| {
                    Error::Runtime("Autonomous idle lost its scheduling category.".into())
                })?;
                self.status.active_clip = None;
                self.status.state = self.idle_state();
                self.reschedule_idle(category, now);

                // A completion timeout is a terminal result for this animation,
                // not a daemon-fatal condition. RuntimeCore continues holding the
                // clip's final target (the immutable anchor), and exposes the
                // timeout through last_motion for diagnostics. Keeping character
                // mode alive avoids dropping torque and losing the anchor because
                // one low-priority idle took too long to satisfy the telemetry
                // settle gate.
                debug_assert!(phase.is_terminal());
            }
            return Ok(());
        }

        if self.foreground_pending && core.snapshot().motion.is_none() {
            self.capture_anchor(core);
            self.foreground_pending = false;
            self.status.state = CharacterState::Settling;
            self.status.state = self.idle_state();
            self.reset_timers(now);
        }

        if !matches!(
            self.status.state,
            CharacterState::HomeIdle | CharacterState::PoseIdle
        ) || core.mode() != RuntimeMode::Holding
        {
            return Ok(());
        }
        // A conversation holds a deliberate eyeline; avoid an idle delaying its return.
        if self.attention.is_some() {
            return Ok(());
        }
        let category = if self.next_micro_at <= self.next_large_at {
            NextIdleCategory::Micro
        } else {
            NextIdleCategory::Large
        };
        self.status.next_idle_category = Some(category);
        let due = match category {
            NextIdleCategory::Micro => self.next_micro_at,
            NextIdleCategory::Large => self.next_large_at,
        };
        if now < due {
            return Ok(());
        }
        let clip = self.choose_idle(category, core);
        let anchor =
            self.status.active_anchor.clone().ok_or_else(|| {
                Error::InvalidState("Character idle has no immutable anchor.".into())
            })?;
        let run_id = core.play_anchored_relative(&clip, anchor, now)?;
        self.active_idle_run_id = Some(run_id);
        self.active_idle_category = Some(category);
        self.status.active_clip = Some(clip.clone());
        self.last_idle = Some(clip);
        Ok(())
    }

    fn choose_idle<D: RuntimeDriver>(
        &mut self,
        category: NextIdleCategory,
        core: &RuntimeCore<D>,
    ) -> String {
        let profile = self
            .status
            .active_anchor
            .as_ref()
            .and_then(|anchor| closest_pose_profile(core, anchor));
        let mut candidates: Vec<&str> = match (profile.as_deref(), category) {
            (Some("directional"), NextIdleCategory::Micro) => vec![
                "idle_breathe",
                "idle_head_curiosity",
                "idle_shoulder_adjust",
                "idle_directional_hold",
            ],
            (Some("directional"), NextIdleCategory::Large) => {
                vec!["idle_breathe", "idle_weight_shift", "idle_directional_hold"]
            }
            (_, NextIdleCategory::Micro) => MICRO_IDLES.to_vec(),
            (_, NextIdleCategory::Large) => LARGE_IDLES.to_vec(),
        };
        if profile.as_deref() == Some("attentive") {
            candidates.push("idle_attentive_hold");
        }
        candidates.retain(|clip| self.last_idle.as_deref() != Some(*clip));
        candidates[self.rng.index(candidates.len())].to_owned()
    }

    fn tick_speaking<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
        analysis: Option<&SpeechAnalysis>,
        frame: Option<usize>,
    ) {
        let (Some(analysis), Some(frame)) = (analysis, frame) else {
            return;
        };
        let elapsed = frame as f64 * 0.020;
        if let Some(run_id) = self.speech_motion_run_id {
            if analysis.duration_seconds > self.speech_planned_until
                && self.speech_planned_until - elapsed < 1.5
            {
                if let Some(anchor) = self.status.active_anchor.clone() {
                    let tail = SpeechAnalysis {
                        rms_20ms: analysis.rms_20ms.iter().skip(frame).copied().collect(),
                        phrase_peaks: analysis
                            .phrase_peaks
                            .iter()
                            .filter_map(|peak| peak.checked_sub(frame))
                            .collect(),
                        quiet_regions: Vec::new(),
                        duration_seconds: (analysis.duration_seconds - elapsed).max(0.0),
                        streaming: analysis.streaming,
                    };
                    if let Ok(performance) =
                        self.compose_speech_performance(&tail, core.motions(), &anchor)
                    {
                        if core
                            .extend_character_performance(run_id, performance, anchor, now)
                            .is_ok()
                        {
                            self.speech_planned_until = analysis.duration_seconds;
                        }
                    }
                }
            }
        }
        if let Some(run_id) = self.speech_motion_run_id {
            if core
                .snapshot()
                .motion
                .as_ref()
                .is_some_and(|motion| motion.run_id == run_id && !motion.state.is_terminal())
            {
                return;
            }
            // A run absent from the active slot cannot still own movement.
            // The bounded terminal history may already contain a newer run.
            self.speech_motion_run_id = None;
            self.status.active_clip = None;
        }
        if self.speech_motion_started {
            return;
        }
        let Some(anchor) = self.status.active_anchor.clone() else {
            return;
        };
        let prior = self
            .thinking_run
            .take()
            .or_else(|| self.active_idle_run_id.take());
        self.active_idle_category = None;
        if core.mode() != RuntimeMode::Holding && prior.is_none() {
            return;
        }
        self.speech_motion_started = true;
        self.speech_planned_until = analysis.duration_seconds;
        let Ok(performance) = self.compose_speech_performance(analysis, core.motions(), &anchor)
        else {
            return;
        };
        // Speech motion remains best-effort: playback is never failed because
        // a generated performance could not be compiled or started.
        let result = if let Some(run) = prior.filter(|run| {
            core.snapshot()
                .motion
                .as_ref()
                .is_some_and(|m| m.run_id == *run && m.state == MovementPhase::Executing)
        }) {
            core.extend_character_performance(run, performance, anchor, now)
        } else {
            core.play_generated_anchored_relative(performance, anchor, now)
        };
        if let Ok(run_id) = result {
            self.speech_motion_run_id = Some(run_id);
            self.status.active_clip = Some("speaking_performance".into());
        }
    }

    fn compose_speech_performance(
        &mut self,
        analysis: &SpeechAnalysis,
        motions: &MotionLibrary,
        anchor: &JointPositions,
    ) -> Result<MotionDefinition> {
        let style = MotionStyle::named("speaking_emphatic")?;
        let performance_seconds = (analysis.duration_seconds
            + if analysis.streaming {
                1.5
            } else {
                -SPEECH_END_LEAD_SECONDS
            })
        .max(0.9);
        let authored_budget = performance_seconds * style.tempo;
        let settle_budget = (SPEECH_FINAL_SETTLE_SECONDS * style.tempo)
            .min(authored_budget * 0.35)
            .max(0.16);
        let active_budget = (authored_budget - settle_budget).max(0.24);
        let mut authored_seconds = 0.0;
        let mut drawings = Vec::new();
        let mut peak_cursor = 0;
        let mut gesture_index = self.speech_gesture_index;
        let mut last_tilt_direction = self.speech_last_tilt;
        let mut last_turn_direction = self.speech_last_turn;
        let mut last_body_beat = self.speech_last_body_beat;
        let maximum_rms = analysis.rms_20ms.iter().copied().fold(0.0_f64, f64::max);

        while active_budget - authored_seconds > 0.22 {
            let current_frame = ((authored_seconds / style.tempo) / 0.020).round() as usize;
            while analysis
                .phrase_peaks
                .get(peak_cursor)
                .is_some_and(|peak| *peak + 10 < current_frame)
            {
                peak_cursor += 1;
            }
            let phrase_peak = analysis
                .phrase_peaks
                .get(peak_cursor)
                .copied()
                .filter(|peak| *peak <= current_frame + SPEECH_PEAK_LOOKAHEAD_FRAMES);
            let emphasis = phrase_peak.is_some();
            if emphasis {
                peak_cursor += 1;
            }
            let energy_ratio = phrase_peak
                .and_then(|peak| analysis.rms_20ms.get(peak).copied())
                .map(|rms| rms / maximum_rms.max(f64::EPSILON))
                .unwrap_or(0.0);
            let body_beat_available = last_body_beat
                .is_none_or(|last| gesture_index - last >= SPEECH_BODY_BEAT_INTERVAL_DRAWINGS);
            let body_beat = emphasis
                && energy_ratio >= 0.72
                && body_beat_available
                && self.last_speech_clip.as_deref() != Some("speak_explanatory_lean");
            let clip = if body_beat {
                "speak_explanatory_lean".to_owned()
            } else {
                self.choose_speech_clip(emphasis)
            };
            let definition = motions.motion(&clip)?;
            let nominal_seconds = if emphasis { 0.72 } else { 1.05 }
                * SPEECH_GESTURE_DURATION_SCALE
                * self.rng.range(0.90, 1.10);
            let remaining = active_budget - authored_seconds;
            let fit = (remaining / nominal_seconds).min(1.0);
            if fit < 0.24 && !drawings.is_empty() {
                break;
            }

            let source = &definition.keyframes[0].target;
            let head_scale = if emphasis {
                self.rng.range(1.05, 1.24)
            } else {
                self.rng.range(0.88, 1.10)
            };
            let mut head_target = JointPositions::new();
            if let Some(source_roll) = source.get("head_roll_joint") {
                let direction = if last_tilt_direction >= 0.0 {
                    -1.0
                } else {
                    1.0
                };
                last_tilt_direction = direction;
                head_target.insert(
                    "head_roll_joint".into(),
                    source_roll.abs() * direction * head_scale,
                );
            } else {
                let direction = if last_tilt_direction >= 0.0 {
                    -1.0
                } else {
                    1.0
                };
                last_tilt_direction = direction;
                head_target.insert(
                    "head_roll_joint".into(),
                    direction * self.rng.range(0.045, 0.080) * head_scale,
                );
            }
            let head_pitch = source.get("head_pitch_joint").copied().unwrap_or_else(|| {
                if gesture_index % 3 == 2 {
                    -self.rng.range(0.030, 0.045)
                } else {
                    self.rng.range(0.040, 0.065)
                }
            });
            head_target.insert("head_pitch_joint".into(), head_pitch * head_scale);

            let anchor_yaw = anchor.get("base_yaw_joint").copied().unwrap_or(0.0);
            let turn_direction =
                self.choose_speech_turn_direction(anchor_yaw, last_turn_direction, emphasis);
            last_turn_direction = turn_direction;
            if turn_direction != 0 {
                let magnitude = if emphasis {
                    self.rng.range(0.070, 0.110)
                } else {
                    self.rng.range(0.045, 0.085)
                };
                head_target.insert("base_yaw_joint".into(), turn_direction as f64 * magnitude);
            }

            let body_scale = if body_beat {
                self.rng.range(0.78, 0.96)
            } else {
                self.rng.range(0.32, 0.48)
            };
            let source_shoulder = source
                .get("shoulder_pitch_joint")
                .copied()
                .unwrap_or_else(|| self.rng.range(-0.035, 0.035));
            let source_elbow = source
                .get("elbow_pitch_joint")
                .copied()
                .unwrap_or(-source_shoulder.signum() * self.rng.range(0.035, 0.050));
            let body_target = JointPositions::from([
                ("shoulder_pitch_joint".into(), source_shoulder * body_scale),
                ("elbow_pitch_joint".into(), source_elbow * body_scale),
            ]);

            let duration_seconds = nominal_seconds * fit;
            authored_seconds += duration_seconds;
            drawings.push(PlannedSpeechDrawing {
                clip: clip.clone(),
                head_target,
                body_target,
                duration_seconds,
                body_beat,
            });
            if body_beat {
                last_body_beat = Some(gesture_index);
            }
            self.last_speech_clip = Some(clip);
            gesture_index += 1;
        }

        if drawings.is_empty() {
            return Err(Error::Runtime(
                "Speech performance could not allocate an expressive keyframe.".into(),
            ));
        }

        // Each phrase is staged in two drawings: the head leads the thought,
        // then the shoulder and elbow follow while the head already begins the
        // next arc. This provides anticipation and overlapping action without
        // giving the secondary body motion a competing rhythmic oscillator.
        let mut keyframes = Vec::with_capacity(drawings.len() * 2 + 1);
        let mut previous_body = self.speech_previous_body.clone();
        for (index, drawing) in drawings.iter().enumerate() {
            let lead_fraction = self.rng.range(0.64, 0.74);
            keyframes.push(MotionKeyframe {
                pose_name: None,
                target: merge_speech_layers(&drawing.head_target, &previous_body),
                duration_seconds: drawing.duration_seconds * lead_fraction,
                arrival: KeyframeArrival::Through,
                hold_seconds: 0.0,
                marker: Some(format!("gesture_{index}_{}", drawing.clip)),
            });
            let next_head = drawings
                .get(index + 1)
                .map(|next| next.head_target.clone())
                .unwrap_or_default();
            let following_head = blend_speech_head(&drawing.head_target, &next_head, 0.18);
            keyframes.push(MotionKeyframe {
                pose_name: None,
                target: merge_speech_layers(&following_head, &drawing.body_target),
                duration_seconds: drawing.duration_seconds * (1.0 - lead_fraction),
                arrival: KeyframeArrival::Through,
                hold_seconds: 0.0,
                marker: Some(if drawing.body_beat {
                    format!("body_beat_{index}")
                } else {
                    format!("body_follow_{index}")
                }),
            });
            previous_body = drawing.body_target.clone();
        }
        self.speech_gesture_index = gesture_index;
        self.speech_last_body_beat = last_body_beat;
        self.speech_last_tilt = last_tilt_direction;
        self.speech_last_turn = last_turn_direction;
        self.speech_previous_body = previous_body;
        keyframes.push(MotionKeyframe {
            pose_name: None,
            target: JointPositions::new(),
            duration_seconds: (authored_budget - authored_seconds).max(0.12),
            arrival: KeyframeArrival::Settle,
            hold_seconds: 0.0,
            marker: Some("speech_settled".into()),
        });
        Ok(MotionDefinition {
            name: "speaking_performance".into(),
            description: "Utterance-length continuous speaking performance.".into(),
            space: MotionSpace::AnchorRelative,
            style,
            return_to_anchor: true,
            keyframes,
        })
    }

    fn choose_speech_turn_direction(
        &mut self,
        anchor_yaw: f64,
        previous: i8,
        emphasis: bool,
    ) -> i8 {
        let weighted: &[i8] = if emphasis {
            &[-1, 0, 0, 0, 1]
        } else {
            &[-1, -1, 0, 1, 1]
        };
        let candidates: Vec<i8> = weighted
            .iter()
            .copied()
            .filter(|direction| *direction != previous)
            .filter(|direction| !(anchor_yaw >= 0.75 && *direction > 0))
            .filter(|direction| !(anchor_yaw <= -0.75 && *direction < 0))
            .collect();
        candidates[self.rng.index(candidates.len())]
    }

    fn choose_speech_clip(&mut self, emphasis: bool) -> String {
        let weighted: &[(&str, u64)] = if emphasis {
            &[("speak_emphasis_nod", 3), ("speak_explanatory_lean", 2)]
        } else {
            &[
                ("speak_calm_sway", 5),
                ("speak_explanatory_lean", 2),
                ("speak_reflective_tilt", 3),
            ]
        };
        let candidates: Vec<(&str, u64)> = weighted
            .iter()
            .copied()
            .filter(|(clip, _)| self.last_speech_clip.as_deref() != Some(*clip))
            .collect();
        let total_weight: u64 = candidates.iter().map(|(_, weight)| weight).sum();
        let mut ticket = self.rng.next() % total_weight;
        for (clip, weight) in candidates {
            if ticket < weight {
                return clip.to_owned();
            }
            ticket -= weight;
        }
        unreachable!("positive speech weights always select a candidate")
    }

    fn capture_anchor<D: RuntimeDriver>(&mut self, core: &RuntimeCore<D>) {
        self.status.active_anchor = Some(
            core.snapshot()
                .joints
                .iter()
                .map(|joint| (joint.name.clone(), joint.position))
                .collect(),
        );
    }

    fn idle_state(&self) -> CharacterState {
        if self
            .status
            .active_anchor
            .as_ref()
            .and_then(|anchor| anchor.get("base_yaw_joint"))
            .is_some_and(|yaw| yaw.abs() < 0.05)
        {
            CharacterState::HomeIdle
        } else {
            CharacterState::PoseIdle
        }
    }

    fn reset_timers(&mut self, now: f64) {
        self.next_micro_at = now
            + self
                .rng
                .range(MICRO_IDLE_MIN_SECONDS, MICRO_IDLE_MAX_SECONDS);
        self.next_large_at = now
            + self
                .rng
                .range(LARGE_IDLE_MIN_SECONDS, LARGE_IDLE_MAX_SECONDS);
        self.update_next_idle_category();
    }

    fn reschedule_idle(&mut self, category: NextIdleCategory, now: f64) {
        match category {
            NextIdleCategory::Micro => {
                self.next_micro_at = now
                    + self
                        .rng
                        .range(MICRO_IDLE_MIN_SECONDS, MICRO_IDLE_MAX_SECONDS);
            }
            NextIdleCategory::Large => {
                self.next_large_at = now
                    + self
                        .rng
                        .range(LARGE_IDLE_MIN_SECONDS, LARGE_IDLE_MAX_SECONDS);
            }
        }
        self.update_next_idle_category();
    }

    fn update_next_idle_category(&mut self) {
        self.status.next_idle_category = Some(if self.next_micro_at <= self.next_large_at {
            NextIdleCategory::Micro
        } else {
            NextIdleCategory::Large
        });
    }

    fn finish_stop(&mut self) {
        self.thinking_run = None;
        self.clear_attention();
        self.status = CharacterStatus {
            enabled: false,
            state: CharacterState::Off,
            active_anchor: None,
            active_clip: None,
            next_idle_category: None,
        };
        self.active_idle_run_id = None;
        self.active_idle_category = None;
        self.starting_run_id = None;
        self.foreground_pending = false;
        self.foreground_scene_run_id = None;
        self.speech_motion_run_id = None;
        self.speech_motion_started = false;
    }
}

fn attention_return_motion() -> MotionDefinition {
    let mut motion = speech_settle_motion();
    motion.name = "attention_return".into();
    motion.style = MotionStyle::named("return_home").expect("built-in return style exists");
    motion.description = "Return from conversational attention to the previous anchor.".into();
    motion.keyframes[0].duration_seconds = 1.2;
    motion.keyframes[0].marker = Some("attention_returned".into());
    motion
}

fn merge_speech_layers(
    head_target: &JointPositions,
    body_target: &JointPositions,
) -> JointPositions {
    let mut target = head_target.clone();
    target.extend(body_target.clone());
    target
}

fn blend_speech_head(
    current: &JointPositions,
    next: &JointPositions,
    lookahead: f64,
) -> JointPositions {
    ["base_yaw_joint", "head_roll_joint", "head_pitch_joint"]
        .into_iter()
        .filter_map(|joint| {
            let current_value = current.get(joint).copied().unwrap_or(0.0);
            let next_value = next.get(joint).copied().unwrap_or(0.0);
            let blended = current_value * (1.0 - lookahead) + next_value * lookahead;
            (blended.abs() > f64::EPSILON).then(|| (joint.to_owned(), blended))
        })
        .collect()
}

fn thinking_motion() -> MotionDefinition {
    let mut motion = speech_settle_motion();
    motion.name = "thinking_head".into();
    motion.description = "Small head-led thought around the conversational anchor.".into();
    for (yaw, tilt) in [(0.05, 0.035), (-0.04, -0.025), (0.025, 0.02)]
        .into_iter()
        .rev()
    {
        motion.keyframes.insert(
            0,
            MotionKeyframe {
                pose_name: None,
                target: [
                    ("head_roll_joint".into(), yaw),
                    ("head_pitch_joint".into(), tilt),
                    ("base_yaw_joint".into(), yaw * 0.25),
                ]
                .into(),
                duration_seconds: 1.6,
                arrival: KeyframeArrival::Through,
                hold_seconds: 0.0,
                marker: None,
            },
        );
    }
    motion.keyframes.last_mut().unwrap().duration_seconds = 1.6;
    motion
}

fn speech_settle_motion() -> MotionDefinition {
    MotionDefinition {
        name: "speak_settle".into(),
        description: "Blend an interrupted speaking performance back to its anchor.".into(),
        space: MotionSpace::AnchorRelative,
        style: MotionStyle::named("speaking_calm").expect("built-in speaking style exists"),
        return_to_anchor: true,
        keyframes: vec![MotionKeyframe {
            pose_name: None,
            target: JointPositions::new(),
            duration_seconds: 0.42,
            arrival: KeyframeArrival::Settle,
            hold_seconds: 0.0,
            marker: Some("speech_settled".into()),
        }],
    }
}

fn checked(response: String) -> Result<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(&response)?;
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        Ok(value)
    } else {
        Err(Error::InvalidState(
            value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("character operation failed")
                .to_owned(),
        ))
    }
}

fn terminal_phase<D: RuntimeDriver>(core: &RuntimeCore<D>, run_id: u64) -> Option<MovementPhase> {
    [
        core.snapshot().motion.as_ref(),
        core.snapshot().last_motion.as_ref(),
    ]
    .into_iter()
    .flatten()
    .find(|motion| motion.run_id == run_id && motion.state.is_terminal())
    .map(|motion| motion.state)
}

fn closest_pose_profile<D: RuntimeDriver>(
    core: &RuntimeCore<D>,
    anchor: &JointPositions,
) -> Option<String> {
    core.poses()
        .names()
        .into_iter()
        .filter_map(|name| {
            let definition = core.poses().definition(&name).ok()?;
            let distance: f64 = definition
                .positions
                .iter()
                .map(|(joint, value)| (anchor[joint] - value).powi(2))
                .sum();
            Some((distance, definition.idle_profile.clone()))
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .and_then(|(_, profile)| profile)
}

fn closest_pose_default_lighting<D: RuntimeDriver>(
    core: &RuntimeCore<D>,
    anchor: &JointPositions,
) -> Option<String> {
    core.poses()
        .names()
        .into_iter()
        .filter_map(|name| {
            let definition = core.poses().definition(&name).ok()?;
            let distance: f64 = definition
                .positions
                .iter()
                .map(|(joint, value)| (anchor[joint] - value).powi(2))
                .sum();
            Some((distance, definition.default_lighting.clone()))
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .and_then(|(_, lighting)| lighting)
}

fn closest_pose_is_shutdown_only<D: RuntimeDriver>(
    core: &RuntimeCore<D>,
    anchor: &JointPositions,
) -> bool {
    core.poses()
        .names()
        .into_iter()
        .filter_map(|name| {
            let definition = core.poses().definition(&name).ok()?;
            let distance: f64 = definition
                .positions
                .iter()
                .map(|(joint, value)| (anchor[joint] - value).powi(2))
                .sum();
            let shutdown_only = definition
                .tags
                .iter()
                .any(|tag| matches!(tag.as_str(), "shutdown_only" | "mechanical"));
            Some((distance, shutdown_only))
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .is_some_and(|(_, shutdown_only)| shutdown_only)
}

#[derive(Debug)]
struct SeededRandom {
    state: u64,
}
impl SeededRandom {
    fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }
    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
    fn range(&mut self, lower: f64, upper: f64) -> f64 {
        lower + (upper - lower) * (self.next() as f64 / u64::MAX as f64)
    }
    fn index(&mut self, length: usize) -> usize {
        (self.next() % length as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use crate::ORION_JOINT_NAMES;
    use crate::driver::JointLimit;
    use crate::motion::{MotionLibrary, MotionSequence};
    use crate::pose::PoseLibrary;
    use crate::state::JointState;

    use super::*;

    struct CharacterTestDriver;

    impl RuntimeDriver for CharacterTestDriver {
        fn apply_servo_profile(&mut self) -> Result<()> {
            Ok(())
        }
        fn activate(&mut self) -> Result<Vec<JointState>> {
            Ok(self.read()?)
        }
        fn deactivate(&mut self) -> Result<()> {
            Ok(())
        }
        fn read(&mut self) -> Result<Vec<JointState>> {
            Ok(ORION_JOINT_NAMES
                .iter()
                .map(|name| JointState {
                    name: (*name).to_owned(),
                    position: 0.0,
                    velocity: 0.0,
                    current_ma: 0.0,
                    voltage_v: 7.4,
                    temperature_c: 25.0,
                    status: 0,
                })
                .collect())
        }
        fn write(&mut self, _positions_radians: &JointPositions) -> Result<()> {
            Ok(())
        }
        fn joint_limits(&self) -> Result<Vec<JointLimit>> {
            Ok(ORION_JOINT_NAMES
                .iter()
                .map(|name| JointLimit {
                    name: (*name).to_owned(),
                    lower_rad: -3.0,
                    upper_rad: 3.0,
                })
                .collect())
        }
        fn validate_positions(&self, _positions_radians: &JointPositions) -> Result<()> {
            Ok(())
        }
        fn clamp_positions_to_safe_range(
            &self,
            positions_radians: &JointPositions,
        ) -> Result<JointPositions> {
            Ok(positions_radians.clone())
        }
    }

    fn core() -> RuntimeCore<CharacterTestDriver> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let poses =
            PoseLibrary::load(root.join("motion/config/poses.yaml"), &ORION_JOINT_NAMES).unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        RuntimeCore::new(CharacterTestDriver, poses, motions).unwrap()
    }

    struct FollowingDriver {
        positions: JointPositions,
    }
    impl RuntimeDriver for FollowingDriver {
        fn apply_servo_profile(&mut self) -> Result<()> {
            Ok(())
        }
        fn activate(&mut self) -> Result<Vec<JointState>> {
            self.read()
        }
        fn deactivate(&mut self) -> Result<()> {
            Ok(())
        }
        fn read(&mut self) -> Result<Vec<JointState>> {
            let mut states = CharacterTestDriver.read()?;
            for state in &mut states {
                state.position = self.positions[&state.name];
            }
            Ok(states)
        }
        fn write(&mut self, positions: &JointPositions) -> Result<()> {
            self.positions = positions.clone();
            Ok(())
        }
        fn joint_limits(&self) -> Result<Vec<JointLimit>> {
            CharacterTestDriver.joint_limits()
        }
        fn validate_positions(&self, positions: &JointPositions) -> Result<()> {
            CharacterTestDriver.validate_positions(positions)
        }
        fn clamp_positions_to_safe_range(
            &self,
            positions: &JointPositions,
        ) -> Result<JointPositions> {
            Ok(positions.clone())
        }
    }

    fn following_core() -> RuntimeCore<FollowingDriver> {
        let source = core();
        let poses = source.poses().clone();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        let positions = poses.pose("home").unwrap().clone();
        RuntimeCore::new(FollowingDriver { positions }, poses, motions).unwrap()
    }

    fn advance<D: RuntimeDriver>(
        character: &mut CharacterCoordinator,
        core: &mut RuntimeCore<D>,
        start: f64,
        end: f64,
    ) {
        for step in 1..=((end - start) * 50.0) as usize {
            let now = start + step as f64 * 0.02;
            core.tick(now).unwrap();
            character
                .tick(now, core, false, None, false, None, None)
                .unwrap();
        }
    }

    #[test]
    fn cancelled_and_timed_out_startup_never_become_home_idle() {
        for cancel in [false, true] {
            let mut core = core();
            let mut character = CharacterCoordinator::new(1);
            character.start(0.0, &mut core).unwrap();
            if cancel {
                checked(core.handle_command("stop", 0.1)).unwrap();
            }
            advance(&mut character, &mut core, 0.1, 20.1);
            assert!(!character.status.enabled);
            assert_eq!(character.status.state, CharacterState::Off);
            assert!(character.status.active_anchor.is_none());
        }
    }

    #[test]
    fn attention_holds_conversation_anchor_and_returns_after_neutral() {
        for side in ["left", "right"] {
            let mut core = following_core();
            let mut character = CharacterCoordinator::new(1);
            character.start(0.0, &mut core).unwrap();
            advance(&mut character, &mut core, 0.0, 3.0);
            let original = character.status.active_anchor.clone().unwrap();
            character.attend(side, 0.9, 3.0, &mut core).unwrap();
            assert_eq!(character.status.active_anchor.as_ref(), Some(&original));
            advance(&mut character, &mut core, 3.0, 6.0);
            let facing = character.status.active_anchor.clone().unwrap();
            assert!((facing["base_yaw_joint"].abs() - 0.35).abs() < 0.001);
            character.set_reaction("thinking", 6.0, &mut core).unwrap();
            advance(&mut character, &mut core, 6.0, 10.0);
            assert_eq!(character.status.active_anchor.as_ref(), Some(&facing));
            character.set_reaction("neutral", 10.0, &mut core).unwrap();
            advance(&mut character, &mut core, 10.0, 30.0);
            assert_eq!(character.status.active_anchor.as_ref(), Some(&original));
            assert!(character.attention.is_none());
        }
    }

    #[test]
    fn attention_rejects_low_confidence_and_off_and_yields_to_foreground() {
        let mut core = following_core();
        let mut character = CharacterCoordinator::new(1);
        assert!(character.attend("left", 0.9, 0.0, &mut core).is_err());
        character.start(0.0, &mut core).unwrap();
        advance(&mut character, &mut core, 0.0, 3.0);
        assert!(character.attend("left", 0.5, 3.0, &mut core).is_err());
        assert!(character.attend("right", f64::NAN, 3.0, &mut core).is_err());
        character.attend("left", 0.9, 3.0, &mut core).unwrap();
        let original = character.status.active_anchor.clone();
        checked(core.handle_command("stop", 3.1)).unwrap();
        advance(&mut character, &mut core, 3.1, 3.2);
        assert_eq!(character.status.active_anchor, original);
        assert!(character.attention.is_none());
        character.attend("right", 0.9, 3.2, &mut core).unwrap();
        character.note_foreground_started(3.3);
        assert!(character.attention.is_none());
    }

    #[test]
    fn seeded_schedule_stays_in_contract_ranges() {
        let mut random = SeededRandom::new(42);
        for _ in 0..100 {
            assert!((8.0..=20.0).contains(&random.range(8.0, 20.0)));
            assert!((35.0..=75.0).contains(&random.range(35.0, 75.0)));
        }
    }

    #[test]
    fn completing_one_idle_category_preserves_the_other_deadline() {
        let mut character = CharacterCoordinator::new(42);
        character.next_micro_at = 10.0;
        character.next_large_at = 40.0;

        character.reschedule_idle(NextIdleCategory::Micro, 12.0);
        assert_eq!(character.next_large_at, 40.0);
        assert!((20.0..=32.0).contains(&character.next_micro_at));

        let micro_deadline = character.next_micro_at;
        character.reschedule_idle(NextIdleCategory::Large, 41.0);
        assert_eq!(character.next_micro_at, micro_deadline);
        assert!((76.0..=116.0).contains(&character.next_large_at));
    }

    #[test]
    fn idle_timeout_is_recoverable_and_preserves_the_anchor() {
        let mut core = core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();

        let anchor = core.poses().pose("home").unwrap().clone();
        let run_id = core
            .play_anchored_relative("idle_breathe", anchor.clone(), 0.0)
            .unwrap();
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::HomeIdle;
        character.status.active_anchor = Some(anchor.clone());
        character.status.active_clip = Some("idle_breathe".into());
        character.active_idle_run_id = Some(run_id);
        character.active_idle_category = Some(NextIdleCategory::Micro);
        character.next_large_at = 40.0;

        let mut now = 0.0;
        while terminal_phase(&core, run_id).is_none() {
            now += 0.1;
            core.tick(now).unwrap();
            assert!(now < 30.0, "idle did not reach a terminal phase");
        }
        assert_eq!(terminal_phase(&core, run_id), Some(MovementPhase::TimedOut));

        character
            .tick(now, &mut core, false, None, false, None, None)
            .unwrap();

        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
        assert_eq!(character.status.state, CharacterState::HomeIdle);
        assert!(character.active_idle_run_id.is_none());
        assert!(character.status.active_clip.is_none());
        assert!(
            (now + MICRO_IDLE_MIN_SECONDS..=now + MICRO_IDLE_MAX_SECONDS)
                .contains(&character.next_micro_at)
        );
        assert_eq!(character.next_large_at, 40.0);
    }

    #[test]
    fn priority_orders_scene_then_speech_then_reaction() {
        let core = &mut core();
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::Listening;
        character.status.active_anchor = Some(core.poses().pose("home").unwrap().clone());

        character
            .tick(1.0, core, true, None, true, None, None)
            .unwrap();
        assert_eq!(character.status.state, CharacterState::ForegroundScene);

        character
            .tick(1.1, core, false, None, true, None, None)
            .unwrap();
        assert_eq!(character.status.state, CharacterState::Speaking);

        character
            .tick(1.2, core, false, None, false, None, None)
            .unwrap();
        assert_eq!(character.status.state, CharacterState::HomeIdle);
    }

    #[test]
    fn speech_preserves_the_immutable_idle_anchor() {
        let core = &mut core();
        let anchor = core.poses().pose("home").unwrap().clone();
        let measured: JointPositions = core
            .snapshot()
            .joints
            .iter()
            .map(|joint| (joint.name.clone(), joint.position))
            .collect();
        assert_ne!(anchor, measured);

        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::HomeIdle;
        character.status.active_anchor = Some(anchor.clone());
        character.note_speech_started(1.0);
        character
            .tick(1.1, core, false, None, true, None, None)
            .unwrap();
        character
            .tick(1.2, core, false, None, false, None, None)
            .unwrap();

        assert_eq!(character.status.state, CharacterState::HomeIdle);
        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
        assert!(!character.foreground_pending);
    }

    #[test]
    fn every_speech_run_resets_its_continuous_performance() {
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.speech_motion_started = true;

        character.note_speech_started(1.0);

        assert!(!character.speech_motion_started);
    }

    #[test]
    fn utterance_length_speech_performance_flows_until_one_final_settle() {
        let core = core();
        let mut character = CharacterCoordinator::new(42);
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 1_000],
            quiet_regions: vec![(240, 255), (690, 705)],
            phrase_peaks: vec![60, 210, 360, 510, 660, 810, 940],
            duration_seconds: 20.0,
            streaming: false,
        };
        let anchor = core.poses().pose("home").unwrap().clone();
        let performance = character
            .compose_speech_performance(&analysis, core.motions(), &anchor)
            .unwrap();

        assert!(performance.keyframes.len() > 12);
        assert!(
            performance
                .keyframes
                .iter()
                .take(performance.keyframes.len() - 1)
                .all(|keyframe| keyframe.arrival == KeyframeArrival::Through)
        );
        assert_eq!(
            performance.keyframes.last().unwrap().arrival,
            KeyframeArrival::Settle
        );
        assert!(performance.keyframes.last().unwrap().target.is_empty());

        let markers = performance.markers();
        let gestures: Vec<_> = markers
            .iter()
            .filter(|marker| marker.starts_with("gesture_"))
            .collect();
        assert!(gestures.len() >= 8);
        assert!(
            gestures
                .windows(2)
                .all(|pair| pair[0].splitn(3, '_').nth(2) != pair[1].splitn(3, '_').nth(2))
        );

        let sequence = MotionSequence::new(&performance, anchor.clone()).unwrap();
        assert!((19.5..=20.1).contains(&sequence.duration_seconds()));
        for index in 0..sequence.keyframe_count() - 1 {
            let arrival = sequence.keyframe_arrival_time(index).unwrap();
            // A change of direction may have one instantaneous zero crossing,
            // but there must be commanded motion on both neighbouring 50 Hz
            // samples rather than a visible stopped plateau.
            let before = sequence.sample_state((arrival - 0.040).max(0.0)).unwrap();
            let after = sequence.sample_state(arrival + 0.040).unwrap();
            let before_speed: f64 = before.velocities.values().map(|value| value.abs()).sum();
            let after_speed: f64 = after.velocities.values().map(|value| value.abs()).sum();
            assert!(
                before_speed > 0.005,
                "stopped before speech keyframe {index}: {before_speed}"
            );
            assert!(
                after_speed > 0.005,
                "stopped after speech keyframe {index}: {after_speed}"
            );
        }
        assert_eq!(
            sequence.sample(sequence.duration_seconds()).unwrap(),
            anchor
        );
    }

    #[test]
    fn speech_performance_stages_head_first_and_keeps_body_beats_secondary() {
        let core = core();
        let anchor = core.poses().pose("home").unwrap().clone();
        let analysis = SpeechAnalysis {
            rms_20ms: (0..1_000)
                .map(|frame| 0.12 + (frame % 137) as f64 / 1_000.0)
                .collect(),
            quiet_regions: vec![],
            phrase_peaks: vec![60, 210, 360, 510, 660, 810, 940],
            duration_seconds: 20.0,
            streaming: false,
        };
        let mut character = CharacterCoordinator::new(42);
        let performance = character
            .compose_speech_performance(&analysis, core.motions(), &anchor)
            .unwrap();
        let active_keyframes = &performance.keyframes[..performance.keyframes.len() - 1];
        assert_eq!(active_keyframes.len() % 2, 0);

        let mut body_beat_indices = Vec::new();
        let mut ordinary_body_shapes = std::collections::BTreeSet::new();
        let mut turn_count = 0;
        for pair_start in (0..active_keyframes.len()).step_by(2) {
            let head_lead = &active_keyframes[pair_start];
            let body_follow = &active_keyframes[pair_start + 1];
            assert!(head_lead.marker.as_deref().unwrap().starts_with("gesture_"));
            let head_activity: f64 = ["base_yaw_joint", "head_roll_joint", "head_pitch_joint"]
                .into_iter()
                .map(|joint| head_lead.target.get(joint).copied().unwrap_or(0.0).abs())
                .sum();
            assert!(head_activity >= 0.08, "weak head lead: {head_activity}");
            turn_count += usize::from(
                head_lead
                    .target
                    .get("base_yaw_joint")
                    .is_some_and(|yaw| yaw.abs() >= 0.045),
            );

            let shoulder = body_follow
                .target
                .get("shoulder_pitch_joint")
                .copied()
                .unwrap_or(0.0)
                .abs();
            let elbow = body_follow
                .target
                .get("elbow_pitch_joint")
                .copied()
                .unwrap_or(0.0)
                .abs();
            let marker = body_follow.marker.as_deref().unwrap();
            if let Some(index) = marker.strip_prefix("body_beat_") {
                body_beat_indices.push(index.parse::<usize>().unwrap());
                assert!(shoulder >= 0.07, "body beat shoulder was not readable");
                assert!(elbow >= 0.09, "body beat elbow was not readable");
            } else {
                assert!(marker.starts_with("body_follow_"));
                assert!(shoulder <= 0.050, "ordinary shoulder competed with head");
                assert!(elbow <= 0.060, "ordinary elbow competed with head");
                assert!(
                    shoulder + elbow < head_activity,
                    "ordinary body action overtook the head lead"
                );
                ordinary_body_shapes
                    .insert(((shoulder * 10_000.0) as i64, (elbow * 10_000.0) as i64));
            }
        }

        let gesture_count = active_keyframes.len() / 2;
        assert!(turn_count >= gesture_count / 3);
        assert!(!body_beat_indices.is_empty());
        assert!(body_beat_indices.len() <= gesture_count.div_ceil(3));
        assert!(
            body_beat_indices
                .windows(2)
                .all(|pair| pair[1] - pair[0] >= 3)
        );
        assert!(ordinary_body_shapes.len() >= 4);
    }

    #[test]
    fn thinking_compiles_with_calibrated_limits_at_every_powered_anchor() {
        let mut core = core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();
        for name in core.poses().names() {
            let definition = core.poses().definition(&name).unwrap();
            if !definition.tags.iter().any(|tag| tag == "idle_anchor") {
                continue;
            }
            let anchor = definition.positions.clone();
            let mut character = CharacterCoordinator::new(42);
            character.status.enabled = true;
            character.status.active_anchor = Some(anchor);
            character.set_reaction("thinking", 0.0, &mut core).unwrap();
            character
                .tick(0.0, &mut core, false, None, false, None, None)
                .unwrap();
            assert!(
                character.thinking_run.is_some(),
                "thinking failed at {name}"
            );
            character.set_reaction("neutral", 0.1, &mut core).unwrap();
            assert!(character.thinking_run.is_none());
            assert_ne!(character.status.state, CharacterState::Thinking);
        }
    }

    #[test]
    fn thinking_is_head_led_and_speech_replaces_its_commanded_spline() {
        let mut core = core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();
        let anchor = core.poses().pose("home").unwrap().clone();
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.active_anchor = Some(anchor.clone());
        character.set_reaction("thinking", 0.0, &mut core).unwrap();
        character
            .tick(0.0, &mut core, false, None, false, None, None)
            .unwrap();
        let run = character
            .thinking_run
            .expect("thinking must compile and start");
        for frame in 1..50 {
            let now = frame as f64 * 0.02;
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
            assert_eq!(character.status.state, CharacterState::Thinking);
        }
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 200],
            quiet_regions: vec![],
            phrase_peaks: vec![40],
            duration_seconds: 4.0,
            streaming: true,
        };
        character.note_speech_started(1.0);
        character
            .tick(1.0, &mut core, false, None, true, Some(&analysis), Some(0))
            .unwrap();
        assert_eq!(character.speech_motion_run_id, Some(run));
        assert!(character.thinking_run.is_none());
        assert_eq!(character.status.active_anchor, Some(anchor));
        assert_eq!(character.status.state, CharacterState::Speaking);
        for frame in &thinking_motion().keyframes {
            assert!(
                frame
                    .target
                    .iter()
                    .all(|(name, offset)| name.starts_with("head_")
                        || (name == "base_yaw_joint" && offset.abs() <= 0.015))
            );
        }
    }

    #[test]
    fn streamed_speech_extends_without_resetting_anchor_or_settling_between_chunks() {
        let mut core = core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();
        let anchor = core.poses().pose("home").unwrap().clone();
        let mut analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 110],
            quiet_regions: vec![],
            phrase_peaks: vec![40],
            duration_seconds: 2.2,
            streaming: true,
        };
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::HomeIdle;
        character.status.active_anchor = Some(anchor.clone());
        character.note_speech_started(0.0);
        character
            .tick(0.0, &mut core, false, None, true, Some(&analysis), Some(0))
            .unwrap();
        let run = character.speech_motion_run_id.unwrap();
        for frame in 1..200 {
            let now = frame as f64 * 0.02;
            core.tick(now).unwrap();
            if frame % 40 == 0 {
                analysis.rms_20ms.extend(vec![0.2; 40]);
                analysis.duration_seconds += 0.8;
            }
            character
                .tick(
                    now,
                    &mut core,
                    false,
                    None,
                    true,
                    Some(&analysis),
                    Some(frame),
                )
                .unwrap();
            assert_eq!(character.speech_motion_run_id, Some(run));
            assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
            assert_eq!(character.status.state, CharacterState::Speaking);
        }
        character
            .tick(4.0, &mut core, false, None, false, None, None)
            .unwrap();
        assert_eq!(character.status.state, CharacterState::Settling);
        assert_eq!(
            character.status.active_clip.as_deref(),
            Some("speak_settle")
        );
    }

    #[test]
    fn ending_speech_interrupts_the_long_performance_and_blends_to_anchor() {
        let mut core = core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();
        let anchor = core.poses().pose("home").unwrap().clone();
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 500],
            quiet_regions: vec![],
            phrase_peaks: vec![80, 240, 400],
            duration_seconds: 10.0,
            streaming: false,
        };
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::HomeIdle;
        character.status.active_anchor = Some(anchor.clone());
        character.note_speech_started(0.0);

        character
            .tick(0.1, &mut core, false, None, true, Some(&analysis), Some(0))
            .unwrap();
        let performance_run = character.speech_motion_run_id.unwrap();
        assert_eq!(core.mode(), RuntimeMode::Moving);

        character
            .tick(0.2, &mut core, false, None, false, None, None)
            .unwrap();
        assert_eq!(character.status.state, CharacterState::Settling);
        assert_eq!(
            character.status.active_clip.as_deref(),
            Some("speak_settle")
        );
        assert_ne!(character.speech_motion_run_id, Some(performance_run));

        let mut now = 0.2;
        while character.status.state == CharacterState::Settling {
            now += 0.1;
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
            assert!(now < 10.0, "speech settle did not terminate");
        }
        assert_eq!(character.status.state, CharacterState::HomeIdle);
        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
        assert!(character.speech_motion_run_id.is_none());
    }

    #[test]
    fn neutral_playback_acknowledgement_preserves_settle_and_next_speech() {
        let mut core = following_core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();
        let anchor = core.poses().pose("home").unwrap().clone();
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 500],
            quiet_regions: vec![],
            phrase_peaks: vec![80, 240, 400],
            duration_seconds: 10.0,
            streaming: false,
        };
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::HomeIdle;
        character.status.active_anchor = Some(anchor.clone());
        character.note_speech_started(0.0);
        character
            .tick(0.1, &mut core, false, None, true, Some(&analysis), Some(0))
            .unwrap();
        for reaction in ["neutral", "listening", "thinking"] {
            character.set_reaction(reaction, 0.15, &mut core).unwrap();
            assert_eq!(character.status.state, CharacterState::Speaking);
        }
        core.tick(0.2).unwrap();
        character
            .tick(0.2, &mut core, false, None, false, None, None)
            .unwrap();
        let settle_run = character.speech_motion_run_id.unwrap();

        // Studio sees audio completion before physical settling has completed.
        character.set_reaction("neutral", 0.21, &mut core).unwrap();
        assert_eq!(character.status.state, CharacterState::Settling);
        advance(&mut character, &mut core, 0.21, 4.21);
        assert!(character.speech_motion_run_id.is_none());
        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));

        // A later idle replaces the runtime's most recent terminal movement.
        character.next_micro_at = 4.21;
        advance(&mut character, &mut core, 4.21, 12.21);
        assert_ne!(
            core.snapshot().last_motion.as_ref().unwrap().run_id,
            settle_run
        );
        character.preempt_idle(12.22, &mut core).unwrap();
        character.note_speech_started(12.22);
        character
            .tick(
                12.24,
                &mut core,
                false,
                None,
                true,
                Some(&analysis),
                Some(0),
            )
            .unwrap();
        assert_eq!(
            character.status.active_clip.as_deref(),
            Some("speaking_performance")
        );
        assert_eq!(core.mode(), RuntimeMode::Moving);
    }

    #[test]
    fn speech_recovers_when_prior_movement_left_terminal_history() {
        let mut core = following_core();
        checked(core.handle_command("configure", 0.0)).unwrap();
        checked(core.handle_command("enable", 0.0)).unwrap();
        let anchor = core.poses().pose("home").unwrap().clone();
        let prior = core
            .play_anchored_relative("idle_breathe", anchor.clone(), 0.0)
            .unwrap();
        for step in 1..=500 {
            core.tick(step as f64 * 0.02).unwrap();
        }
        core.play_anchored_relative("idle_breathe", anchor.clone(), 10.0)
            .unwrap();
        for step in 501..=1000 {
            core.tick(step as f64 * 0.02).unwrap();
        }
        assert!(terminal_phase(&core, prior).is_none());
        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.active_anchor = Some(anchor);
        character.speech_motion_run_id = Some(prior);
        character.note_speech_started(20.0);
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 500],
            quiet_regions: vec![],
            phrase_peaks: vec![80, 240, 400],
            duration_seconds: 10.0,
            streaming: false,
        };
        character
            .tick(
                20.02,
                &mut core,
                false,
                None,
                true,
                Some(&analysis),
                Some(0),
            )
            .unwrap();
        assert_eq!(
            character.status.active_clip.as_deref(),
            Some("speaking_performance")
        );
        assert_eq!(core.mode(), RuntimeMode::Moving);
    }

    #[test]
    fn generated_speech_performance_remains_readable_at_every_supported_anchor() {
        let core = core();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let calibration = crate::calibration::load_calibration_file(
            root.join("simulation/mujoco/config/servo_calibration.json"),
            &ORION_JOINT_NAMES,
        )
        .unwrap();
        let limits: Vec<JointLimit> = calibration
            .iter()
            .map(|joint| {
                let (lower_rad, upper_rad) = joint.safe_range_radians();
                JointLimit {
                    name: joint.name.clone(),
                    lower_rad,
                    upper_rad,
                }
            })
            .collect();
        let analysis = SpeechAnalysis {
            rms_20ms: vec![0.2; 1_000],
            quiet_regions: vec![],
            phrase_peaks: vec![100, 300, 500, 700, 900],
            duration_seconds: 20.0,
            streaming: false,
        };
        for anchor_name in ["home", "attentive", "look_left", "look_right"] {
            let anchor = core.poses().pose(anchor_name).unwrap().clone();
            let mut character = CharacterCoordinator::new(42);
            let performance = character
                .compose_speech_performance(&analysis, core.motions(), &anchor)
                .unwrap();
            let scale = performance
                .uniform_amplitude_scale(&anchor, &limits)
                .unwrap();
            assert!(scale > 0.75, "{anchor_name} collapsed to scale {scale}");
            let zero_velocity = anchor.keys().map(|joint| (joint.clone(), 0.0)).collect();
            let sequence = MotionSequence::compile_scaled_calibrated(
                &performance,
                anchor.clone(),
                zero_velocity,
                anchor.clone(),
                scale,
                &limits,
            )
            .unwrap();
            for sample in 0..=100 {
                let positions = sequence
                    .sample(sequence.duration_seconds() * sample as f64 / 100.0)
                    .unwrap();
                for limit in &limits {
                    assert!((limit.lower_rad..=limit.upper_rad).contains(&positions[&limit.name]));
                }
            }
            assert_eq!(
                sequence.sample(sequence.duration_seconds()).unwrap(),
                anchor
            );
        }
    }

    #[test]
    fn only_a_completed_scene_replaces_the_idle_anchor() {
        let core = &mut core();
        let home = core.poses().pose("home").unwrap().clone();
        let measured: JointPositions = core
            .snapshot()
            .joints
            .iter()
            .map(|joint| (joint.name.clone(), joint.position))
            .collect();
        assert_ne!(home, measured);

        let mut character = CharacterCoordinator::new(42);
        character.status.enabled = true;
        character.status.state = CharacterState::HomeIdle;
        character.status.active_anchor = Some(home.clone());
        character.note_foreground_scene_started(1.0, 7);
        character
            .tick(1.1, core, false, Some((7, false)), false, None, None)
            .unwrap();
        assert_eq!(character.status.active_anchor.as_ref(), Some(&home));

        character.note_foreground_scene_started(2.0, 8);
        character
            .tick(2.1, core, false, Some((8, true)), false, None, None)
            .unwrap();
        assert_eq!(character.status.active_anchor.as_ref(), Some(&measured));
    }

    #[test]
    fn idle_selection_never_immediately_repeats() {
        let core = core();
        let mut character = CharacterCoordinator::new(17);
        character.status.active_anchor = Some(core.poses().pose("home").unwrap().clone());
        for category in [NextIdleCategory::Micro, NextIdleCategory::Large] {
            let mut previous = None;
            for _ in 0..50 {
                character.last_idle = previous.clone();
                let selected = character.choose_idle(category, &core);
                assert_ne!(Some(selected.clone()), previous);
                previous = Some(selected);
            }
        }
    }

    #[test]
    fn directional_idles_avoid_yaw_clips_that_collapse_at_the_right_limit() {
        let core = core();
        let mut character = CharacterCoordinator::new(17);
        character.status.active_anchor = Some(core.poses().pose("look_right").unwrap().clone());

        for category in [NextIdleCategory::Micro, NextIdleCategory::Large] {
            for _ in 0..100 {
                let selected = character.choose_idle(category, &core);
                assert!(!matches!(
                    selected.as_str(),
                    "idle_micro_glance" | "idle_soft_head_shake"
                ));
                character.last_idle = Some(selected);
            }
        }
    }

    #[test]
    fn speech_selection_is_seeded_weighted_and_never_immediately_repeats() {
        let mut first = CharacterCoordinator::new(91);
        let mut second = CharacterCoordinator::new(91);
        let mut calm = 0;
        let mut explanatory = 0;
        let mut first_sequence = Vec::new();
        let mut second_sequence = Vec::new();

        for _ in 0..1_000 {
            first.last_speech_clip = None;
            let clip = first.choose_speech_clip(false);
            if clip == "speak_calm_sway" {
                calm += 1;
            } else if clip == "speak_explanatory_lean" {
                explanatory += 1;
            }
            first_sequence.push(clip);

            second.last_speech_clip = None;
            let clip = second.choose_speech_clip(false);
            second_sequence.push(clip);
        }

        assert_eq!(first_sequence, second_sequence);
        assert!(calm > explanatory * 2);

        for emphasis in [false, true] {
            let mut character = CharacterCoordinator::new(17);
            let mut previous = None;
            for _ in 0..100 {
                character.last_speech_clip = previous.clone();
                let selected = character.choose_speech_clip(emphasis);
                assert_ne!(Some(selected.clone()), previous);
                previous = Some(selected);
            }
        }
    }

    #[test]
    fn mechanical_rest_turns_character_off_without_scheduling_animation() {
        let mut core = core();
        let mut character = CharacterCoordinator::new(51);
        character.status.enabled = true;
        character.status.state = CharacterState::PoseIdle;
        character.status.active_anchor = Some(core.poses().pose("rest").unwrap().clone());
        character.status.next_idle_category = Some(NextIdleCategory::Micro);

        character
            .tick(100.0, &mut core, false, None, false, None, None)
            .unwrap();

        assert!(!character.status.enabled);
        assert_eq!(character.status.state, CharacterState::Off);
        assert!(character.status.active_anchor.is_none());
        assert!(character.status.active_clip.is_none());
        assert!(character.status.next_idle_category.is_none());
    }
}
