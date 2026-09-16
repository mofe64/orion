//! Pi-owned inactivity policy. Motion still goes through RuntimeCore; elapsed
//! time never substitutes for the measured completion of a particular run.
use serde::Serialize;

use super::voice_feedback::VoiceFeedback;
use crate::{
    CharacterCoordinator, CharacterState, Error, LightingDevice, MovementPhase, Result, Rgbw8,
    RuntimeCore, RuntimeDriver, RuntimeMode,
};

pub const DEFAULT_REST_AFTER_SECONDS: f64 = 1800.0;
const REST_FADE_SECONDS: f64 = 1.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RestState {
    #[default]
    Disabled,
    Awake,
    GoingToRest,
    Resting,
    Waking,
    Fault,
}

#[derive(Serialize)]
pub struct RestStatus {
    pub state: RestState,
    pub timeout_seconds: f64,
    pub last_confirmed_at: Option<f64>,
    pub remaining_seconds: Option<f64>,
    pub movement_run_id: Option<u64>,
    pub light_on: bool,
    pub error: Option<String>,
}

struct PendingAttention {
    session: String,
    side: String,
    expires_at: f64,
}

pub struct RestCoordinator {
    state: RestState,
    timeout: f64,
    deadline: Option<f64>,
    last_confirmed_at: Option<f64>,
    rest_run: Option<u64>,
    wake_session: Option<String>,
    attention: Option<PendingAttention>,
    error: Option<String>,
    pub light_on: bool,
}

impl RestCoordinator {
    pub fn new(timeout: f64) -> Self {
        Self {
            state: RestState::Disabled,
            timeout,
            deadline: None,
            last_confirmed_at: None,
            rest_run: None,
            wake_session: None,
            attention: None,
            error: None,
            light_on: false,
        }
    }

    pub fn status(&self, now: f64) -> RestStatus {
        RestStatus {
            state: self.state,
            timeout_seconds: self.timeout,
            last_confirmed_at: self.last_confirmed_at,
            remaining_seconds: self.deadline.map(|at| (at - now).max(0.0)),
            movement_run_id: self.rest_run,
            light_on: self.light_on,
            error: self.error.clone(),
        }
    }

    /// An explicit start arms automatic rest; maintenance startup and Stop do not.
    pub fn started(&mut self) {
        self.disable();
        self.state = RestState::Waking;
        self.last_confirmed_at = None;
    }

    pub fn disable(&mut self) {
        self.state = RestState::Disabled;
        self.deadline = None;
        self.rest_run = None;
        self.wake_session = None;
        self.attention = None;
        self.error = None;
    }

    /// Caller must first accept this session's ASR confirmation exactly once.
    pub fn confirmed(&mut self, session: &str, now: f64) {
        if matches!(self.state, RestState::Disabled | RestState::Fault) {
            return;
        }
        self.last_confirmed_at = Some(now);
        self.deadline = Some(now + self.timeout);
        if self.state != RestState::Awake {
            self.wake_session = Some(session.into());
        }
    }

    pub fn queue_attention(&mut self, session: &str, side: &str, expires_at: f64) {
        if !matches!(self.state, RestState::Disabled | RestState::Fault) {
            self.attention = Some(PendingAttention {
                session: session.into(),
                side: side.into(),
                expires_at,
            });
        }
    }

    pub fn track_rest(&mut self, response: &serde_json::Value) -> Result<()> {
        let run = response["run_id"]
            .as_u64()
            .ok_or_else(|| Error::Runtime("Rest movement has no run ID.".into()))?;
        self.state = RestState::GoingToRest;
        self.rest_run = Some(run);
        self.wake_session = None;
        self.attention = None;
        self.deadline = None;
        self.error = None;
        Ok(())
    }

    pub fn dark(&self) -> bool {
        matches!(
            self.state,
            RestState::GoingToRest | RestState::Resting | RestState::Fault
        )
    }

    pub fn reactions_ready(&self) -> bool {
        matches!(self.state, RestState::Awake | RestState::Disabled)
    }

    pub fn wake_acknowledgment_ready(&self) -> bool {
        self.reactions_ready() || self.state == RestState::Resting
    }

    pub fn speech_ready(&self, character: &CharacterCoordinator) -> bool {
        self.reactions_ready() && !character.attention_is_moving()
    }

    pub fn failed(&self) -> bool {
        self.state == RestState::Fault
    }

    pub fn transitioning(&self) -> bool {
        matches!(
            self.state,
            RestState::GoingToRest | RestState::Resting | RestState::Waking | RestState::Fault
        )
    }

