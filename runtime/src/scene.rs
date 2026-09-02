use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::audio::{AudioDevice, CueLibrary};
use crate::daemon::RuntimeCore;
use crate::driver::RuntimeDriver;
use crate::lighting::{LIGHTING_EFFECT_NAMES, LightingDevice, Rgbw8, render_effect};
use crate::motion::MotionLibrary;
use crate::pose::{JointPositions, PoseLibrary};
use crate::state::MovementPhase;
use crate::{Error, Result};

pub const SCENE_FORMAT_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq)]
pub enum SceneMotion {
    Play { motion: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum SceneAction {
    Motion(SceneMotion),
    Light {
        color: Rgbw8,
        transition_seconds: f64,
    },
    Effect {
        effect: String,
        intensity: f64,
        duration_seconds: f64,
        transition_seconds: f64,
    },
    Audio {
        cue: String,
    },
    OnMarker {
        marker: String,
        action: Box<SceneAction>,
    },
    Finish {
        anchor: String,
        lighting: String,
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

    pub fn validate_audio_cues(&self, cues: &CueLibrary) -> Result<()> {
        for scene in self.scenes.values() {
            scene.validate_audio_cues(cues)?;
        }
        Ok(())
    }
}

impl SceneDefinition {
    pub fn validate_audio_cues(&self, cues: &CueLibrary) -> Result<()> {
        for event in &self.events {
            let action = match &event.action {
                SceneAction::OnMarker { action, .. } => action.as_ref(),
                action => action,
            };
            if let SceneAction::Audio { cue } = action {
                if !cues.contains(cue) {
                    return Err(Error::Runtime(format!(
                        "Scene '{}' references unknown Orion audio cue '{}'.",
                        self.name, cue
                    )));
                }
            }
        }
        Ok(())
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
    motion: Vec<MotionTrackDocument>,
    #[serde(default)]
    lighting: Vec<LightingTrackDocument>,
    #[serde(default)]
    audio: Vec<AudioTrackDocument>,
    finish: Option<FinishDocument>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MotionTrackDocument {
    at: f64,
    play: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LightingTrackDocument {
    #[serde(default)]
    at: Option<f64>,
    #[serde(default)]
    on_marker: Option<String>,
    effect: String,
    #[serde(default = "default_intensity")]
    intensity: f64,
    #[serde(default = "default_effect_duration")]
    duration: f64,
    #[serde(default)]
    transition: f64,
    #[serde(default)]
    palette: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioTrackDocument {
    #[serde(default)]
    at: Option<f64>,
    #[serde(default)]
    on_marker: Option<String>,
    cue: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FinishDocument {
    anchor: String,
    lighting: String,
}

fn default_intensity() -> f64 {
    1.0
}
fn default_effect_duration() -> f64 {
    0.8
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
    parse_scene_document(&contents, &path.display().to_string(), poses, motions)
}

pub fn parse_scene_document(
    contents: &str,
    source: &str,
    _poses: &PoseLibrary,
    motions: &MotionLibrary,
) -> Result<SceneDefinition> {
    let header: serde_yaml::Value = serde_yaml::from_str(contents).map_err(|error| {
        Error::Runtime(format!("Could not parse scene file '{}': {error}", source))
    })?;
    if header
        .get("format_version")
        .and_then(serde_yaml::Value::as_u64)
        != Some(SCENE_FORMAT_VERSION as u64)
    {
        return Err(Error::Runtime(
            "Scene file must use format_version 2 (v2 required).".into(),
        ));
    }
    let document: SceneDocument = serde_yaml::from_str(contents).map_err(|error| {
        Error::Runtime(format!("Could not parse scene file '{}': {error}", source))
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
    if entry.motion.is_empty() && entry.lighting.is_empty() && entry.audio.is_empty() {
        return Err(Error::Runtime(format!(
            "Scene '{}' must contain at least one parallel-track event.",
            entry.name
        )));
    }
    let mut events = Vec::new();
    let mut known_markers = BTreeMap::<String, bool>::new();
    let mut previous_motion_end = 0.0;
    for motion_event in entry.motion {
        if !valid_at(motion_event.at) || motion_event.at < previous_motion_end {
            return Err(Error::Runtime(format!(
                "Scene '{}' motion clips may not overlap and must use finite non-negative times.",
                entry.name
            )));
        }
        let definition = motions.motion(&motion_event.play).map_err(|error| {
            Error::Runtime(format!(
                "Invalid motion reference in scene file '{}': {error}",
                source
            ))
        })?;
        let nominal_duration: f64 = definition
            .keyframes
            .iter()
            .map(|keyframe| {
                keyframe.duration_seconds / definition.style.tempo + keyframe.hold_seconds
            })
            .sum();
        previous_motion_end = motion_event.at + nominal_duration;
        for marker in definition.markers() {
            known_markers.insert(marker, true);
        }
        events.push(SceneEvent {
            at_seconds: motion_event.at,
            action: SceneAction::Motion(SceneMotion::Play {
                motion: motion_event.play,
            }),
        });
    }
    for light in entry.lighting {
        if !LIGHTING_EFFECT_NAMES.contains(&light.effect.as_str())
            || !light.intensity.is_finite()
            || !(0.0..=1.0).contains(&light.intensity)
            || !light.duration.is_finite()
            || light.duration < 0.0
            || !light.transition.is_finite()
            || light.transition < 0.0
        {
            return Err(Error::Runtime(format!(
                "Scene '{}' contains an invalid lighting effect.",
                entry.name
            )));
        }
        if light
            .palette
            .as_deref()
            .is_some_and(|palette| palette != "warm")
        {
            return Err(Error::Runtime(format!(
                "Scene '{}' supports only the warm Orion palette.",
                entry.name
            )));
        }
        let action = SceneAction::Effect {
            effect: light.effect,
            intensity: light.intensity,
            duration_seconds: light.duration,
            transition_seconds: light.transition,
        };
        let (at_seconds, action) = triggered_action(
            light.at,
            light.on_marker,
            action,
            &known_markers,
            &entry.name,
        )?;
        events.push(SceneEvent { at_seconds, action });
    }
    for audio in entry.audio {
        if audio.cue.trim().is_empty() {
            return Err(Error::Runtime(format!(
                "Scene '{}' contains an empty audio cue.",
                entry.name
            )));
        }
        let action = SceneAction::Audio { cue: audio.cue };
        let (at_seconds, action) = triggered_action(
            audio.at,
            audio.on_marker,
            action,
            &known_markers,
            &entry.name,
        )?;
        events.push(SceneEvent { at_seconds, action });
    }
    let finish = entry
        .finish
        .ok_or_else(|| Error::Runtime(format!("Scene '{}' requires finish policy.", entry.name)))?;
    if finish.anchor != "final_pose" || finish.lighting != "pose_default" {
        return Err(Error::Runtime(format!(
            "Scene '{}' finish must use final_pose and pose_default.",
            entry.name
        )));
    }
    events.push(SceneEvent {
        at_seconds: 0.0,
        action: SceneAction::Finish {
            anchor: finish.anchor,
            lighting: finish.lighting,
        },
    });
    events.sort_by(|left, right| left.at_seconds.total_cmp(&right.at_seconds));

    Ok(SceneDefinition {
        name: entry.name,
        description: entry.description,
        events,
    })
}

fn valid_at(at: f64) -> bool {
    at.is_finite() && at >= 0.0
}

fn triggered_action(
    at: Option<f64>,
    marker: Option<String>,
    action: SceneAction,
    known_markers: &BTreeMap<String, bool>,
    scene_name: &str,
) -> Result<(f64, SceneAction)> {
    match (at, marker) {
        (Some(at), None) if valid_at(at) => Ok((at, action)),
        (None, Some(marker)) if known_markers.contains_key(&marker) => Ok((
            0.0,
            SceneAction::OnMarker {
                marker,
                action: Box::new(action),
            },
        )),
        (None, Some(marker)) => Err(Error::Runtime(format!(
            "Scene '{scene_name}' references unknown motion marker '{marker}'."
        ))),
        _ => Err(Error::Runtime(format!(
            "Scene '{scene_name}' events require exactly one of at or on_marker."
        ))),
    }
}

pub trait SceneMotionDevice {
    fn start(&mut self, motion: &SceneMotion, now_seconds: f64) -> Result<u64>;
    fn phase(&self, run_id: u64) -> Option<MovementPhase>;
    fn marker_reached(&self, _run_id: u64, _marker: &str) -> bool {
        false
    }
    fn default_lighting_effect(&self) -> Option<String> {
        None
    }
    fn cancel(&mut self, now_seconds: f64) -> Result<()>;
}

impl<D: RuntimeDriver> SceneMotionDevice for RuntimeCore<D> {
    fn start(&mut self, motion: &SceneMotion, now_seconds: f64) -> Result<u64> {
        let command = match motion {
            SceneMotion::Play { motion } => format!("play {motion}"),
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

    fn marker_reached(&self, run_id: u64, marker: &str) -> bool {
        [
            self.snapshot().motion.as_ref(),
            self.snapshot().last_motion.as_ref(),
        ]
        .into_iter()
        .flatten()
        .find(|movement| movement.run_id == run_id)
        .is_some_and(|movement| {
            movement
                .reached_markers
                .iter()
                .any(|reached| reached == marker)
        })
    }

    fn default_lighting_effect(&self) -> Option<String> {
        let positions: JointPositions = self
            .snapshot()
            .joints
            .iter()
            .map(|joint| (joint.name.clone(), joint.position))
            .collect();
        self.poses()
            .names()
            .into_iter()
            .filter_map(|name| {
                let pose = self.poses().definition(&name).ok()?;
                let distance = pose
                    .positions
                    .iter()
                    .map(|(joint, target)| (positions[joint] - target).powi(2))
                    .sum::<f64>();
                Some((distance, pose.default_lighting.clone()))
            })
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .and_then(|(_, effect)| effect)
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
    Failed,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_audio_cue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug)]
struct LightTransition {
    start: Rgbw8,
    target: Rgbw8,
    starts_at: f64,
    duration_seconds: f64,
}

#[derive(Clone, Debug)]
struct ActiveEffect {
    name: String,
    starts_at: f64,
    duration_seconds: f64,
    intensity: f64,
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
    effect: Option<ActiveEffect>,
    last_motion_run_id: Option<u64>,
    finish_seen: bool,
    has_finish_policy: bool,
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
        let has_finish_policy = definition
            .events
            .iter()
            .any(|event| matches!(event.action, SceneAction::Finish { .. }));
        let finish_seen = !has_finish_policy;
        Ok(Self {
            status: SceneStatus {
                run_id,
                name: definition.name.clone(),
                state: ScenePhase::Executing,
                dispatched_events: 0,
                event_count,
                active_motion_run_id: None,
                active_audio_cue: None,
                error: None,
            },
            definition,
            started_at,
            last_tick_at: started_at,
            dispatched: vec![false; event_count],
            light: initial_light,
            transition: None,
            last_rendered: None,
            effect: None,
            last_motion_run_id: None,
            finish_seen,
            has_finish_policy,
        })
    }

    pub fn tick<M: SceneMotionDevice, L: LightingDevice + ?Sized, A: AudioDevice + ?Sized>(
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

        self.update_audio_phase(audio)?;
        self.update_motion_phase(motion)?;
        if self.status.state.is_terminal() {
            let _ = audio.stop();
            self.status.active_audio_cue = None;
            return Ok(());
        }

        let elapsed = now_seconds - self.started_at;
        for index in 0..self.definition.events.len() {
            if self.dispatched[index] || self.definition.events[index].at_seconds > elapsed {
                continue;
            }
            let event = self.definition.events[index].clone();
            let action = match event.action {
                SceneAction::OnMarker { marker, action } => {
                    let Some(run_id) = self.last_motion_run_id else {
                        continue;
                    };
                    if !motion.marker_reached(run_id, &marker) {
                        continue;
                    }
                    *action
                }
                action => action,
            };
            match action {
                SceneAction::Motion(scene_motion) => {
                    if self.status.active_motion_run_id.is_some() {
                        continue;
                    }
                    let run_id = motion.start(&scene_motion, now_seconds)?;
                    self.status.active_motion_run_id = Some(run_id);
                    self.last_motion_run_id = Some(run_id);
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
                SceneAction::Audio { cue } => {
                    if self.status.active_audio_cue.is_some() {
                        continue;
                    }
                    audio.play(&cue)?;
                    self.status.active_audio_cue = Some(cue);
                }
                SceneAction::Effect {
                    effect,
                    intensity,
                    duration_seconds,
                    transition_seconds,
                } => {
                    self.effect = Some(ActiveEffect {
                        name: effect,
                        starts_at: elapsed,
                        duration_seconds: duration_seconds + transition_seconds,
                        intensity,
                    });
                }
                SceneAction::Finish { .. } => self.finish_seen = true,
                SceneAction::OnMarker { .. } => {
                    unreachable!("marker wrapper is removed before dispatch")
                }
            }
            self.dispatched[index] = true;
            self.status.dispatched_events += 1;
        }

        self.advance_light(elapsed)?;
        if let Some(effect) = &self.effect {
            let effect_elapsed = elapsed - effect.starts_at;
            lighting.render(&render_effect(
                &effect.name,
                effect_elapsed,
                effect.intensity,
            )?)?;
            if effect_elapsed >= effect.duration_seconds {
                self.effect = None;
            }
        } else if self.last_rendered != Some(self.light) {
            lighting.render_uniform(self.light)?;
            self.last_rendered = Some(self.light);
        }

        self.update_audio_phase(audio)?;
        self.update_motion_phase(motion)?;
        if self.status.state.is_terminal() {
            let _ = audio.stop();
            self.status.active_audio_cue = None;
            return Ok(());
        }
        if self.status.dispatched_events == self.status.event_count
            && self.status.active_motion_run_id.is_none()
            && self.status.active_audio_cue.is_none()
            && self.transition.is_none()
            && self.effect.is_none()
            && self.finish_seen
        {
            if self.has_finish_policy {
                let effect = motion
                    .default_lighting_effect()
                    .unwrap_or_else(|| "settle_glow".to_owned());
                lighting.render(&render_effect(&effect, 0.0, 0.55)?)?;
            }
            self.status.state = ScenePhase::Completed;
        }
        Ok(())
    }

    pub fn cancel<M: SceneMotionDevice, A: AudioDevice + ?Sized>(
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
        self.effect = None;
        self.status.active_motion_run_id = None;
        self.status.active_audio_cue = None;
        self.status.state = ScenePhase::Cancelled;
        Ok(())
    }

    pub fn status(&self) -> &SceneStatus {
        &self.status
    }

    pub fn light(&self) -> Rgbw8 {
        self.light
    }

    fn fail<M: SceneMotionDevice, A: AudioDevice + ?Sized>(
        &mut self,
        now_seconds: f64,
        motion: &mut M,
        audio: &mut A,
        error: String,
    ) {
        if self.status.active_motion_run_id.is_some() {
            let _ = motion.cancel(now_seconds);
        }
        let _ = audio.stop();
        self.transition = None;
        self.effect = None;
        self.status.active_motion_run_id = None;
        self.status.active_audio_cue = None;
        self.status.state = ScenePhase::Failed;
        self.status.error = Some(error);
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

    fn update_audio_phase<A: AudioDevice + ?Sized>(&mut self, audio: &mut A) -> Result<()> {
        audio.update()?;
        if self.status.active_audio_cue.is_some() && !audio.is_playing() {
            self.status.active_audio_cue = None;
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

#[derive(Debug)]
pub struct SceneCoordinator {
    library: SceneLibrary,
    next_run_id: u64,
    active: Option<ScenePlayer>,
    last: Option<SceneStatus>,
    light: Rgbw8,
}

impl SceneCoordinator {
    pub fn new(library: SceneLibrary, initial_light: Rgbw8) -> Self {
        Self {
            library,
            next_run_id: 1,
            active: None,
            last: None,
            light: initial_light,
        }
    }

    pub fn start(&mut self, name: &str, now_seconds: f64) -> Result<SceneStatus> {
        let definition = self.library.scene(name)?.clone();
        self.start_definition(definition, now_seconds)
    }

    pub fn start_definition(
        &mut self,
        definition: SceneDefinition,
        now_seconds: f64,
    ) -> Result<SceneStatus> {
        if let Some(active) = self.active.as_ref() {
            return Err(Error::InvalidState(format!(
                "Scene '{}' is already active as run {}.",
                active.status().name,
                active.status().run_id
            )));
        }
        let run_id = self.next_run_id;
        self.next_run_id = self
            .next_run_id
            .checked_add(1)
            .ok_or_else(|| Error::Runtime("Orion scene run ID overflowed.".into()))?;
        let player = ScenePlayer::new(run_id, definition, now_seconds, self.light)?;
        let status = player.status().clone();
        self.active = Some(player);
        Ok(status)
    }

    pub fn tick<M: SceneMotionDevice, L: LightingDevice + ?Sized, A: AudioDevice + ?Sized>(
        &mut self,
        now_seconds: f64,
        motion: &mut M,
        lighting: &mut L,
        audio: &mut A,
    ) -> Result<()> {
        let Some(player) = self.active.as_mut() else {
            return Ok(());
        };
        if let Err(error) = player.tick(now_seconds, motion, lighting, audio) {
            player.fail(now_seconds, motion, audio, error.to_string());
        }
        self.archive_terminal();
        Ok(())
    }

    pub fn cancel<M: SceneMotionDevice, A: AudioDevice + ?Sized>(
        &mut self,
        now_seconds: f64,
        motion: &mut M,
        audio: &mut A,
    ) -> Result<SceneStatus> {
        let player = self
            .active
            .as_mut()
            .ok_or_else(|| Error::InvalidState("No scene is active.".into()))?;
        player.cancel(now_seconds, motion, audio)?;
        self.archive_terminal();
        self.last
            .clone()
            .ok_or_else(|| Error::Runtime("Cancelled scene result was not retained.".into()))
    }

    pub fn active_status(&self) -> Option<&SceneStatus> {
        self.active.as_ref().map(ScenePlayer::status)
    }

    pub fn last_status(&self) -> Option<&SceneStatus> {
        self.last.as_ref()
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub fn names(&self) -> Vec<String> {
        self.library.names()
    }

    pub fn replace_library(&mut self, library: SceneLibrary) -> Result<Vec<String>> {
        if let Some(active) = self.active.as_ref() {
            return Err(Error::InvalidState(format!(
                "Cannot reload scenes while '{}' is active as run {}.",
                active.status().name,
                active.status().run_id
            )));
        }
        let names = library.names();
        self.library = library;
        Ok(names)
    }

    fn archive_terminal(&mut self) {
        if !self
            .active
            .as_ref()
            .is_some_and(|player| player.status().state.is_terminal())
        {
            return;
        }
        let player = self.active.take().expect("terminal scene must be active");
        self.light = player.light();
        self.last = Some(player.status().clone());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::tempdir;

    use super::*;
    use crate::audio::{AudioCommand, RecordingAudioDevice, UnavailableAudioDevice};
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

        fn joint_limits(&self) -> Result<Vec<crate::driver::JointLimit>> {
            Ok(crate::ORION_JOINT_NAMES
                .iter()
                .map(|name| crate::driver::JointLimit {
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

    fn scene_library(scene: SceneDefinition) -> SceneLibrary {
        SceneLibrary {
            scenes: [(scene.name.clone(), scene)].into_iter().collect(),
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
    fn waits_for_audio_playback_before_completing() {
        let scene = SceneDefinition {
            name: "audio_only".into(),
            description: "Wait for one cue.".into(),
            events: vec![SceneEvent {
                at_seconds: 0.0,
                action: SceneAction::Audio {
                    cue: "acknowledge".into(),
                },
            }],
        };
        let mut player = ScenePlayer::new(4, scene, 2.0, Rgbw8::OFF).unwrap();
        let mut motion = FakeMotionDevice::new();
        let mut lighting = RecordingLightingDevice::new(1).unwrap();
        let mut audio = RecordingAudioDevice::blocking();

        player
            .tick(2.0, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        assert_eq!(player.status().state, ScenePhase::Executing);
        assert_eq!(
            player.status().active_audio_cue.as_deref(),
            Some("acknowledge")
        );

        player
            .tick(2.1, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        assert_eq!(player.status().state, ScenePhase::Executing);

        audio.finish();
        player
            .tick(2.2, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        assert_eq!(player.status().state, ScenePhase::Completed);
        assert!(player.status().active_audio_cue.is_none());
    }

    #[test]
    fn due_audio_cues_queue_in_authored_order_under_single_ownership() {
        let scene = SceneDefinition {
            name: "audio_queue".into(),
            description: "Queue simultaneous authored cues.".into(),
            events: vec![
                SceneEvent {
                    at_seconds: 0.0,
                    action: SceneAction::Audio {
                        cue: "notice_warm".into(),
                    },
                },
                SceneEvent {
                    at_seconds: 0.0,
                    action: SceneAction::Audio {
                        cue: "settle_soft".into(),
                    },
                },
            ],
        };
        let mut player = ScenePlayer::new(5, scene, 3.0, Rgbw8::OFF).unwrap();
        let mut motion = FakeMotionDevice::new();
        let mut lighting = RecordingLightingDevice::new(1).unwrap();
        let mut audio = RecordingAudioDevice::blocking();

        player
            .tick(3.0, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        assert_eq!(
            audio.commands(),
            &[AudioCommand::Play("notice_warm".into())]
        );
        assert_eq!(player.status().dispatched_events, 1);

        audio.finish();
        player
            .tick(3.1, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        assert_eq!(
            audio.commands(),
            &[
                AudioCommand::Play("notice_warm".into()),
                AudioCommand::Play("settle_soft".into()),
            ]
        );
        assert_eq!(player.status().dispatched_events, 2);
        assert_eq!(player.status().state, ScenePhase::Executing);

        audio.finish();
        player
            .tick(3.2, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        assert_eq!(player.status().state, ScenePhase::Completed);
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
    fn coordinator_assigns_ids_rejects_busy_and_retains_completion() {
        let mut coordinator = SceneCoordinator::new(scene_library(example_scene()), Rgbw8::OFF);
        let first = coordinator.start("acknowledge", 10.0).unwrap();
        assert_eq!(first.run_id, 1);
        assert!(coordinator.start("acknowledge", 10.0).is_err());

        let mut motion = FakeMotionDevice::new();
        let mut lighting = RecordingLightingDevice::new(2).unwrap();
        let mut audio = RecordingAudioDevice::default();
        coordinator
            .tick(10.0, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        motion.finish(1, MovementPhase::Completed);
        coordinator
            .tick(11.0, &mut motion, &mut lighting, &mut audio)
            .unwrap();

        assert!(!coordinator.is_active());
        let completed = coordinator.last_status().unwrap();
        assert_eq!(completed.run_id, 1);
        assert_eq!(completed.state, ScenePhase::Completed);
        assert_eq!(completed.dispatched_events, completed.event_count);

        let second = coordinator.start("acknowledge", 12.0).unwrap();
        assert_eq!(second.run_id, 2);
    }

    #[test]
    fn coordinator_records_device_failure_without_propagating_it() {
        let mut coordinator = SceneCoordinator::new(scene_library(example_scene()), Rgbw8::OFF);
        coordinator.start("acknowledge", 0.0).unwrap();
        let mut motion = FakeMotionDevice::new();
        let mut lighting = RecordingLightingDevice::new(1).unwrap();
        let mut audio = UnavailableAudioDevice;

        coordinator
            .tick(0.0, &mut motion, &mut lighting, &mut audio)
            .unwrap();
        coordinator
            .tick(0.5, &mut motion, &mut lighting, &mut audio)
            .unwrap();

        assert!(!coordinator.is_active());
        let failed = coordinator.last_status().unwrap();
        assert_eq!(failed.state, ScenePhase::Failed);
        assert!(failed.error.as_deref().unwrap().contains("not configured"));
        assert_eq!(motion.cancel_count, 1);
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
            r#"format_version: 2
scene:
  name: acknowledge
  description: Test scene.
  motion: [{at: 0.0, play: look_at_left_expressive}]
  lighting: [{on_marker: notice, effect: acknowledge_pulse, duration: 0.4}]
  audio: [{on_marker: notice, cue: acknowledge_warm}]
  finish: {anchor: final_pose, lighting: pose_default}
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
            r#"format_version: 2
scene:
  name: bad
  motion: [{at: 0.0, play: missing}]
  lighting: []
  audio: []
  finish: {anchor: final_pose, lighting: pose_default}
"#,
        )
        .unwrap();
        assert!(SceneLibrary::load(directory.path(), &poses, &motions).is_err());
    }

    #[test]
    fn rejects_v1_unknown_fields_and_overlapping_motion_tracks() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let poses = PoseLibrary::load(
            root.join("motion/config/poses.yaml"),
            &crate::ORION_JOINT_NAMES,
        )
        .unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();

        let v1 = r#"format_version: 1
scene: {name: old, motion: [], lighting: [], audio: []}
"#;
        assert!(
            parse_scene_document(v1, "v1-test", &poses, &motions)
                .unwrap_err()
                .to_string()
                .contains("v2 required")
        );

        let unknown = r#"format_version: 2
scene:
  name: unknown
  legacy_sequence: true
  motion: [{at: 0.0, play: look_at_left_expressive}]
  lighting: []
  audio: []
  finish: {anchor: final_pose, lighting: pose_default}
"#;
        assert!(
            parse_scene_document(unknown, "unknown-test", &poses, &motions)
                .unwrap_err()
                .to_string()
                .contains("unknown field")
        );

        let overlap = r#"format_version: 2
scene:
  name: overlap
  motion:
    - {at: 0.0, play: look_at_left_expressive}
    - {at: 0.1, play: look_at_right_expressive}
  lighting: []
  audio: []
  finish: {anchor: final_pose, lighting: pose_default}
"#;
        assert!(
            parse_scene_document(overlap, "overlap-test", &poses, &motions)
                .unwrap_err()
                .to_string()
                .contains("may not overlap")
        );
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
        let cues = CueLibrary::load(root.join("audio/cues")).unwrap();
        scenes.validate_audio_cues(&cues).unwrap();

        let scene = scenes.scene("acknowledge_left").unwrap();
        assert_eq!(scene.events.len(), 5);
        assert_eq!(scene.events[0].at_seconds, 0.0);
        assert!(scene.events.iter().any(|event| matches!(
            &event.action, SceneAction::OnMarker { marker, .. } if marker == "notice"
        )));
        let scene = scenes.scene("acknowledge_right").unwrap();
        assert_eq!(scene.events.len(), 5);
        let scene = scenes.scene("return_home").unwrap();
        assert_eq!(scene.events.len(), 4);
        assert!(matches!(
            &scene.events[0].action,
            SceneAction::Motion(SceneMotion::Play { motion }) if motion == "return_home"
        ));
        assert!(scenes.scene("deployment_smoke").is_ok());
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
