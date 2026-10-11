//! Ownership across lookup, authored scan stops, and measured aiming completion.
use super::*;
use serde::Deserialize;
use serde_json::{Value, json};

// Gateway reserves time for this lease plus command/reply and voice cleanup.
const END_RECOVERY_LEASE_SECONDS: f64 = 8.;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum WorkRequest {
    Begin {
        token: String,
    },
    Scan {
        token: String,
        index: usize,
    },
    Target {
        token: String,
        point: [f64; 3],
        lean: f64,
    },
    End {
        token: String,
    },
    Status {
        token: String,
    },
}

#[derive(Debug)]
pub(super) struct WorkState {
    token: String,
    previous: JointPositions,
    previous_work_anchor: bool,
    pub(super) run: Option<u64>,
    aimed: Option<JointPositions>,
    committed: bool,
    returning: bool,
    failed: bool,
    deadline: f64,
}

impl CharacterCoordinator {
    pub(crate) fn release_work_anchor(&mut self) {
        self.work_anchor = false;
        // Idle releases gaze ownership, but the reach remains the character's
        // anchor until another pose replaces it. Keep its comfort fitting.
    }

    fn finish_work_hold<D: RuntimeDriver>(
        &mut self,
        held: bool,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) {
        self.work = None;
        self.work_anchor = held;
        core.set_character_comfort(held || self.hands.owns_anchor());
        self.status.state = self.idle_state();
        self.reset_timers(now);
    }

