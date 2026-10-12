//! Semantic events are produced on the runtime thread; the writer never controls motion.
use crate::{MovementPhase, StateSnapshot};
use orion_history::{BehaviourEvent, BehaviourWriter};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
};

#[derive(Clone, Debug)]
pub(crate) struct BehaviourLog(Arc<Mutex<State>>);
#[derive(Debug)]
struct State {
    sender: mpsc::SyncSender<BehaviourEvent>,
    boot: String,
    sequence: u64,
    lost: u64,
    motions: BTreeMap<u64, (BehaviourEvent, f64)>,
    gaze: Option<(u64, f64, String)>,
    rest: super::rest::RestState,
    rest_since: f64,
    rest_action: Option<String>,
    initialized: bool,
    held_gaze: Option<(u64, f64, String)>,
    faces: usize,
}

impl BehaviourLog {
    pub(crate) fn new(writer: &BehaviourWriter) -> Self {
        Self(Arc::new(Mutex::new(State {
            sender: writer.sender(),
            boot: uuid_boot(),
            sequence: 0,
            lost: 0,
            motions: BTreeMap::new(),
            gaze: None,
            rest: super::rest::RestState::Disabled,
            rest_since: 0.,
            rest_action: None,
            initialized: false,
            held_gaze: None,
            faces: 0,
        })))
    }
    pub(crate) fn observation(
        &self,
        behaviour: &str,
        target: Option<String>,
        category: Option<String>,
        now: f64,
    ) {
        let mut state = self.0.lock().unwrap();
        let event = state.event("attention", behaviour, "observed", target, category, now);
        state.emit(event);
    }
    pub(crate) fn presence(&self, count: usize, track: Option<u64>, now: f64) {
        let mut state = self.0.lock().unwrap();
        if count > state.faces {
            let event = state.event(
                "attention",
                "person_presence",
                "observed",
                track.map(|id| format!("face:{id}")),
                Some("person".into()),
                now,
            );
            state.emit(event);
        }
        state.faces = count;
    }
    pub(crate) fn motion(
        &self,
        behaviour: &str,
        run: u64,
        target: Option<String>,
        category: Option<String>,
        now: f64,
    ) {
        let mut state = self.0.lock().unwrap();
        if state.motions.contains_key(&run) {
            return;
        }
        let mut event = state.event(
            if behaviour.starts_with("idle_") {
                "idle"
            } else {
                "attention"
            },
            behaviour,
            "started",
            target,
            category,
            now,
        );
        event.reason = Some(
            match behaviour {
                "object_glance" => "selected from the recent object map",
                "object_peek" | "pet_notice" => "object passed the novelty and persistence checks",
                "mutual_gaze" => "sustained frontal face",
                "arrival_notice" => "person present after prolonged observed absence",
                _ => "idle animation selection",
            }
            .into(),
        );
        event.action_id = Some(format!("{}-motion-{run}", state.boot));
        state.emit(event.clone());
        state.motions.insert(run, (event, now));
    }
    pub(crate) fn observe_motion(&self, snapshot: &StateSnapshot, now: f64) {
        let mut state = self.0.lock().unwrap();
        let finished: Vec<_> = state
            .motions
            .keys()
            .filter_map(|id| {
                let phase = snapshot
                    .motion
                    .as_ref()
                    .filter(|m| m.run_id == *id)
                    .map(|m| m.state)
                    .or_else(|| {
                        snapshot
                            .last_motion
                            .as_ref()
                            .filter(|m| m.run_id == *id)
                            .map(|m| m.state)
                    });
                match phase {
                    Some(phase) if !phase.is_terminal() => None,
                    Some(MovementPhase::Completed) => Some((*id, "completed")),
                    Some(MovementPhase::TimedOut) => Some((*id, "failed")),
                    _ => Some((*id, "interrupted")),
                }
            })
            .collect();
        for (id, phase) in finished {
            let (mut event, since) = state.motions.remove(&id).unwrap();
            state.stamp(&mut event, now);
            event.phase = phase.into();
            if phase != "completed" {
                event.reason = snapshot
                    .motion
                    .as_ref()
                    .filter(|m| m.run_id == id)
                    .or_else(|| snapshot.last_motion.as_ref().filter(|m| m.run_id == id))
                    .and_then(|m| m.error.clone())
                    .or_else(|| {
                        Some(
                            if phase == "failed" {
                                "motion timed out"
                            } else {
                                "motion was cancelled or replaced"
                            }
                            .into(),
                        )
                    });
            }
            event.duration_seconds = Some((now - since).max(0.));
            state.emit(event);
        }
    }
    pub(crate) fn observe_state(
        &self,
        gaze: &super::gaze::GazeStatus,
        rest: &super::rest::RestStatus,
        held: Option<u64>,
        now: f64,
    ) {
        let mut state = self.0.lock().unwrap();
        if state.held_gaze.as_ref().map(|g| g.0) != held {
            if let Some((id, since, action)) = state.held_gaze.take() {
                let mut event = state.event(
                    "attention",
                    "gaze_hold",
                    "ended",
                    Some(format!("face:{id}")),
                    Some("person".into()),
                    now,
                );
                event.action_id = Some(action);
                event.duration_seconds = Some((now - since).max(0.));
                state.emit(event);
            }
            if let Some(id) = held {
                let mut event = state.event(
                    "attention",
                    "gaze_hold",
                    "started",
                    Some(format!("face:{id}")),
                    Some("person".into()),
                    now,
                );
                event.action_id = Some(event.id.clone());
                state.held_gaze = Some((id, now, event.id.clone()));
                state.emit(event);
            }
        }
        let track = gaze
            .track_id
            .filter(|_| gaze.enabled && matches!(gaze.state, "tracking"));
        if state.gaze.as_ref().map(|g| g.0) != track {
            if let Some((id, since, action)) = state.gaze.take() {
                let mut event = state.event(
                    "attention",
                    "gaze",
                    "ended",
                    Some(format!("face:{id}")),
                    Some("person".into()),
                    now,
                );
                event.action_id = Some(action);
                event.duration_seconds = Some((now - since).max(0.));
                state.emit(event);
            }
            if let Some(id) = track {
                let mut event = state.event(
                    "attention",
                    "gaze",
                    "started",
                    Some(format!("face:{id}")),
                    Some("person".into()),
                    now,
                );
                event.action_id = Some(event.id.clone());
                state.gaze = Some((id, now, event.id.clone()));
                state.emit(event);
            }
        }
        if !state.initialized {
            state.initialized = true;
            if matches!(
                rest.state,
                super::rest::RestState::Awake | super::rest::RestState::Resting
            ) {
                let mut event = state.event("rest", "rest_state", "observed", None, None, now);
                event.reason = Some(format!("{:?}", rest.state));
                state.emit(event);
                state.rest = rest.state;
                state.rest_since = now;
            }
        }
        if state.rest != rest.state {
            use super::rest::RestState::*;
            let interrupted = match state.rest {
                GoingToRest if !matches!(rest.state, Resting | Fault) => Some("sleep"),
                Waking if !matches!(rest.state, Awake | Fault) => Some("wake"),
                _ => None,
            };
            if let Some(behaviour) = interrupted {
                let mut event = state.event("rest", behaviour, "interrupted", None, None, now);
                event.action_id = state.rest_action.take();
                event.duration_seconds = Some((now - state.rest_since).max(0.));
                event.reason = Some(format!("transition changed to {:?}", rest.state));
                state.emit(event);
            }
            let (behaviour, phase) = match rest.state {
                GoingToRest => ("sleep", "started"),
                Resting => ("sleep", "completed"),
                Waking => ("wake", "started"),
                Awake => ("wake", "completed"),
                Fault => (
                    match state.rest {
                        GoingToRest => "sleep",
                        Waking => "wake",
                        _ => "rest_transition",
                    },
                    "failed",
                ),
                Disabled => ("rest_policy", "disabled"),
            };
            let mut event = state.event("rest", behaviour, phase, None, None, now);
            if matches!(rest.state, GoingToRest | Waking) {
                state.rest_action = Some(event.id.clone());
            }
            event.action_id = state.rest_action.clone();
            if matches!(rest.state, Resting | Awake | Fault) && state.rest_action.is_some() {
                event.duration_seconds = Some((now - state.rest_since).max(0.));
            }
            event.session_id = rest.transition_session_id.clone();
            event.reason = rest
                .error
                .clone()
                .or_else(|| rest.transition_reason.clone());
            state.emit(event);
            state.rest = rest.state;
            state.rest_since = now;
            if matches!(rest.state, Resting | Awake | Fault | Disabled) {
                state.rest_action = None;
            }
        }
    }
}