    pub fn tick<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
        character: &mut CharacterCoordinator,
        feedback: &VoiceFeedback,
        foreground_busy: bool,
    ) {
        if let Err(error) = self.advance(now, core, character, feedback, foreground_busy) {
            self.state = RestState::Fault;
            self.error = Some(error.to_string());
            self.deadline = None;
            self.wake_session = None;
            self.attention = None;
            eprintln!("oriond: rest lifecycle failed; awaiting explicit recovery: {error}");
        }
    }

    fn advance<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
        character: &mut CharacterCoordinator,
        feedback: &VoiceFeedback,
        foreground_busy: bool,
    ) -> Result<()> {
        if self
            .wake_session
            .as_ref()
            .is_some_and(|id| !feedback.owns(id) || !feedback.confirmed_activity())
        {
            self.wake_session = None;
        }
        match self.state {
            RestState::GoingToRest => {
                let run = self.rest_run.expect("rest transition owns its run");
                if core
                    .snapshot()
                    .motion
                    .as_ref()
                    .is_some_and(|motion| motion.run_id == run)
                {
                    return Ok(());
                }
                let completed = core.snapshot().last_motion.as_ref().is_some_and(|motion| {
                    motion.run_id == run && motion.state == MovementPhase::Completed
                });
                if !completed {
                    return Err(Error::Runtime(
                        "Rest did not complete; torque has not been released.".into(),
                    ));
                }
                self.rest_run = None;
                if self.wake_session.is_some() {
                    // Finish the descent, then go home without cycling holding torque.
                    character.start(now, core)?;
                    self.state = RestState::Waking;
                } else {
                    checked(core.handle_command("disable", now))?;
                    self.state = RestState::Resting;
                }
            }
            RestState::Resting if self.wake_session.is_some() => {
                character.start(now, core)?;
                self.state = RestState::Waking;
            }
            RestState::Waking => {
                if !character.status().enabled {
                    return Err(Error::Runtime(
                        "Home startup did not complete; voice movement remains disabled.".into(),
                    ));
                }
                if character.status().state == CharacterState::Starting {
                    return Ok(());
                }
                if core.mode() != RuntimeMode::Holding {
                    return Ok(());
                }
                self.state = RestState::Awake;
                self.deadline.get_or_insert(now + self.timeout);
                self.wake_session = None;
                character.set_reaction(feedback.reaction(), now, core)?;
            }
            RestState::Awake => {
                if !character.status().enabled {
                    self.disable();
                    return Ok(());
                }
                if self.deadline.is_some_and(|deadline| now >= deadline)
                    && !foreground_busy
                    && !feedback.confirmed_activity()
                    && character.can_auto_rest(core)
                {
                    let response = character.rest(now, core)?;
                    self.track_rest(&response)?;
                    return Ok(());
                }
            }
            _ => {}
        }
        if self.state == RestState::Awake {
            if let Some(attention) = self.attention.take() {
                if feedback.owns(&attention.session)
                    && feedback.confirmed_activity()
                    && now < attention.expires_at
                {
                    // Direction is optional: refusal never prevents a valid voice turn.
                    let _ = character.attend(&attention.side, 0.75, now, core);
                    let _ = character.set_reaction(feedback.reaction(), now, core);
                }
            }
        }
        Ok(())
    }
}

fn checked(response: String) -> Result<()> {
    let response: serde_json::Value =
        serde_json::from_str(&response).map_err(|error| Error::Runtime(error.to_string()))?;
    if response["ok"] != true {
        return Err(Error::Runtime(
            response["error"]
                .as_str()
                .unwrap_or("Runtime command failed")
                .into(),
        ));
    }
    Ok(())
}

/// Apply rest darkness at the device boundary, including scene and manual lamp
/// writes. Keep the last visible frame for a smooth fade; preferences stay with
/// their original owners and cannot repaint the light while resting.
pub struct RestLighting {
    device: Box<dyn LightingDevice>,
    frame: Vec<Rgbw8>,
    fade: Option<(f64, Vec<Rgbw8>)>,
    now: f64,
}

impl RestLighting {
    pub fn new(device: Box<dyn LightingDevice>) -> Self {
        let frame = vec![Rgbw8::OFF; device.pixel_count()];
        Self {
            device,
            frame,
            fade: None,
            now: 0.0,
        }
    }

    pub fn update(&mut self, dark: bool, now: f64) -> Result<()> {
        self.now = now;
        if dark && self.fade.is_none() {
            self.fade = Some((now, self.frame.clone()));
        }
        if !dark {
            self.fade = None;
        }
        if dark {
            self.render(&self.frame.clone())?;
        }
        Ok(())
    }

    pub fn is_on(&self) -> bool {
        self.frame.iter().any(|color| *color != Rgbw8::OFF)
    }
}