    pub(crate) fn work_request<D: RuntimeDriver>(
        &mut self,
        request: WorkRequest,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<Value> {
        let token = match &request {
            WorkRequest::Begin { token }
            | WorkRequest::Scan { token, .. }
            | WorkRequest::Target { token, .. }
            | WorkRequest::End { token }
            | WorkRequest::Status { token } => token,
        };
        if token.len() != 32 || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::InvalidArgument("Invalid aiming owner".into()));
        }
        if let WorkRequest::Begin { token } = request {
            if self.work.is_some()
                || !self.status.enabled
                || !self.gaze.status().enabled
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
            {
                return Err(Error::InvalidState(
                    "Aiming requires an awake, unstopped, available character".into(),
                ));
            }
            let previous = self
                .status
                .active_anchor
                .clone()
                .ok_or_else(|| Error::InvalidState("No character anchor".into()))?;
            // Prove the recovery path before taking ownership or stopping gaze.
            core.prepare_work_return(&previous)?;
            let previous_work_anchor = self.work_anchor;
            self.preempt_for_foreground(now, core)?;
            if core.mode() != RuntimeMode::Holding {
                return Err(Error::InvalidState(
                    "Aiming cannot interrupt foreground movement".into(),
                ));
            }
            self.attention = None;
            self.work = Some(WorkState {
                token,
                previous,
                previous_work_anchor,
                run: None,
                aimed: None,
                committed: false,
                returning: false,
                failed: false,
                deadline: now + 60.,
            });
            self.status.state = CharacterState::ForegroundScene;
            return Ok(json!({"ok":true,"status":"ready"}));
        }
        self.tick_work(now, core)?;
        let work = self
            .work
            .as_mut()
            .filter(|work| work.token == *token)
            .ok_or_else(|| Error::InvalidState("Aiming owner is stale or revoked".into()))?;
        if matches!(request, WorkRequest::Status { .. }) {
            return Ok(
                json!({"ok":true,"run_id":work.run,"committed":work.committed,"failed":work.failed}),
            );
        }
        if matches!(request, WorkRequest::End { .. }) {
            if work.run.is_some() {
                checked(core.handle_command("stop", now))?;
                work.run = None;
            }
            work.aimed = None;
            work.failed = false;
            work.deadline = work.deadline.min(now + END_RECOVERY_LEASE_SECONDS);
        }
        if work.run.is_some() || work.failed {
            return Err(Error::InvalidState(
                "Wait for successful measured completion before continuing the search".into(),
            ));
        }
        match request {
            WorkRequest::Target { point, lean, .. } => {
                let (solution, prepared) =
                    core.prepare_light_target(&work.previous, point, lean)?;
                let Some(prepared) = prepared else {
                    return Ok(json!({"ok":true,"status":"unreachable"}));
                };
                let run = core.install_work_motion(prepared, now)?;
                work.run = Some(run);
                work.aimed = solution.positions;
                Ok(
                    json!({"ok":true,"status":solution.status,"aim_error_rad":solution.aim_error_rad,"lean_limited":solution.lean_limited,"run_id":run}),
                )
            }
            WorkRequest::Scan { index, .. } => {
                let prepared = core.prepare_work_scan(&work.previous, index)?;
                let run = core.install_work_motion(prepared, now)?;
                work.run = Some(run);
                Ok(json!({"ok":true,"run_id":run}))
            }
            WorkRequest::End { .. } => {
                if work.committed {
                    self.finish_work_hold(true, now, core);
                    Ok(json!({"ok":true}))
                } else {
                    let prepared = core.prepare_work_return(&work.previous)?;
                    let run = core.install_work_motion(prepared, now)?;
                    work.run = Some(run);
                    work.returning = true;
                    Ok(json!({"ok":true,"run_id":run}))
                }
            }
            _ => unreachable!(),
        }
    }

    /// Bounded lifecycle work only. Planning is performed by work_request.
    pub(super) fn tick_work<D: RuntimeDriver>(
        &mut self,
        now: f64,
        core: &mut RuntimeCore<D>,
    ) -> Result<bool> {
        let Some(work) = &mut self.work else {
            return Ok(false);
        };
        if let Some(run) = work.run {
            if let Some(phase) = terminal_phase(core, run) {
                work.run = None;
                work.failed = phase != MovementPhase::Completed;
                if !work.failed {
                    if let Some(target) = work.aimed.take() {
                        self.status.active_anchor = Some(target);
                        work.committed = true;
                        self.work_anchor = true;
                        core.set_character_comfort(true);
                    }
                    if work.returning {
                        self.status.active_anchor = Some(work.previous.clone());
                        let held = work.previous_work_anchor;
                        self.finish_work_hold(held, now, core);
                        return Ok(false);
                    }
                }
            }
        }
        if now >= work.deadline {
            if work.run.is_some_and(|run| {
                core.snapshot()
                    .motion
                    .as_ref()
                    .is_some_and(|m| m.run_id == run)
            }) {
                checked(core.handle_command("stop", now))?;
            }
            let held = work.committed || work.previous_work_anchor;
            self.finish_work_hold(held, now, core);
            return Ok(false);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expression::gaze::{GazeStopCommand, tests::following};

    #[test]
    fn end_recovers_a_failed_scan_promptly_and_bounds_the_return_lease() {
        let (mut core, _, _) = following();
        core.configure_comfort_box(crate::HardwareVersion::V2.profile().comfort_box.clone().unwrap()).unwrap();
        let mut character = CharacterCoordinator::new(1);
        character.set_gaze_enabled(true);
        character.start(0., &mut core).unwrap();
        for i in 1..=250 {
            let t = i as f64 * 0.02;
            core.tick(t).unwrap();
            character.tick(t, &mut core, false, None, false, None, None).unwrap();
        }
        let token = "a".repeat(32);
        character.work_request(WorkRequest::Begin { token: token.clone() }, 5., &mut core).unwrap();
        character.work_request(WorkRequest::Scan { token: token.clone(), index: 0 }, 5., &mut core).unwrap();
        core.tick(5.5).unwrap();
        checked(core.handle_command("stop", 5.5)).unwrap();
        character.tick_work(5.5, &mut core).unwrap();
        assert!(character.work.as_ref().unwrap().failed);
        let ended = character.work_request(WorkRequest::End { token }, 5.5, &mut core).unwrap();
        assert!(ended["run_id"].is_u64());
        assert!(!character.work.as_ref().unwrap().failed);
        assert_eq!(character.work.as_ref().unwrap().deadline, 5.5 + END_RECOVERY_LEASE_SECONDS);
        // Even if measured completion never arrives, End's recovery cannot
        // retain motion until the original sixty-second search lease.
        character.tick_work(5.5 + END_RECOVERY_LEASE_SECONDS, &mut core).unwrap();
        assert!(character.work.is_none());
        assert!(core.snapshot().motion.is_none());
    }

    #[test]
    fn end_and_expiry_keep_fitted_reach_through_soft_stop_and_confirmed_wake() {
        for expired in [false, true] {
            let (mut core, _, _) = following();
            let config = crate::HardwareVersion::V2
                .profile()
                .comfort_box
                .clone()
                .unwrap();
            let comfort = crate::motion::ik::ComfortBox::from_config(
                &config,
                core.poses(),
                &core.driver().joint_limits().unwrap(),
            )
            .unwrap();
            core.configure_comfort_box(config).unwrap();
            let mut character = CharacterCoordinator::new(1);
            character.set_gaze_enabled(true);
            character.start(0., &mut core).unwrap();
            for i in 1..=250 {
                let t = i as f64 * 0.02;
                core.tick(t).unwrap();
                character
                    .tick(t, &mut core, false, None, false, None, None)
                    .unwrap();
            }
            let token = "a".repeat(32);
            character
                .work_request(
                    WorkRequest::Begin {
                        token: token.clone(),
                    },
                    5.,
                    &mut core,
                )
                .unwrap();
            character
                .work_request(
                    WorkRequest::Target {
                        token: token.clone(),
                        point: [0., -0.28, 0.],
                        lean: 0.8,
                    },
                    5.,
                    &mut core,
                )
                .unwrap();
            for i in 251..=750 {
                let t = i as f64 * 0.02;
                core.tick(t).unwrap();
                character
                    .tick(t, &mut core, false, None, false, None, None)
                    .unwrap();
            }
            assert!(character.work.as_ref().unwrap().committed);
            let anchor = character.status.active_anchor.clone().unwrap();
            assert!(anchor["shoulder_pitch_joint"] > 1.0);
            if expired {
                character.tick_work(65., &mut core).unwrap();
            } else {
                character
                    .work_request(WorkRequest::End { token }, 15., &mut core)
                    .unwrap();
            }
            assert!(character.work.is_none());
            assert!(character.work_anchor);
            character.forget_gaze_for_external_stop(GazeStopCommand::Stop);
            character.clear_attention();
            assert!(character.resume_gaze_for_confirmed_wake(&core).unwrap());
            assert!(character.work_anchor);
            assert_eq!(character.status.active_anchor.as_ref(), Some(&anchor));
            // Both a held work anchor and Idle's released gaze hold keep the
            // same reach protected until a foreground pose replaces it.
            for (index, clip) in ["speak_react_happy", "idle_breathe"]
                .into_iter()
                .enumerate()
            {
                if index == 1 {
                    character.release_work_anchor();
                }
                let at = 70. + index as f64 * 10.;
                let definition = core.motions().motion(clip).unwrap().clone();
                let run = core
                    .play_generated_anchored_relative(definition, anchor.clone(), at)
                    .unwrap();
                for i in 1..=450 {
                    core.tick(at + i as f64 * 0.02).unwrap();
                    let pose = core
                        .snapshot()
                        .joints
                        .iter()
                        .map(|j| (j.name.clone(), j.position))
                        .collect();
                    assert!(comfort.contains(&pose), "{clip}: {pose:?}");
                }
                assert_eq!(terminal_phase(&core, run), Some(MovementPhase::Completed));
            }
        }
    }
}