fn uuid_boot() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
}
impl State {
    fn stamp(&mut self, event: &mut BehaviourEvent, now: f64) {
        self.sequence += 1;
        event.id = format!("{}-{:012}", self.boot, self.sequence);
        event.at = chrono::Utc::now().to_rfc3339();
        event.runtime_time_seconds = now;
    }
    fn event(
        &mut self,
        category: &str,
        behaviour: &str,
        phase: &str,
        target: Option<String>,
        target_category: Option<String>,
        now: f64,
    ) -> BehaviourEvent {
        let mut event = BehaviourEvent {
            id: String::new(),
            at: String::new(),
            runtime_time_seconds: now,
            category: category.into(),
            behaviour: behaviour.into(),
            phase: phase.into(),
            target,
            target_category,
            reason: None,
            action_id: None,
            duration_seconds: None,
            session_id: None,
        };
        self.stamp(&mut event, now);
        event
    }
    fn emit(&mut self, event: BehaviourEvent) {
        if self.lost > 0 {
            let mut gap = self.event(
                "system",
                "log_gap",
                "observed",
                None,
                None,
                event.runtime_time_seconds,
            );
            gap.reason = Some(format!("{} events could not be queued", self.lost));
            if self.sender.try_send(gap).is_ok() {
                self.lost = 0;
            }
        }
        if self.sender.try_send(event).is_err() {
            self.lost += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_sleep_and_wake_have_terminal_events() {
        use super::super::rest::RestState;
        let directory = tempfile::tempdir().unwrap();
        let writer = BehaviourWriter::start(directory.path().into()).unwrap();
        let log = BehaviourLog::new(&writer);
        let mut rest = super::super::rest::RestCoordinator::new(1800.).status(0.);
        let gaze = super::super::gaze::GazeCoordinator::default()
            .status()
            .clone();
        for (time, state) in [
            (0., RestState::Awake),
            (1., RestState::GoingToRest),
            (3., RestState::Waking),
            (4., RestState::Disabled),
        ] {
            rest.state = state;
            log.observe_state(&gaze, &rest, None, time);
        }
        drop(writer);
        let history = orion_history::Store::new(directory.path().into())
            .behaviours(None, None)
            .unwrap();
        let events = history["items"].as_array().unwrap();
        for (behaviour, duration) in [("sleep", 2.), ("wake", 1.)] {
            let started = events
                .iter()
                .find(|e| e["behaviour"] == behaviour && e["phase"] == "started")
                .unwrap();
            let ended = events
                .iter()
                .find(|e| e["behaviour"] == behaviour && e["phase"] == "interrupted")
                .unwrap();
            assert_eq!(started["actionId"], ended["actionId"]);
            assert_eq!(ended["durationSeconds"], duration);
        }
        assert!(!events.iter().any(|e| e["phase"] == "completed"));
    }
    fn motion(run: u64, phase: MovementPhase) -> crate::MotionState {
        crate::MotionState {
            run_id: run,
            name: "peek".into(),
            state: phase,
            keyframe: None,
            keyframe_index: None,
            keyframe_count: None,
            reached_markers: vec![],
            progress: 1.,
            max_position_error_rad: None,
            max_velocity_rad_s: None,
            completion_diagnostic: None,
            error: None,
        }
    }
    #[test]
    fn motion_outcomes_and_gaze_durations_are_transitions_not_tick_snapshots() {
        let directory = tempfile::tempdir().unwrap();
        let writer = BehaviourWriter::start(directory.path().into()).unwrap();
        let log = BehaviourLog::new(&writer);
        let mut snapshot = StateSnapshot::new(crate::RuntimeMode::Holding, 0, 50., vec![]).unwrap();
        log.motion(
            "object_peek",
            1,
            Some("book:1".into()),
            Some("book".into()),
            1.,
        );
        snapshot.motion = Some(motion(1, MovementPhase::Executing));
        log.observe_motion(&snapshot, 2.);
        snapshot.motion = None;
        snapshot.last_motion = Some(motion(1, MovementPhase::Cancelled));
        log.observe_motion(&snapshot, 3.);
        log.motion("idle_micro_glance", 2, None, None, 4.);
        snapshot.last_motion = Some(motion(2, MovementPhase::Completed));
        log.observe_motion(&snapshot, 6.);
        log.motion("object_peek", 3, None, Some("book".into()), 7.);
        snapshot.last_motion = Some(motion(3, MovementPhase::TimedOut));
        log.observe_motion(&snapshot, 8.);
        let rest = super::super::rest::RestCoordinator::new(1800.);
        let mut gaze = super::super::gaze::GazeCoordinator::default()
            .status()
            .clone();
        gaze.enabled = true;
        gaze.state = "tracking";
        gaze.track_id = Some(7);
        for time in [10., 11., 12.] {
            log.observe_state(&gaze, &rest.status(time), Some(7), time);
        }
        gaze.state = "idle";
        log.observe_state(&gaze, &rest.status(14.), None, 14.);
        drop(writer);
        let events = orion_history::Store::new(directory.path().into())
            .behaviours(None, None)
            .unwrap();
        let events = events["items"].as_array().unwrap();
        assert_eq!(events.len(), 10);
        assert!(events.iter().any(|e| e["behaviour"] == "object_peek"
            && e["phase"] == "interrupted"
            && e["durationSeconds"] == 2.));
        assert!(
            events
                .iter()
                .any(|e| e["behaviour"] == "object_peek" && e["phase"] == "failed")
        );
        assert!(events.iter().any(|e| e["behaviour"] == "gaze_hold"
            && e["phase"] == "ended"
            && e["durationSeconds"] == 4.));
        assert_eq!(
            events
                .iter()
                .filter(|e| e["behaviour"] == "gaze_hold")
                .count(),
            2
        );
    }
}