impl LightingDevice for RestLighting {
    fn pixel_count(&self) -> usize {
        self.device.pixel_count()
    }
    fn render(&mut self, pixels: &[Rgbw8]) -> Result<()> {
        let frame = if let Some((start, frozen)) = &self.fade {
            frozen
                .iter()
                .map(|color| color.interpolate(Rgbw8::OFF, (self.now - start) / REST_FADE_SECONDS))
                .collect::<Result<Vec<_>>>()?
        } else {
            pixels.to_vec()
        };
        self.device.render(&frame)?;
        self.frame = frame;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use std::{cell::Cell, rc::Rc};

    struct FollowingDriver {
        positions: JointPositions,
        stall: Rc<Cell<bool>>,
        release_fails: Rc<Cell<bool>>,
        releases: Rc<Cell<usize>>,
    }
    impl RuntimeDriver for FollowingDriver {
        fn apply_servo_profile(&mut self) -> Result<()> {
            Ok(())
        }
        fn activate(&mut self) -> Result<Vec<JointState>> {
            self.read()
        }
        fn deactivate(&mut self) -> Result<()> {
            if self.release_fails.get() {
                return Err(Error::Runtime("release failed".into()));
            }
            self.releases.set(self.releases.get() + 1);
            Ok(())
        }
        fn read(&mut self) -> Result<Vec<JointState>> {
            Ok(self
                .positions
                .iter()
                .map(|(name, position)| JointState {
                    name: name.clone(),
                    position: *position,
                    velocity: 0.0,
                    current_ma: 0.0,
                    voltage_v: 7.4,
                    temperature_c: 25.0,
                    status: 0,
                })
                .collect())
        }
        fn write(&mut self, positions: &JointPositions) -> Result<()> {
            if !self.stall.get() {
                self.positions = positions.clone();
            }
            Ok(())
        }
        fn joint_limits(&self) -> Result<Vec<JointLimit>> {
            Ok(ORION_JOINT_NAMES
                .iter()
                .map(|name| JointLimit {
                    name: (*name).into(),
                    lower_rad: -3.0,
                    upper_rad: 3.0,
                })
                .collect())
        }
        fn validate_positions(&self, _: &JointPositions) -> Result<()> {
            Ok(())
        }
        fn clamp_positions_to_safe_range(
            &self,
            positions: &JointPositions,
        ) -> Result<JointPositions> {
            Ok(positions.clone())
        }
    }

    struct Fixture {
        core: RuntimeCore<FollowingDriver>,
        character: CharacterCoordinator,
        stall: Rc<Cell<bool>>,
        release_fails: Rc<Cell<bool>>,
        releases: Rc<Cell<usize>>,
    }

    impl Fixture {
        fn home() -> Self {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap();
            let poses =
                PoseLibrary::load(root.join("motion/config/poses.yaml"), &ORION_JOINT_NAMES)
                    .unwrap();
            let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
            let stall = Rc::new(Cell::new(false));
            let release_fails = Rc::new(Cell::new(false));
            let releases = Rc::new(Cell::new(0));
            let driver = FollowingDriver {
                positions: poses.pose("home").unwrap().clone(),
                stall: stall.clone(),
                release_fails: release_fails.clone(),
                releases: releases.clone(),
            };
            let mut fixture = Self {
                core: RuntimeCore::new(driver, poses, motions).unwrap(),
                character: CharacterCoordinator::new(42),
                stall,
                release_fails,
                releases,
            };
            fixture.character.start(0.0, &mut fixture.core).unwrap();
            fixture.advance_motion(0.0, 2.0);
            fixture
                .character
                .tick(2.0, &mut fixture.core, false, None, false, None, None)
                .unwrap();
            assert_eq!(
                fixture.core.snapshot().last_motion.as_ref().unwrap().state,
                MovementPhase::Completed
            );
            fixture
        }

        // Drive only the motion dependency to prepare a completion or failure.
        // Server-loop ordering is exercised by runtime/tests/test_rest.py.
        fn advance_motion(&mut self, from: f64, to: f64) {
            for step in 1..=((to - from) * 50.0).round() as usize {
                self.core.tick(from + step as f64 / 50.0).unwrap();
            }
        }

        fn policy_tick(&mut self, rest: &mut RestCoordinator, now: f64, busy: bool) {
            rest.tick(
                now,
                &mut self.core,
                &mut self.character,
                &VoiceFeedback::default(),
                busy,
            );
        }

        fn armed(&mut self, timeout: f64) -> RestCoordinator {
            let mut rest = RestCoordinator::new(timeout);
            rest.started();
            self.policy_tick(&mut rest, 2.0, false);
            assert_eq!(rest.status(2.0).state, RestState::Awake);
            rest
        }
    }

    #[test]
    fn deadline_arms_after_home_and_triggers_at_exact_confirmed_timeout() {
        let mut f = Fixture::home();
        let mut rest = f.armed(DEFAULT_REST_AFTER_SECONDS);
        assert_eq!(rest.status(2.0).remaining_seconds, Some(1800.0));
        assert_eq!(rest.status(2.0).last_confirmed_at, None);
        rest.confirmed("session", 10.0);
        assert_eq!(rest.status(10.0).last_confirmed_at, Some(10.0));
        f.policy_tick(&mut rest, 610.0, false);
        assert_eq!(rest.state, RestState::Awake);
        f.policy_tick(&mut rest, 1809.999, false);
        assert_eq!(rest.state, RestState::Awake);
        f.policy_tick(&mut rest, 1810.0, false);
        assert_eq!(rest.state, RestState::GoingToRest);
        assert_eq!(f.releases.get(), 0);
        assert!(f.core.snapshot().torque_enabled);
    }

    #[test]
    fn foreground_busy_defers_expired_deadline_without_extending_it() {
        let mut f = Fixture::home();
        let mut rest = f.armed(3.0);
        f.policy_tick(&mut rest, 6.0, true);
        assert_eq!(rest.state, RestState::Awake);
        assert_eq!(rest.status(6.0).remaining_seconds, Some(0.0));
        f.policy_tick(&mut rest, 6.0, false);
        assert_eq!(rest.state, RestState::GoingToRest);
    }

    #[test]
    fn rest_releases_torque_only_after_its_own_measured_completion() {
        let mut f = Fixture::home();
        let mut rest = f.armed(1.0);
        f.policy_tick(&mut rest, 3.0, false);
        f.advance_motion(3.0, 3.2);
        f.policy_tick(&mut rest, 3.2, false);
        assert_eq!(rest.state, RestState::GoingToRest);
        assert_eq!(f.releases.get(), 0);
        f.advance_motion(3.2, 11.0);
        f.policy_tick(&mut rest, 11.0, false);
        assert_eq!(rest.state, RestState::Resting);
        assert_eq!(f.releases.get(), 1);
        assert!(!f.core.snapshot().torque_enabled);
    }

    #[test]
    fn timeout_cancellation_release_failure_and_wrong_run_never_claim_resting() {
        for failure in ["timeout", "cancel", "release", "wrong_run"] {
            let mut f = Fixture::home();
            let mut rest = f.armed(1.0);
            f.stall.set(failure == "timeout");
            f.release_fails.set(failure == "release");
            f.policy_tick(&mut rest, 3.0, false);
            if failure == "cancel" {
                checked(f.core.handle_command("stop", 3.0)).unwrap();
            }
            if failure == "wrong_run" {
                rest.track_rest(&serde_json::json!({"run_id": 999}))
                    .unwrap();
            }
            f.advance_motion(3.0, 13.0);
            f.policy_tick(&mut rest, 13.0, false);
            assert_eq!(rest.state, RestState::Fault, "{failure}");
            assert!(f.core.snapshot().torque_enabled, "{failure}");
            assert_eq!(f.releases.get(), 0, "{failure}");
            assert!(rest.status(13.0).error.is_some());
            rest.confirmed("session", 14.0);
            assert!(rest.wake_session.is_none());
            assert!(!rest.speech_ready(&f.character));
        }
    }

    #[test]
    fn disabled_policy_ignores_confirmation_and_clears_pending_wake() {
        let mut rest = RestCoordinator::new(600.0);
        rest.confirmed("session", 1.0);
        assert!(rest.deadline.is_none());
        rest.started();
        rest.confirmed("session", 2.0);
        rest.queue_attention("session", "left", 5.0);
        rest.disable();
        assert_eq!(rest.state, RestState::Disabled);
        assert!(rest.deadline.is_none());
        assert!(rest.wake_session.is_none());
        assert!(rest.attention.is_none());
        rest.confirmed("session", 3.0);
        assert_eq!(rest.last_confirmed_at, Some(2.0));
    }

    #[test]
    fn darkness_fades_last_frame_and_blocks_every_later_write() {
        let mut light = RestLighting::new(Box::new(RecordingLightingDevice::orion()));
        light.render_uniform(Rgbw8::new(60, 40, 20, 80)).unwrap();
        light.update(true, 10.0).unwrap();
        assert!(light.is_on());
        light.update(true, 11.0).unwrap();
        assert!(!light.is_on());
        for color in [Rgbw8::new(255, 0, 0, 255), Rgbw8::new(0, 255, 0, 0)] {
            light.render_uniform(color).unwrap();
            assert!(!light.is_on());
        }
        light.update(false, 12.0).unwrap();
        light.render_uniform(Rgbw8::new(1, 2, 3, 4)).unwrap();
        assert!(light.is_on());
    }
}
