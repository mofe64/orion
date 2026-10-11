//! Capture-grounded autonomous context. Detector IO and IK run at command time.
use super::*;
use crate::expression::curiosity::{CuriosityBudget, CuriosityContext, CuriosityKind};
use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct BehaviourSettings {
    pub object_glances: bool,
    pub mutual_gaze: bool,
    pub arrival_notice: bool,
    pub new_object_peek: bool,
    pub presence_defers_rest: bool,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MappedObject {
    pub id: String,
    #[serde(default)]
    pub interest_key: Option<String>,
    pub category: String,
    pub azimuth: f64,
    pub elevation: f64,
    pub capture_boottime_ns: String,
    pub point: Option<[f64; 3]>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NovelObject {
    pub id: String,
    pub interest_key: String,
    pub category: String,
    pub azimuth: f64,
    pub elevation: f64,
    pub point: Option<[f64; 3]>,
    pub capture_boottime_ns: u64,
    pub since_boottime_ns: u64,
}
impl NovelObject {
    fn valid(&self, stamp: u64) -> bool {
        self.id.starts_with("object:")
            && self.id.len() <= 96
            && self.interest_key.starts_with("object:")
            && self.interest_key.len() <= 96
            && !self.category.is_empty()
            && self.category.len() <= 32
            && self.category != "person"
            && self.azimuth.is_finite()
            && self.azimuth.abs() <= std::f64::consts::PI
            && self.elevation.is_finite()
            && self.elevation.abs() <= std::f64::consts::FRAC_PI_2
            && self
                .point
                .is_none_or(|p| p.into_iter().all(f64::is_finite) && p[2].abs() < 1e-6)
            && self.capture_boottime_ns <= stamp
            && stamp - self.capture_boottime_ns <= 1_000_000_000
            && self.since_boottime_ns <= self.capture_boottime_ns
            && self.capture_boottime_ns - self.since_boottime_ns <= 12_000_000_000
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeenFace {
    pub track_id: u64,
    pub azimuth: f64,
    pub elevation: f64,
    pub frontal: bool,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Surroundings {
    pub capture_boottime_ns: u64,
    pub camera_on: bool,
    pub tracking_mode: String,
    pub settings: BehaviourSettings,
    pub objects: Vec<MappedObject>,
    pub looking_away: Option<bool>,
    #[serde(default)]
    pub faces: Vec<SeenFace>,
    #[serde(default)]
    pub novelty: Option<NovelObject>,
    #[serde(default)]
    pub hand_count: u8,
    #[serde(default)]
    pub hand_point: Option<[f64; 3]>,
}
#[derive(Debug)]
struct ObjectGlance {
    id: String,
    interest_key: String,
    stamp: u64,
    anchor: JointPositions,
    definition: MotionDefinition,
}
#[derive(Debug, Default)]
pub(super) struct SurroundingsState {
    pub(super) budget: CuriosityBudget,
    received: Option<f64>,
    stamp: u64,
    pub(super) settings: BehaviourSettings,
    glance: Option<ObjectGlance>,
    last_object: Option<String>,
    glance_count: u8,
    active_object: bool,
    pub(super) runtime_allowed: bool,
    pub(super) mutual: super::reactions::MutualPolicy,
    pub(super) arrival: super::reactions::ArrivalPolicy,
    pub(super) last_novelty: Option<String>,
    pub(super) reaction: Option<super::reactions::Reaction>,
}
impl SurroundingsState {
    pub(super) fn clear(&mut self) {
        self.received = None;
        self.glance = None;
        self.settings = BehaviourSettings::default();
        self.mutual.clear();
        self.arrival.clear();
        // Budget and last target survive privacy/mode changes.
    }
    pub(super) fn fresh(&self, now: f64) -> bool {
        self.received
            .is_some_and(|at| (0.0..=2.0).contains(&(now - at)))
    }
}
impl CharacterCoordinator {
    pub(crate) fn clear_surroundings<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        self.surroundings.clear();
        self.leave_hands(now, core)?;
        self.settle_object_idle(now, core)?;
        self.settle_reaction(now, core)
    }
    pub(crate) fn set_curiosity_available(&mut self, allowed: bool) {
        self.surroundings.runtime_allowed = allowed;
    }
    pub(crate) fn curiosity_ready<D: RuntimeDriver>(&self, core: &RuntimeCore<D>) -> bool {
        self.curiosity_available()
            && self.surroundings.reaction.is_none()
            && core.mode() == RuntimeMode::Holding
    }
    pub(super) fn curiosity_available(&self) -> bool {
        self.surroundings.runtime_allowed
            && self.status.enabled
            && self.gaze.status().enabled
            && !self.gaze.is_stopped()
            && !self.hands.blocks_gaze()
            && self.work.is_none()
            && !self.work_anchor
            && self.attention.is_none()
            && !self.foreground_pending
            && self.foreground_scene_run_id.is_none()
            && self.speech_motion_run_id.is_none()
            && matches!(
                self.status.state,
                CharacterState::HomeIdle | CharacterState::PoseIdle
            )
    }
    pub(crate) fn observe_surroundings<D: RuntimeDriver>(
        &mut self,
        value: Surroundings,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        let boot = core
            .camera_boottime_ns()
            .ok_or_else(|| Error::InvalidState("V2 camera clock unavailable".into()))?;
        if value.hand_count > 8
            || value.hand_point.is_some_and(|p| {
                !p.into_iter().all(f64::is_finite)
                    || p[2].abs() > 1e-6
                    || p[1] >= 0.0
                    || p[0].hypot(p[1]) > 1.0
            })
            || value
                .novelty
                .as_ref()
                .is_some_and(|object| !object.valid(value.capture_boottime_ns))
            || value.objects.len() > 32
            || value.faces.len() > 16
            || value.faces.iter().any(|face| {
                face.track_id == 0
                    || !face.azimuth.is_finite()
                    || !face.elevation.is_finite()
                    || face.azimuth.abs() > std::f64::consts::PI
                    || face.elevation.abs() > std::f64::consts::FRAC_PI_2
            })
            || value.capture_boottime_ns <= self.surroundings.stamp
            || value.capture_boottime_ns > boot.saturating_add(50_000_000)
            || boot.saturating_sub(value.capture_boottime_ns) > 1_000_000_000
            || !matches!(value.tracking_mode.as_str(), "faces" | "off" | "hands")
        {
            return Err(Error::InvalidArgument(
                "Invalid or stale surroundings".into(),
            ));
        }
        self.surroundings.stamp = value.capture_boottime_ns;
        if !value.camera_on || value.tracking_mode == "off" {
            self.clear_surroundings(now, core)?;
            return Ok(());
        }
        self.surroundings.received = Some(now);
        self.surroundings.settings = value.settings;
        let _ = self
            .surroundings
            .budget
            .observe_focus(now, value.looking_away);
        if value.tracking_mode == "hands" {
            self.surroundings.mutual.clear();
            self.surroundings.arrival.clear();
            self.surroundings.glance = None;
            self.settle_reaction(now, core)?;
            return self.observe_hands(value.hand_count, value.hand_point, now, core);
        }
        self.leave_hands(now, core)?;
        if let Err(error) = self.observe_arrival(&value.faces, now, core) {
            eprintln!("oriond: arrival notice declined: {error}");
        }
        if let Err(error) = self.observe_novelty(value.novelty.as_ref(), now, core) {
            eprintln!("oriond: object peek declined: {error}");
        }
        if let Err(error) = self.observe_mutual(&value.faces, now, core) {
            eprintln!("oriond: mutual gaze declined: {error}");
        }
        if !self.surroundings.settings.object_glances {
            self.surroundings.glance = None;
            return Ok(());
        }
        let Some(anchor) = self.status.active_anchor.as_ref() else {
            return Ok(());
        };
        let chosen = value.objects.iter().find(|object| {
            object.id.len() <= 96
                && object.id.starts_with("object:")
                && self.surroundings.last_object.as_ref() != Some(&object.id)
                && object
                    .interest_key
                    .as_ref()
                    .is_none_or(|key| key.starts_with("object:") && key.len() <= 96)
                && object.category != "person"
                && !object.category.is_empty()
                && object.azimuth.is_finite()
                && object.azimuth.abs() <= std::f64::consts::PI
                && object.elevation.is_finite()
                && object.elevation.abs() <= std::f64::consts::FRAC_PI_2
                && object
                    .point
                    .is_none_or(|point| point.into_iter().all(f64::is_finite))
                && object
                    .capture_boottime_ns
                    .parse::<u64>()
                    .is_ok_and(|stamp| stamp <= boot && boot - stamp < 600_000_000_000)
        });
        let Some(object) = chosen else {
            self.surroundings.glance = None;
            return Ok(());
        };
        let stamp = object.capture_boottime_ns.parse().unwrap();
        if self.surroundings.glance.as_ref().is_some_and(|cached| {
            cached.id == object.id && cached.stamp == stamp && &cached.anchor == anchor
        }) {
            return Ok(());
        }
        let model = core
            .camera_kinematics()
            .ok_or_else(|| Error::InvalidState("V2 geometry unavailable".into()))?;
        let limits = core.driver().joint_limits()?;
        // Preserve the authored glance envelope. A distant object supplies a
        // direction, never permission to enlarge or speed up the idle clip.
        let delta = object.azimuth - anchor["base_yaw_joint"];
        let yaw = delta.sin().atan2(delta.cos()).clamp(-0.11, 0.11);
        let mut facing = anchor.clone();
        *facing.get_mut("base_yaw_joint").unwrap() += yaw;
        let solved = crate::expression::gaze::solve_aim(
            model,
            &facing,
            object.azimuth,
            object.elevation,
            &limits,
        )?
        .0;
        let neck = (solved["head_roll_joint"] - anchor["head_roll_joint"]).clamp(-0.07, 0.07);
        let mut definition = core.motions().motion("idle_micro_glance")?.clone();
        for frame in &mut definition.keyframes {
            if let Some(offset) = frame.target.get_mut("base_yaw_joint") {
                *offset *= yaw / 0.11;
            }
            if let Some(offset) = frame.target.get_mut("head_roll_joint") {
                *offset *= neck / -0.07;
            }
        }
        // Command-time preflight uses the same calibrated compiler as execution.
        let scale = definition.uniform_amplitude_scale(anchor, &limits)?;
        crate::MotionSequence::compile_scaled_calibrated(
            &definition,
            anchor.clone(),
            anchor.keys().map(|key| (key.clone(), 0.0)).collect(),
            None,
            anchor.clone(),
            scale,
            &limits,
        )?;
        self.surroundings.glance = Some(ObjectGlance {
            id: object.id.clone(),
            interest_key: object
                .interest_key
                .clone()
                .unwrap_or_else(|| object.id.clone()),
            stamp,
            anchor: anchor.clone(),
            definition,
        });
        Ok(())
    }
    pub(super) fn object_idle<D: RuntimeDriver>(
        &mut self,
        clip: &str,
        now: f64,
        core: &RuntimeCore<D>,
    ) -> Option<MotionDefinition> {
        if clip != "idle_micro_glance" {
            return None;
        }
        self.surroundings.glance_count = (self.surroundings.glance_count + 1) % 3;
        if self.surroundings.glance_count != 0
            || !self.surroundings.fresh(now)
            || !self.surroundings.settings.object_glances
            || !self.curiosity_ready(core)
        {
            return None;
        }
        let glance = self.surroundings.glance.as_ref()?;
        if self.status.active_anchor.as_ref() != Some(&glance.anchor)
            || self.surroundings.last_object.as_ref() == Some(&glance.id)
            || core
                .camera_boottime_ns()
                .is_none_or(|boot| boot.saturating_sub(glance.stamp) >= 600_000_000_000)
        {
            return None;
        }
        let context = CuriosityContext {
            enabled: true,
            camera_on: true,
            tracking_on: true,
            powered_and_awake: true,
            character_available: true,
            ..Default::default()
        };
        self.surroundings
            .budget
            .try_spend(now, &glance.interest_key, CuriosityKind::Glance, context)
            .ok()?;
        self.surroundings.last_object = Some(glance.id.clone());
        self.surroundings.active_object = true;
        Some(glance.definition.clone())
    }
    pub(super) fn settle_object_idle<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<()> {
        if !self.surroundings.active_object {
            return Ok(());
        }
        let Some(run) = self.active_idle_run_id else {
            self.surroundings.active_object = false;
            return Ok(());
        };
        if self.surroundings.fresh(now)
            && self.surroundings.runtime_allowed
            && self.surroundings.settings.object_glances
            && self.gaze.status().enabled
        {
            return Ok(());
        }
        self.surroundings.active_object = false;
        if core
            .snapshot()
            .motion
            .as_ref()
            .is_some_and(|motion| motion.run_id == run && motion.state == MovementPhase::Executing)
            && let Some(anchor) = self.status.active_anchor.clone()
        {
            core.extend_character_performance(run, attention_return_motion(), anchor, now)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    #[test]
    fn object_glances_preserve_clip_envelope_budget_anchor_and_stop() {
        let (mut core, _, boot) = crate::expression::gaze::tests::following();
        let mut character = CharacterCoordinator::new(1);
        character.set_gaze_enabled(true);
        character.start(0., &mut core).unwrap();
        for step in 1..=250 {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
        }
        character.set_curiosity_available(true);
        let anchor = character.status.active_anchor.clone().unwrap();
        let message = |stamp: u64| {
            serde_json::from_value(serde_json::json!({
                "capture_boottime_ns":stamp, "camera_on":true, "tracking_mode":"faces",
                "settings":{"object_glances":true}, "looking_away":null,
                "objects":[{"id":"object:1","category":"book","azimuth":0.4,"elevation":-0.3,
                    "capture_boottime_ns":stamp.to_string(),"point":null},
                    {"id":"object:2","category":"cup","azimuth":-0.4,"elevation":-0.3,
                    "capture_boottime_ns":stamp.to_string(),"point":null}]
            }))
            .unwrap()
        };
        character
            .observe_surroundings(message(boot.load(Ordering::Relaxed)), 5., &mut core)
            .unwrap();
        assert!(
            character
                .object_idle("idle_micro_glance", 5., &core)
                .is_none()
        );
        assert!(
            character
                .object_idle("idle_micro_glance", 5., &core)
                .is_none()
        );
        let clip = character
            .object_idle("idle_micro_glance", 5., &core)
            .unwrap();
        assert_eq!(clip.name, "idle_micro_glance");
        for frame in &clip.keyframes {
            assert!(frame.target.get("base_yaw_joint").unwrap_or(&0.).abs() <= 0.11);
            assert!(frame.target.get("head_roll_joint").unwrap_or(&0.).abs() <= 0.07);
        }
        assert!(
            clip.keyframes
                .last()
                .unwrap()
                .target
                .values()
                .all(|q| *q == 0.)
        );
        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
        boot.fetch_add(1, Ordering::Relaxed);
        character
            .observe_surroundings(message(boot.load(Ordering::Relaxed)), 5.1, &mut core)
            .unwrap();
        assert_eq!(
            character.surroundings.glance.as_ref().unwrap().id,
            "object:2"
        );
        // Even another eligible target cannot bypass the common one-minute spacing.
        for _ in 0..3 {
            assert!(
                character
                    .object_idle("idle_micro_glance", 5.1, &core)
                    .is_none()
            );
        }
        // Clearing the camera context never replenishes the shared budget.
        character.surroundings.clear();
        boot.fetch_add(1, Ordering::Relaxed);
        character
            .observe_surroundings(message(boot.load(Ordering::Relaxed)), 5.2, &mut core)
            .unwrap();
        for _ in 0..3 {
            assert!(
                character
                    .object_idle("idle_micro_glance", 5.2, &core)
                    .is_none()
            );
        }
        for _ in 0..3 {
            assert!(
                character
                    .object_idle("idle_micro_glance", 70., &core)
                    .is_none()
            );
        }
        let run = core
            .play_generated_anchored_relative(clip, anchor.clone(), 5.2)
            .unwrap();
        character.active_idle_run_id = Some(run);
        character.active_idle_category = Some(NextIdleCategory::Micro);
        core.tick(5.3).unwrap();
        character.clear_surroundings(5.3, &mut core).unwrap();
        for step in 266..=500 {
            let now = step as f64 * 0.02;
            boot.store(1_000_000_000 + (now * 1e9) as u64, Ordering::Relaxed);
            core.tick(now).unwrap();
            character
                .tick(now, &mut core, false, None, false, None, None)
                .unwrap();
        }
        assert_eq!(core.mode(), RuntimeMode::Holding);
        assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
        for joint in &core.snapshot().joints {
            assert!((joint.position - anchor[&joint.name]).abs() < 0.01);
        }
        character.forget_gaze_for_external_stop(crate::expression::gaze::GazeStopCommand::Stop);
        character.set_curiosity_available(true);
        assert!(!character.curiosity_ready(&core));
    }
}
