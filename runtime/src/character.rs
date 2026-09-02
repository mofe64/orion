use serde::Serialize;

use crate::daemon::RuntimeCore;
use crate::driver::RuntimeDriver;
use crate::pose::JointPositions;
use crate::speech::SpeechAnalysis;
use crate::state::{MovementPhase, RuntimeMode};
use crate::{Error, Result};

const MICRO_IDLE_MIN_SECONDS: f64 = 8.0;
const MICRO_IDLE_MAX_SECONDS: f64 = 20.0;
const LARGE_IDLE_MIN_SECONDS: f64 = 35.0;
const LARGE_IDLE_MAX_SECONDS: f64 = 75.0;

const MICRO_IDLES: [&str; 4] = [
    "idle_breathe",
    "idle_head_curiosity",
    "idle_micro_glance",
    "idle_shoulder_adjust",
];
const LARGE_IDLES: [&str; 3] = ["idle_weight_shift", "idle_soft_head_shake", "idle_breathe"];

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
    speech_motion_run_id: Option<u64>,
    next_speech_gesture_frame: usize,
    last_speech_clip: Option<String>,
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
            speech_motion_run_id: None,
            next_speech_gesture_frame: 20,
            last_speech_clip: None,
        }
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
        self.preempt_idle(now, core)?;
        self.reset_timers(now);
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
        if self.active_idle_run_id.is_some() && core.mode() == RuntimeMode::Moving {
            checked(core.handle_command("stop", now))?;
        }
        self.active_idle_run_id = None;
        self.active_idle_category = None;
        self.status.active_clip = None;
        Ok(())
    }

    pub fn note_foreground_started(&mut self, now: f64) {
        if !self.status.enabled {
            return;
        }
        self.foreground_pending = true;
        self.status.state = CharacterState::ForegroundScene;
        self.status.active_clip = None;
        self.reset_timers(now);
    }

    pub fn note_foreground_scene_started(&mut self, now: f64, run_id: u64) {
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
        if speech_active {
            if self.status.state != CharacterState::Speaking {
                if self.status.active_anchor.is_none() {
                    self.capture_anchor(core);
                }
                self.status.state = CharacterState::Speaking;
                self.next_speech_gesture_frame = 20 + self.rng.index(45);
            }
            self.tick_speaking(now, core, speech_analysis, speech_frame);
            return Ok(());
        }

        if matches!(
            self.status.state,
            CharacterState::Speaking | CharacterState::Settling
        ) {
            if let Some(run_id) = self.speech_motion_run_id {
                if terminal_phase(core, run_id).is_none() {
                    self.status.state = CharacterState::Settling;
                    return Ok(());
                }
                self.speech_motion_run_id = None;
            }
            self.status.active_clip = None;
            self.status.state = self.idle_state();
            self.reset_timers(now);
        }

        if let Some(run_id) = self.starting_run_id {
            if terminal_phase(core, run_id).is_some() {
                self.starting_run_id = None;
                if self.status.state == CharacterState::ShuttingDown {
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

        if let Some(run_id) = self.active_idle_run_id {
            if let Some(phase) = terminal_phase(core, run_id) {
                if phase != MovementPhase::Completed {
                    return Err(Error::Runtime(
                        "Autonomous idle did not return cleanly to its anchor.".into(),
                    ));
                }
                self.active_idle_run_id = None;
                let category = self.active_idle_category.take().ok_or_else(|| {
                    Error::Runtime("Autonomous idle lost its scheduling category.".into())
                })?;
                self.status.active_clip = None;
                self.status.state = self.idle_state();
                self.reschedule_idle(category, now);
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
        let mut candidates: Vec<&str> = match category {
            NextIdleCategory::Micro => MICRO_IDLES.to_vec(),
            NextIdleCategory::Large => LARGE_IDLES.to_vec(),
        };
        if profile.as_deref() == Some("attentive") {
            candidates.push("idle_attentive_hold");
        }
        if profile.as_deref() == Some("directional") {
            candidates.push("idle_directional_hold");
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
        if let Some(run_id) = self.speech_motion_run_id {
            if terminal_phase(core, run_id).is_some() {
                self.speech_motion_run_id = None;
                self.status.active_clip = None;
            } else {
                return;
            }
        }
        let (Some(analysis), Some(frame), Some(anchor)) =
            (analysis, frame, self.status.active_anchor.clone())
        else {
            return;
        };
        if frame < self.next_speech_gesture_frame || core.mode() != RuntimeMode::Holding {
            return;
        }
        let energy = analysis.rms_20ms.get(frame).copied().unwrap_or(0.0);
        let maximum = analysis.rms_20ms.iter().copied().fold(0.0, f64::max);
        if energy < (maximum * 0.16).max(0.004) {
            self.next_speech_gesture_frame = frame + 12;
            return;
        }
        let emphasis = analysis
            .phrase_peaks
            .iter()
            .any(|peak| peak.abs_diff(frame) <= 3);
        let clip = self.choose_speech_clip(emphasis);
        // Speech motion is deliberately best-effort: playback remains the
        // primary operation if a gesture cannot be compiled or started.
        if let Ok(run_id) = core.play_anchored_relative(&clip, anchor, now) {
            self.speech_motion_run_id = Some(run_id);
            self.status.active_clip = Some(clip.clone());
            self.last_speech_clip = Some(clip);
        }
        self.next_speech_gesture_frame = frame + 70 + self.rng.index(100);
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
    use crate::motion::MotionLibrary;
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
