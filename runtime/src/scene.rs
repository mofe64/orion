use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::audio::AudioDevice;
use crate::daemon::RuntimeCore;
use crate::driver::RuntimeDriver;
use crate::lighting::{LightingDevice, Rgbw8};
use crate::motion::MotionLibrary;
use crate::pose::PoseLibrary;
use crate::state::MovementPhase;
use crate::{Error, Result};

pub const SCENE_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub enum SceneMotion {
    Play { motion: String },
    Goto { pose: String, duration_seconds: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub enum SceneAction {
    Motion(SceneMotion),
    Light {
        color: Rgbw8,
        transition_seconds: f64,
    },
    Audio {
        cue: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneEvent {
    pub at_seconds: f64,
    pub action: SceneAction,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneDefinition {
    pub name: String,
    pub description: String,
    pub events: Vec<SceneEvent>,
}

#[derive(Clone, Debug)]
pub struct SceneLibrary {
    scenes: BTreeMap<String, SceneDefinition>,
}

impl SceneLibrary {
    pub fn load(
        directory: impl AsRef<Path>,
        poses: &PoseLibrary,
        motions: &MotionLibrary,
    ) -> Result<Self> {
        let directory = directory.as_ref();
        if !directory.is_dir() {
            return Err(Error::Runtime(format!(
                "Scene library is not a directory: {}",
                directory.display()
            )));
        }
        let mut files = Vec::new();
        collect_yaml_files(directory, &mut files)?;
        files.sort();
        if files.is_empty() {
            return Err(Error::Runtime(format!(
                "Scene library contains no YAML files: {}",
                directory.display()
            )));
        }

        let mut scenes = BTreeMap::new();
        for path in files {
            let scene = load_scene_file(&path, poses, motions)?;
            if scenes.insert(scene.name.clone(), scene).is_some() {
                return Err(Error::Runtime(format!(
                    "Duplicate Orion scene name in {}",
                    path.display()
                )));
            }
        }
        Ok(Self { scenes })
    }

    pub fn scene(&self, name: &str) -> Result<&SceneDefinition> {
        self.scenes
            .get(name)
            .ok_or_else(|| Error::InvalidArgument(format!("Unknown Orion scene: {name}")))
    }

    pub fn names(&self) -> Vec<String> {
        self.scenes.keys().cloned().collect()
    }
}

fn collect_yaml_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory).map_err(|error| {
        Error::Runtime(format!(
            "Could not read scene library '{}': {error}",
            directory.display()
        ))
    })? {
        let path = entry?.path();
        if path.is_dir() {
            collect_yaml_files(&path, files)?;
        } else if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yaml" | "yml")
        ) {
            files.push(path);
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneDocument {
    #[serde(default)]
    format_version: u32,
    scene: Option<SceneEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneEntry {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    timeline: Vec<SceneEventDocument>,
}

#[derive(Debug, Deserialize)]
struct SceneEventDocument {
    at: f64,
    #[serde(flatten)]
    action: SceneActionDocument,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum SceneActionDocument {
    PlayMotion {
        motion: String,
    },
    GotoPose {
        pose: String,
        duration_seconds: f64,
    },
    Light {
        #[serde(default)]
        red: u8,
        #[serde(default)]
        green: u8,
        #[serde(default)]
        blue: u8,
        #[serde(default)]
        white: u8,
        #[serde(default)]
        transition_seconds: f64,
    },
    Audio {
        cue: String,
    },
}

fn load_scene_file(
    path: &Path,
    poses: &PoseLibrary,
    motions: &MotionLibrary,
) -> Result<SceneDefinition> {
    let contents = fs::read_to_string(path).map_err(|error| {
        Error::Runtime(format!(
            "Could not read scene file '{}': {error}",
            path.display()
        ))
    })?;
    let document: SceneDocument = serde_yaml::from_str(&contents).map_err(|error| {
        Error::Runtime(format!(
            "Could not parse scene file '{}': {error}",
            path.display()
        ))
    })?;
    if document.format_version != SCENE_FORMAT_VERSION {
        return Err(Error::Runtime(format!(
            "Scene file must use format_version {SCENE_FORMAT_VERSION}."
        )));
    }
    let Some(entry) = document.scene else {
        return Err(Error::Runtime(
            "Scene file must contain a scene mapping.".into(),
        ));
    };
    if entry.name.trim().is_empty() {
        return Err(Error::Runtime("Scene name cannot be empty.".into()));
    }
    if entry.timeline.is_empty() {
        return Err(Error::Runtime(format!(
            "Scene '{}' must contain timeline events.",
            entry.name
        )));
    }

    let mut events = Vec::with_capacity(entry.timeline.len());
    let mut previous_at = 0.0;
    for (index, event) in entry.timeline.into_iter().enumerate() {
        if !event.at.is_finite() || event.at < 0.0 || (index > 0 && event.at < previous_at) {
            return Err(Error::Runtime(format!(
                "Scene '{}' timeline times must be finite, non-negative, and ordered.",
                entry.name
            )));
        }
        previous_at = event.at;
        let action = match event.action {
            SceneActionDocument::PlayMotion { motion } => {
                if motion.trim().is_empty() {
                    return Err(Error::Runtime(format!(
                        "Scene '{}' contains an empty motion name.",
                        entry.name
                    )));
                }
                motions.motion(&motion).map_err(|error| {
                    Error::Runtime(format!(
                        "Invalid motion reference in scene file '{}': {error}",
                        path.display()
                    ))
                })?;
                SceneAction::Motion(SceneMotion::Play { motion })
            }
            SceneActionDocument::GotoPose {
                pose,
                duration_seconds,
            } => {
                if pose.trim().is_empty()
                    || !duration_seconds.is_finite()
                    || duration_seconds <= 0.0
                {
                    return Err(Error::Runtime(format!(
                        "Scene '{}' goto_pose requires a pose and positive finite duration.",
                        entry.name
                    )));
                }
                poses.pose(&pose).map_err(|error| {
                    Error::Runtime(format!(
                        "Invalid pose reference in scene file '{}': {error}",
                        path.display()
                    ))
                })?;
                SceneAction::Motion(SceneMotion::Goto {
                    pose,
                    duration_seconds,
                })
            }
            SceneActionDocument::Light {
                red,
                green,
                blue,
                white,
                transition_seconds,
            } => {
                if !transition_seconds.is_finite() || transition_seconds < 0.0 {
                    return Err(Error::Runtime(format!(
                        "Scene '{}' light transition must be finite and non-negative.",
                        entry.name
                    )));
                }
                SceneAction::Light {
                    color: Rgbw8::new(red, green, blue, white),
                    transition_seconds,
                }
            }
            SceneActionDocument::Audio { cue } => {
                if cue.trim().is_empty() {
                    return Err(Error::Runtime(format!(
                        "Scene '{}' contains an empty audio cue.",
                        entry.name
                    )));
                }
                SceneAction::Audio { cue }
            }
        };
        events.push(SceneEvent {
            at_seconds: event.at,
            action,
        });
    }

    Ok(SceneDefinition {
        name: entry.name,
        description: entry.description,
        events,
    })
}

pub trait SceneMotionDevice {
    fn start(&mut self, motion: &SceneMotion, now_seconds: f64) -> Result<u64>;
    fn phase(&self, run_id: u64) -> Option<MovementPhase>;
    fn cancel(&mut self, now_seconds: f64) -> Result<()>;
}

impl<D: RuntimeDriver> SceneMotionDevice for RuntimeCore<D> {
    fn start(&mut self, motion: &SceneMotion, now_seconds: f64) -> Result<u64> {
        let command = match motion {
            SceneMotion::Play { motion } => format!("play {motion}"),
            SceneMotion::Goto {
                pose,
                duration_seconds,
            } => format!("goto {pose} {duration_seconds:.6}"),
        };
        let response = self.handle_command(&command, now_seconds);
        let value: serde_json::Value = serde_json::from_str(&response)?;
        if value.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
            let reason = value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("movement was rejected");
            return Err(Error::InvalidState(format!(
                "Scene movement '{command}' was rejected: {reason}"
            )));
        }
        value
            .get("run_id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                Error::Runtime("Accepted scene movement did not include a run ID.".into())
            })
    }

    fn phase(&self, run_id: u64) -> Option<MovementPhase> {
        [
            self.snapshot().motion.as_ref(),
            self.snapshot().last_motion.as_ref(),
        ]
        .into_iter()
        .flatten()
        .find(|movement| movement.run_id == run_id)
        .map(|movement| movement.state)
    }

    fn cancel(&mut self, now_seconds: f64) -> Result<()> {
        let response = self.handle_command("stop", now_seconds);
        let value: serde_json::Value = serde_json::from_str(&response)?;
        if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
            Ok(())
        } else {
            Err(Error::InvalidState(
                value
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("scene movement cancellation was rejected")
                    .to_owned(),
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenePhase {
    Executing,
    Completed,
    TimedOut,
    Cancelled,
}

impl ScenePhase {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Executing)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SceneStatus {
    pub run_id: u64,
    pub name: String,
    pub state: ScenePhase,
    pub dispatched_events: usize,
    pub event_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_motion_run_id: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
struct LightTransition {
    start: Rgbw8,
    target: Rgbw8,
    starts_at: f64,
    duration_seconds: f64,
}

#[derive(Debug)]
pub struct ScenePlayer {
    definition: SceneDefinition,
    status: SceneStatus,
    started_at: f64,
    last_tick_at: f64,
    dispatched: Vec<bool>,
    light: Rgbw8,
    transition: Option<LightTransition>,
    last_rendered: Option<Rgbw8>,
}

impl ScenePlayer {
    pub fn new(
        run_id: u64,
        definition: SceneDefinition,
        started_at: f64,
        initial_light: Rgbw8,
    ) -> Result<Self> {
        if run_id == 0 || !started_at.is_finite() || definition.events.is_empty() {
            return Err(Error::InvalidArgument(
                "Scene playback requires a non-zero run ID, finite start time, and events.".into(),
            ));
        }
        let event_count = definition.events.len();
        Ok(Self {
            status: SceneStatus {
                run_id,
                name: definition.name.clone(),
                state: ScenePhase::Executing,
                dispatched_events: 0,
                event_count,
                active_motion_run_id: None,
            },
            definition,
            started_at,
            last_tick_at: started_at,
            dispatched: vec![false; event_count],
            light: initial_light,
            transition: None,
            last_rendered: None,
        })
    }

    pub fn tick<M: SceneMotionDevice, L: LightingDevice, A: AudioDevice>(
        &mut self,
        now_seconds: f64,
        motion: &mut M,
        lighting: &mut L,
        audio: &mut A,
    ) -> Result<()> {
        if !now_seconds.is_finite() || now_seconds < self.last_tick_at {
            return Err(Error::InvalidArgument(
                "Scene time must be finite and monotonic.".into(),
            ));
        }
        self.last_tick_at = now_seconds;
        if self.status.state.is_terminal() {
            return Ok(());
        }

        self.update_motion_phase(motion)?;
        if self.status.state.is_terminal() {
            return Ok(());
        }

        let elapsed = now_seconds - self.started_at;
        for index in 0..self.definition.events.len() {
            if self.dispatched[index] || self.definition.events[index].at_seconds > elapsed {
                continue;
            }
            let event = self.definition.events[index].clone();
            match event.action {
                SceneAction::Motion(scene_motion) => {
                    if self.status.active_motion_run_id.is_some() {
                        continue;
                    }
                    let run_id = motion.start(&scene_motion, now_seconds)?;
                    self.status.active_motion_run_id = Some(run_id);
                }
                SceneAction::Light {
                    color,
                    transition_seconds,
                } => {
                    self.advance_light(event.at_seconds)?;
                    if transition_seconds == 0.0 {
                        self.light = color;
                        self.transition = None;
                    } else {
                        self.transition = Some(LightTransition {
                            start: self.light,
                            target: color,
                            starts_at: event.at_seconds,
                            duration_seconds: transition_seconds,
                        });
                    }
                }
                SceneAction::Audio { cue } => audio.play(&cue)?,
            }
            self.dispatched[index] = true;
            self.status.dispatched_events += 1;
        }

        self.advance_light(elapsed)?;
        if self.last_rendered != Some(self.light) {
            lighting.render_uniform(self.light)?;
            self.last_rendered = Some(self.light);
        }

        self.update_motion_phase(motion)?;
        if self.status.dispatched_events == self.status.event_count
            && self.status.active_motion_run_id.is_none()
            && self.transition.is_none()
        {
            self.status.state = ScenePhase::Completed;
        }
        Ok(())
    }

    pub fn cancel<M: SceneMotionDevice, A: AudioDevice>(
        &mut self,
        now_seconds: f64,
        motion: &mut M,
        audio: &mut A,
    ) -> Result<()> {
        if !now_seconds.is_finite() {
            return Err(Error::InvalidArgument(
                "Scene cancellation time must be finite.".into(),
            ));
        }
        if self.status.state.is_terminal() {
            return Ok(());
        }
        if self.status.active_motion_run_id.is_some() {
            motion.cancel(now_seconds)?;
        }
        audio.stop()?;
        self.transition = None;
        self.status.active_motion_run_id = None;
        self.status.state = ScenePhase::Cancelled;
        Ok(())
    }

    pub fn status(&self) -> &SceneStatus {
        &self.status
    }

    pub fn light(&self) -> Rgbw8 {
        self.light
    }

    fn update_motion_phase<M: SceneMotionDevice>(&mut self, motion: &M) -> Result<()> {
        let Some(run_id) = self.status.active_motion_run_id else {
            return Ok(());
        };
        let phase = motion.phase(run_id).ok_or_else(|| {
            Error::Runtime(format!(
                "Scene '{}' lost movement run {run_id}.",
                self.definition.name
            ))
        })?;
        match phase {
            MovementPhase::Executing | MovementPhase::Settling => {}
            MovementPhase::Completed => self.status.active_motion_run_id = None,
            MovementPhase::TimedOut => {
                self.status.active_motion_run_id = None;
                self.status.state = ScenePhase::TimedOut;
            }
            MovementPhase::Cancelled => {
                self.status.active_motion_run_id = None;
                self.status.state = ScenePhase::Cancelled;
            }
        }
        Ok(())
    }

    fn advance_light(&mut self, elapsed_seconds: f64) -> Result<()> {
        let Some(transition) = self.transition else {
            return Ok(());
        };
        let progress = ((elapsed_seconds - transition.starts_at) / transition.duration_seconds)
            .clamp(0.0, 1.0);
        self.light = transition.start.interpolate(transition.target, progress)?;
        if progress >= 1.0 {
            self.light = transition.target;
            self.transition = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::tempdir;

    use super::*;
    use crate::audio::{AudioCommand, RecordingAudioDevice};
    use crate::daemon::CompletionCriteria;
    use crate::driver::RuntimeDriver;
    use crate::lighting::RecordingLightingDevice;
    use crate::pose::JointPositions;
    use crate::state::JointState;

    struct FakeMotionDevice {
        next_run_id: u64,
        phases: BTreeMap<u64, MovementPhase>,
        started: Vec<SceneMotion>,
        cancel_count: usize,
    }

    impl FakeMotionDevice {
        fn new() -> Self {
            Self {
                next_run_id: 1,
                phases: BTreeMap::new(),
                started: Vec::new(),
                cancel_count: 0,
            }
        }

        fn finish(&mut self, run_id: u64, phase: MovementPhase) {
            self.phases.insert(run_id, phase);
        }
    }

    impl SceneMotionDevice for FakeMotionDevice {
        fn start(&mut self, motion: &SceneMotion, _now_seconds: f64) -> Result<u64> {
            let run_id = self.next_run_id;
            self.next_run_id += 1;
            self.started.push(motion.clone());
            self.phases.insert(run_id, MovementPhase::Executing);
            Ok(run_id)
        }

        fn phase(&self, run_id: u64) -> Option<MovementPhase> {
            self.phases.get(&run_id).copied()
        }

        fn cancel(&mut self, _now_seconds: f64) -> Result<()> {
            self.cancel_count += 1;
            Ok(())
        }
    }

    struct FollowingRuntimeDriver {
        positions: JointPositions,
        configured: bool,
        active: bool,
    }

    impl FollowingRuntimeDriver {
        fn new() -> Self {
            Self {
                positions: crate::ORION_JOINT_NAMES
                    .iter()
                    .map(|name| ((*name).to_owned(), 0.0))
                    .collect(),
                configured: false,
                active: false,
            }
        }

        fn states(&self) -> Vec<JointState> {
            self.positions
                .iter()
                .map(|(name, position)| JointState {
                    name: name.clone(),
                    position: *position,
                    velocity: 0.0,
                    current_ma: 0.0,
                    voltage_v: 5.0,
                    temperature_c: 25.0,
                    status: 0,
                })
                .collect()
        }
    }

    impl RuntimeDriver for FollowingRuntimeDriver {
        fn apply_servo_profile(&mut self) -> Result<()> {
            self.configured = true;
            Ok(())
        }

        fn activate(&mut self) -> Result<Vec<JointState>> {
            if !self.configured {
                return Err(Error::InvalidState("driver is not configured".into()));
            }
            self.active = true;
            Ok(self.states())
        }

        fn deactivate(&mut self) -> Result<()> {
            self.active = false;
            Ok(())
        }

        fn read(&mut self) -> Result<Vec<JointState>> {
            Ok(self.states())
        }

        fn write(&mut self, positions_radians: &JointPositions) -> Result<()> {
            if !self.active {
                return Err(Error::InvalidState("driver is inactive".into()));
            }
            self.positions = positions_radians.clone();
            Ok(())
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

    fn example_scene() -> SceneDefinition {
        SceneDefinition {
            name: "acknowledge".into(),
            description: "Move, fade warm white, and play a cue.".into(),
            events: vec![
                SceneEvent {
                    at_seconds: 0.0,
                    action: SceneAction::Motion(SceneMotion::Play {
                        motion: "look_at_left".into(),
                    }),
                },
                SceneEvent {
                    at_seconds: 0.0,
                    action: SceneAction::Light {
                        color: Rgbw8::new(0, 0, 0, 100),
                        transition_seconds: 1.0,
                    },
                },
                SceneEvent {
                    at_seconds: 0.5,
                    action: SceneAction::Audio {
                        cue: "acknowledge".into(),
                    },
                },
            ],
        }
    }

    #[test]
    fn coordinates_motion_light_and_audio_with_one_clock() {
        let mut player = ScenePlayer::new(9, example_scene(), 10.0, Rgbw8::OFF).unwrap();
        let mut motion = FakeMotionDevice::new();
        let mut lighting = RecordingLightingDevice::new(2).unwrap();
        let mut audio = RecordingAudioDevice::default();

        player
            .tick(10.0, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        assert_eq!(motion.started.len(), 1);
        assert_eq!(player.light(), Rgbw8::OFF);
        assert_eq!(player.status().state, ScenePhase::Executing);

        player
            .tick(10.5, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        assert_eq!(player.light(), Rgbw8::new(0, 0, 0, 50));
        assert_eq!(
            audio.commands(),
            &[AudioCommand::Play("acknowledge".into())]
        );

        motion.finish(1, MovementPhase::Completed);
        player
            .tick(11.0, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        assert_eq!(player.light(), Rgbw8::new(0, 0, 0, 100));
        assert_eq!(player.status().state, ScenePhase::Completed);
        assert_eq!(player.status().dispatched_events, 3);
    }

    #[test]
    fn propagates_motion_timeout_and_cancels_devices() {
        let mut player = ScenePlayer::new(1, example_scene(), 0.0, Rgbw8::OFF).unwrap();
        let mut motion = FakeMotionDevice::new();
        let mut lighting = RecordingLightingDevice::new(1).unwrap();
        let mut audio = RecordingAudioDevice::default();
        player
            .tick(0.0, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        motion.finish(1, MovementPhase::TimedOut);
        player
            .tick(0.1, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        assert_eq!(player.status().state, ScenePhase::TimedOut);

        let mut second = ScenePlayer::new(2, example_scene(), 1.0, Rgbw8::OFF).unwrap();
        second
            .tick(1.0, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        second.cancel(1.1, &mut motion, &mut audio).unwrap();
        assert_eq!(second.status().state, ScenePhase::Cancelled);
        assert_eq!(motion.cancel_count, 1);
        assert_eq!(audio.commands().last(), Some(&AudioCommand::Stop));
    }

    #[test]
    fn loads_and_validates_scene_references() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let poses = PoseLibrary::load(
            root.join("motion/config/poses.yaml"),
            &crate::ORION_JOINT_NAMES,
        )
        .unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("acknowledge.yaml"),
            r#"format_version: 1
scene:
  name: acknowledge
  description: Test scene.
  timeline:
    - at: 0.0
      type: play_motion
      motion: look_at_left
    - at: 0.0
      type: light
      white: 40
      transition_seconds: 0.25
    - at: 0.2
      type: audio
      cue: acknowledge
    - at: 1.0
      type: goto_pose
      pose: home
      duration_seconds: 2.0
"#,
        )
        .unwrap();

        let scenes = SceneLibrary::load(directory.path(), &poses, &motions).unwrap();
        let scene = scenes.scene("acknowledge").unwrap();
        assert_eq!(scene.events.len(), 4);
        assert!(matches!(
            scene.events[0].action,
            SceneAction::Motion(SceneMotion::Play { .. })
        ));
    }

    #[test]
    fn rejects_unknown_motion_and_unordered_events() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let poses = PoseLibrary::load(
            root.join("motion/config/poses.yaml"),
            &crate::ORION_JOINT_NAMES,
        )
        .unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("bad.yaml"),
            r#"format_version: 1
scene:
  name: bad
  timeline:
    - at: 1.0
      type: play_motion
      motion: missing
    - at: 0.0
      type: light
      white: 1
"#,
        )
        .unwrap();
        assert!(SceneLibrary::load(directory.path(), &poses, &motions).is_err());
    }

    #[test]
    fn loads_the_tracked_orion_scene_library() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let poses = PoseLibrary::load(
            root.join("motion/config/poses.yaml"),
            &crate::ORION_JOINT_NAMES,
        )
        .unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        let scenes = SceneLibrary::load(root.join("scenes"), &poses, &motions).unwrap();

        let scene = scenes.scene("acknowledge_left").unwrap();
        assert_eq!(scene.events.len(), 4);
        assert_eq!(scene.events[0].at_seconds, 0.0);
        assert_eq!(scene.events[3].at_seconds, 4.2);
    }

    #[test]
    fn waits_for_the_real_runtime_movement_lifecycle() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let poses = PoseLibrary::load(
            root.join("motion/config/poses.yaml"),
            &crate::ORION_JOINT_NAMES,
        )
        .unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        let mut core = RuntimeCore::with_completion_criteria(
            FollowingRuntimeDriver::new(),
            poses,
            motions,
            CompletionCriteria {
                position_tolerance_rad: 0.05,
                velocity_tolerance_rad_s: 0.05,
                settle_duration_seconds: 0.1,
                settle_timeout_seconds: 1.0,
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&core.handle_command("configure", 0.0))
                .unwrap()["ok"],
            true
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&core.handle_command("enable", 0.0)).unwrap()
                ["ok"],
            true
        );

        let scene = SceneDefinition {
            name: "runtime_scene".into(),
            description: String::new(),
            events: vec![SceneEvent {
                at_seconds: 0.0,
                action: SceneAction::Motion(SceneMotion::Play {
                    motion: "look_at_left".into(),
                }),
            }],
        };
        let mut player = ScenePlayer::new(1, scene, 0.0, Rgbw8::OFF).unwrap();
        let mut lighting = RecordingLightingDevice::new(1).unwrap();
        let mut audio = RecordingAudioDevice::default();
        player
            .tick(0.0, &mut core, &mut lighting, &mut audio)
            .unwrap();
        assert_eq!(player.status().state, ScenePhase::Executing);
        assert_eq!(core.snapshot().motion.as_ref().unwrap().run_id, 1);

        for now in [0.5, 1.0, 1.5, 2.0, 2.1, 2.2] {
            core.tick(now).unwrap();
            player
                .tick(now, &mut core, &mut lighting, &mut audio)
                .unwrap();
        }

        assert_eq!(
            core.snapshot().last_motion.as_ref().unwrap().state,
            MovementPhase::Completed
        );
        assert_eq!(player.status().state, ScenePhase::Completed);
    }
}
