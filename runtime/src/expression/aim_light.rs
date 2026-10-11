//! Ownership across lookup, authored scan stops, and measured aiming completion.
use super::*;
use serde::Deserialize;
use serde_json::{Value, json};

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
        if matches!(request, WorkRequest::End { .. }) && work.run.is_some() && !work.failed {
            checked(core.handle_command("stop", now))?;
            work.run = None;
            work.aimed = None;
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
                    json!({"ok":true,"status":solution.status,"aim_error_rad":solution.aim_error_rad,"run_id":run}),
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
                    self.work = None;
                    self.work_anchor = true;
                    self.status.state = self.idle_state();
                    self.reset_timers(now);
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
        if now >= work.deadline {
            if work.run.is_some_and(|run| {
                core.snapshot()
                    .motion
                    .as_ref()
                    .is_some_and(|m| m.run_id == run)
            }) {
                checked(core.handle_command("stop", now))?;
            }
            self.work = None;
            self.status.state = self.idle_state();
            self.reset_timers(now);
            return Ok(false);
        }
        if let Some(run) = work.run {
            if let Some(phase) = terminal_phase(core, run) {
                work.run = None;
                work.failed = phase != MovementPhase::Completed;
                if !work.failed {
                    if let Some(target) = work.aimed.take() {
                        self.status.active_anchor = Some(target);
                        work.committed = true;
                    }
                    if work.returning {
                        self.status.active_anchor = Some(work.previous.clone());
                        self.work_anchor = work.previous_work_anchor;
                        self.work = None;
                        self.status.state = self.idle_state();
                        self.reset_timers(now);
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }
}
