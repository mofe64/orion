use std::path::PathBuf;

use super::options::parse_rgbw;
use crate::expression::{lamp::LampProgram, voice_feedback::VoiceFeedback};
use crate::{
    AudioDevice, CharacterCoordinator, CueLibrary, MotionLibrary, ORION_JOINT_NAMES, PoseLibrary,
    RuntimeCore, RuntimeDriver, SceneCoordinator, SceneLibrary, SpeechCoordinator,
    parse_scene_document,
};

fn speech_cue_argument(command: &str) -> (&str, Option<&str>) {
    if command.starts_with("speech stream ") || command.starts_with("speech append ") {
        let command = command.trim_end();
        if let Some((prefix, value)) = command.rsplit_once(char::is_whitespace)
            && let Some(value) = value.strip_prefix("cues=")
        {
            return (prefix.trim_end(), Some(value));
        }
    }
    (command, None)
}
fn parse_speech_cues(
    value: Option<&str>,
) -> crate::Result<Vec<crate::expression::speech::SpeechCue>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let names: Vec<_> = value.split(',').collect();
    if names.len() > 4 {
        return Err(crate::Error::InvalidArgument(
            "At most four speech cues per chunk.".into(),
        ));
    }
    names.into_iter().map(str::parse).collect()
}

#[derive(Clone)]
pub(super) struct AssetReloadContext {
    pub(super) poses_file: PathBuf,
    pub(super) user_poses_directory: PathBuf,
    pub(super) motions_directory: PathBuf,
    pub(super) scenes_directory: PathBuf,
    pub(super) cues: CueLibrary,
}

impl AssetReloadContext {
    fn load_poses(&self) -> crate::Result<PoseLibrary> {
        PoseLibrary::load_with_user_directory(
            &self.poses_file,
            &self.user_poses_directory,
            &ORION_JOINT_NAMES,
        )
    }

    fn load_motions(&self, poses: &PoseLibrary) -> crate::Result<MotionLibrary> {
        MotionLibrary::load(&self.motions_directory, poses)
    }

    fn load_scenes(
        &self,
        poses: &PoseLibrary,
        motions: &MotionLibrary,
    ) -> crate::Result<SceneLibrary> {
        let library = SceneLibrary::load(&self.scenes_directory, poses, motions)?;
        library.validate_audio_cues(&self.cues)?;
        Ok(library)
    }

    fn load_all(&self) -> crate::Result<(PoseLibrary, MotionLibrary, SceneLibrary)> {
        let poses = self.load_poses()?;
        let motions = self.load_motions(&poses)?;
        let scenes = self.load_scenes(&poses, &motions)?;
        Ok((poses, motions, scenes))
    }
}

pub(super) fn handle_daemon_command_with_character<D: RuntimeDriver, A: AudioDevice + ?Sized>(
    command: &str,
    now_seconds: f64,
    core: &mut RuntimeCore<D>,
    scenes: &mut SceneCoordinator,
    speech: &mut SpeechCoordinator,
    character: &mut CharacterCoordinator,
    audio: &mut A,
    asset_reload: Option<&AssetReloadContext>,
) -> String {
    match handle_daemon_command_inner(
        command,
        now_seconds,
        core,
        scenes,
        speech,
        audio,
        asset_reload,
        Some(character),
    ) {
        Ok(response) => response,
        Err(error) => serde_json::json!({"ok": false, "error": error.to_string()}).to_string(),
    }
}

#[cfg(test)]
pub(super) fn handle_daemon_command<D: RuntimeDriver, A: AudioDevice + ?Sized>(
    command: &str,
    now_seconds: f64,
    core: &mut RuntimeCore<D>,
    scenes: &mut SceneCoordinator,
    speech: &mut SpeechCoordinator,
    audio: &mut A,
) -> String {
    handle_daemon_command_with_reload(command, now_seconds, core, scenes, speech, audio, None)
}

#[cfg(test)]
pub(super) fn handle_daemon_command_with_reload<D: RuntimeDriver, A: AudioDevice + ?Sized>(
    command: &str,
    now_seconds: f64,
    core: &mut RuntimeCore<D>,
    scenes: &mut SceneCoordinator,
    speech: &mut SpeechCoordinator,
    audio: &mut A,
    asset_reload: Option<&AssetReloadContext>,
) -> String {
    match handle_daemon_command_inner(
        command,
        now_seconds,
        core,
        scenes,
        speech,
        audio,
        asset_reload,
        None,
    ) {
        Ok(response) => response,
        Err(error) => serde_json::json!({"ok": false, "error": error.to_string()}).to_string(),
    }
}

pub(super) fn handle_daemon_command_inner<D: RuntimeDriver, A: AudioDevice + ?Sized>(
    command: &str,
    now_seconds: f64,
    core: &mut RuntimeCore<D>,
    scenes: &mut SceneCoordinator,
    speech: &mut SpeechCoordinator,
    audio: &mut A,
    asset_reload: Option<&AssetReloadContext>,
    mut character: Option<&mut CharacterCoordinator>,
) -> crate::Result<String> {
    let (command, cue_value) = speech_cue_argument(command);
    let cues = parse_speech_cues(cue_value)?;
    if let Some(argument) = command.strip_prefix("volume ") {
        let current = audio.volume_status().volume;
        let level = match argument {
            "status" => None,
            "louder" => Some(current.saturating_add(10).min(100)),
            "quieter" => Some(current.saturating_sub(10)),
            value => Some(
                value
                    .parse::<u8>()
                    .ok()
                    .filter(|level| *level <= 100)
                    .ok_or_else(|| crate::Error::InvalidArgument("Volume must be 0–100.".into()))?,
            ),
        };
        if let Some(level) = level {
            audio.set_volume(level)?;
        }
        return Ok(serde_json::json!({"ok":true,"audio":audio.volume_status()}).to_string());
    }
    if let Some(arguments) = command.strip_prefix("camera-rays ") {
        let request = serde_json::from_str(arguments).map_err(|error| {
            crate::Error::InvalidArgument(format!("Invalid camera-rays JSON: {error}"))
        })?;
        return Ok(serde_json::to_string(&core.camera_rays(request)?)?);
    }
    if let Some(arguments) = command.strip_prefix("aim-light ") {
        if scenes.is_active() || speech.is_active() {
            return Err(crate::Error::InvalidState(
                "Aiming cannot interrupt scene or speech".into(),
            ));
        }
        let request = serde_json::from_str::<crate::expression::character::WorkRequest>(arguments)
            .map_err(|error| {
                crate::Error::InvalidArgument(format!("Invalid aim-light JSON: {error}"))
            })?;
        let coordinator = character
            .as_deref_mut()
            .ok_or_else(|| crate::Error::InvalidState("Character is unavailable".into()))?;
        return Ok(coordinator
            .work_request(request, now_seconds, core)?
            .to_string());
    }
    if matches!(command, "gaze status" | "gaze clear" | "gaze resume") {
        let coordinator = character
            .as_deref_mut()
            .ok_or_else(|| crate::Error::InvalidState("Character is unavailable".into()))?;
        if command == "gaze clear" {
            coordinator.clear_gaze(now_seconds, core)?;
        } else if command == "gaze resume" {
            coordinator.resume_gaze(core)?;
        }
        return Ok(serde_json::json!({"ok":true,"gaze":coordinator.gaze_status()}).to_string());
    }
    if let Some(arguments) = command.strip_prefix("gaze target ") {
        let target = serde_json::from_str(arguments).map_err(|error| {
            crate::Error::InvalidArgument(format!("Invalid gaze target JSON: {error}"))
        })?;
        let coordinator = character
            .as_deref_mut()
            .ok_or_else(|| crate::Error::InvalidState("Character is unavailable".into()))?;
        if (scenes.is_active() || speech.is_active()) && coordinator.gaze_status().enabled {
            return Ok(
                serde_json::json!({"ok":true,"accepted":false,"gaze":coordinator.gaze_status()})
                    .to_string(),
            );
        }
        let accepted = coordinator.queue_gaze_target(target, now_seconds, core)?;
        return Ok(
            serde_json::json!({"ok":true,"accepted":accepted,"gaze":coordinator.gaze_status()})
                .to_string(),
        );
    }
    if command == "character status" {
        let status = character.as_deref().map(CharacterCoordinator::status);
        return Ok(serde_json::json!({"ok": true, "character": status}).to_string());
    }
    if let Some(arguments) = command.strip_prefix("character attend ") {
        if scenes.is_active() || speech.is_active() {
            return Err(crate::Error::InvalidState(
                "Attention cannot interrupt scene or speech.".into(),
            ));
        }
        let parts: Vec<_> = arguments.split_whitespace().collect();
        if parts.len() != 2 {
            return Err(crate::Error::InvalidArgument(
                "Use character attend left|right CONFIDENCE".into(),
            ));
        }
        let confidence: f64 = parts[1]
            .parse()
            .map_err(|_| crate::Error::InvalidArgument("Invalid attention confidence".into()))?;
        let coordinator = character
            .as_deref_mut()
            .ok_or_else(|| crate::Error::InvalidState("Character is unavailable".into()))?;
        let status = coordinator.attend(parts[0], confidence, now_seconds, core)?;
        return Ok(serde_json::json!({"ok": true, "character": status}).to_string());
    }
    if matches!(command, "stop" | "disable") {
        if let Some(coordinator) = character.as_deref_mut() {
            coordinator.clear_attention();
        }
    }
    if command == "character start" {
        let coordinator = character.as_deref_mut().ok_or_else(|| {
            crate::Error::InvalidState("Character coordinator is not configured.".into())
        })?;
        let status = coordinator.start(now_seconds, core)?;
        return Ok(
            serde_json::json!({"ok": true, "command": "character_start", "character": status})
                .to_string(),
        );
    }
    if command == "character rest" {
        if scenes.is_active() {
            scenes.cancel(now_seconds, core, audio)?;
        }
        if speech.is_active() {
            speech.cancel(audio)?;
        }
        let coordinator = character.as_deref_mut().ok_or_else(|| {
            crate::Error::InvalidState("Character coordinator is not configured.".into())
        })?;
        return Ok(coordinator.rest(now_seconds, core)?.to_string());
    }
    if command == "character stop" {
        if scenes.is_active() {
            scenes.cancel(now_seconds, core, audio)?;
        }
        if speech.is_active() {
            speech.cancel(audio)?;
        }
        let coordinator = character.as_deref_mut().ok_or_else(|| {
            crate::Error::InvalidState("Character coordinator is not configured.".into())
        })?;
        let status = coordinator.stop(now_seconds, core)?;
        return Ok(
            serde_json::json!({"ok": true, "command": "character_stop", "character": status})
                .to_string(),
        );
    }
    if let Some(state) = command.strip_prefix("character state ") {
        let coordinator = character.as_deref_mut().ok_or_else(|| {
            crate::Error::InvalidState("Character coordinator is not configured.".into())
        })?;
        let status = coordinator.set_reaction(state.trim(), now_seconds, core)?;
        return Ok(
            serde_json::json!({"ok": true, "command": "character_state", "character": status})
                .to_string(),
        );
    }
    if command == "speech status" {
        return Ok(serde_json::json!({
            "ok": true,
            "speech": speech.active_status(),
            "last_speech": speech.last_status(),
        })
        .to_string());
    }
    if let Some(arguments) = command.strip_prefix("speech append ") {
        let fields: Vec<_> = arguments.split_whitespace().collect();
        if fields.len() != 3 {
            return Err(crate::Error::InvalidArgument(
                "Expected run, sequence and chunk identifier.".into(),
            ));
        }
        let run = fields[0]
            .parse::<u64>()
            .map_err(|_| crate::Error::InvalidArgument("Invalid speech run.".into()))?;
        let sequence = fields[1]
            .parse::<usize>()
            .map_err(|_| crate::Error::InvalidArgument("Invalid speech sequence.".into()))?;
        speech.append_stream(run, sequence, fields[2], cues)?;
        return Ok(serde_json::json!({"ok":true,"run_id":run}).to_string());
    }
    if let Some(arguments) = command.strip_prefix("speech end ") {
        let fields: Vec<_> = arguments.split_whitespace().collect();
        if fields.len() != 2 {
            return Err(crate::Error::InvalidArgument(
                "Expected run and final sequence.".into(),
            ));
        }
        let run = fields[0]
            .parse::<u64>()
            .map_err(|_| crate::Error::InvalidArgument("Invalid speech run.".into()))?;
        let sequence = fields[1]
            .parse::<usize>()
            .map_err(|_| crate::Error::InvalidArgument("Invalid speech sequence.".into()))?;
        speech.end_stream(run, sequence)?;
        return Ok(serde_json::json!({"ok":true,"run_id":run}).to_string());
    }
    if let Some(identifier) = command
        .strip_prefix("speech file ")
        .or_else(|| command.strip_prefix("speech stream "))
    {
        if scenes.is_active() {
            return Ok(serde_json::json!({"ok": false, "command": "speech_file", "error": "scene already active"}).to_string());
        }
        let status = if command.starts_with("speech stream ") {
            speech.start_stream(identifier.trim(), cues)?
        } else {
            speech.start_spooled(identifier.trim())?
        };
        return Ok(serde_json::json!({"ok": true, "command": "speech_file", "run_id": status.run_id, "state": status.state}).to_string());
    }
    if command == "speech stop" {
        let status = speech.cancel(audio)?;
        return Ok(serde_json::json!({
            "ok": true,
            "command": "speech_stop",
            "last_speech": status,
        })
        .to_string());
    }
    if command == "scene status" {
        return Ok(serde_json::json!({
            "ok": true,
            "scene": scenes.active_status(),
            "last_scene": scenes.last_status(),
        })
        .to_string());
    }
    if command == "scene reload" {
        let context = asset_reload
            .ok_or_else(|| crate::Error::InvalidState("Scene reload is not configured.".into()))?;
        let library = context.load_scenes(core.poses(), core.motions())?;
        let names = scenes.replace_library(library)?;
        return Ok(serde_json::json!({
            "ok": true,
            "command": "scene_reload",
            "scenes": names,
        })
        .to_string());
    }
    if command == "asset reload" {
        let context = asset_reload
            .ok_or_else(|| crate::Error::InvalidState("Asset reload is not configured.".into()))?;
        if scenes.is_active() || core.snapshot().motion.is_some() {
            return Ok(serde_json::json!({
                "ok": false,
                "command": "asset_reload",
                "error": "cannot reload assets while a scene or movement is active",
            })
            .to_string());
        }
        let (poses, motions, scene_library) = context.load_all()?;
        core.replace_motion_assets(poses, motions)?;
        let scene_names = scenes.replace_library(scene_library)?;
        return Ok(serde_json::json!({
            "ok": true,
            "command": "asset_reload",
            "poses": core.poses().names(),
            "motions": core.motions().names(),
            "scenes": scene_names,
        })
        .to_string());
    }
    if command == "scene list" {
        return Ok(serde_json::json!({"ok": true, "scenes": scenes.names()}).to_string());
    }
    if let Some(document) = command.strip_prefix("scene preview ") {
        if let Some(coordinator) = character.as_deref_mut() {
            coordinator.preempt_for_foreground(now_seconds, core)?;
        }
        let context = asset_reload
            .ok_or_else(|| crate::Error::InvalidState("Scene preview is not configured.".into()))?;
        if let Some(motion) = core.snapshot().motion.as_ref() {
            return Ok(serde_json::json!({
                "ok": false,
                "command": "scene_preview",
                "error": "motion already active",
                "active_run_id": motion.run_id,
            })
            .to_string());
        }
        if speech.is_active() {
            speech.cancel(audio)?;
        }
        let definition = parse_scene_document(
            document,
            "Studio ephemeral preview",
            core.poses(),
            core.motions(),
        )?;
        definition.validate_audio_cues(&context.cues)?;
        let status = scenes.start_definition(definition, now_seconds)?;
        if let Some(coordinator) = character.as_deref_mut() {
            coordinator.note_foreground_scene_started(now_seconds, status.run_id);
        }
        return Ok(serde_json::json!({
            "ok": true,
            "command": "scene_preview",
            "run_id": status.run_id,
            "scene": status.name,
            "state": status.state,
            "event_count": status.event_count,
            "persisted": false,
        })
        .to_string());
    }
    if let Some(name) = command.strip_prefix("scene start ") {
        if let Some(coordinator) = character.as_deref_mut() {
            coordinator.preempt_for_foreground(now_seconds, core)?;
        }
        let name = name.trim();
        if name.is_empty() || name.split_whitespace().count() != 1 {
            return Ok(serde_json::json!({
                "ok": false,
                "command": "scene_start",
                "error": "expected scene start SCENE",
            })
            .to_string());
        }
        if let Some(motion) = core.snapshot().motion.as_ref() {
            return Ok(serde_json::json!({
                "ok": false,
                "command": "scene_start",
                "error": "motion already active",
                "active_run_id": motion.run_id,
            })
            .to_string());
        }
        if speech.is_active() {
            speech.cancel(audio)?;
        }
        let status = scenes.start(name, now_seconds)?;
        if let Some(coordinator) = character.as_deref_mut() {
            coordinator.note_foreground_scene_started(now_seconds, status.run_id);
        }
        return Ok(serde_json::json!({
            "ok": true,
            "command": "scene_start",
            "run_id": status.run_id,
            "scene": status.name,
            "state": status.state,
            "event_count": status.event_count,
        })
        .to_string());
    }
    if command == "scene stop" || (command == "stop" && scenes.is_active()) {
        let status = scenes.cancel(now_seconds, core, audio)?;
        return Ok(serde_json::json!({
            "ok": true,
            "command": "scene_stop",
            "last_scene": status,
        })
        .to_string());
    }
    if scenes.is_active() && (command.starts_with("goto ") || command.starts_with("play ")) {
        return Ok(serde_json::json!({
            "ok": false,
            "command": command.split_whitespace().next().unwrap_or("motion"),
            "error": "scene already active",
            "active_scene_run_id": scenes.active_status().map(|status| status.run_id),
        })
        .to_string());
    }
    if (command.starts_with("goto ") || command.starts_with("play "))
        && let Some(coordinator) = character.as_deref_mut()
    {
        if speech.is_active() {
            speech.cancel(audio)?;
        }
        coordinator.preempt_for_foreground(now_seconds, core)?;
        let response = core.handle_command(command, now_seconds);
        let accepted = serde_json::from_str::<serde_json::Value>(&response)
            .ok()
            .and_then(|value| value.get("ok").and_then(serde_json::Value::as_bool))
            == Some(true);
        if accepted {
            coordinator.note_foreground_started(now_seconds);
        }
        return Ok(response);
    }
    Ok(core.handle_command(command, now_seconds))
}

pub(super) fn dispatch_command<D: RuntimeDriver>(
    command: &str,
    now_seconds: f64,
    core: &mut RuntimeCore<D>,
    scenes: &mut SceneCoordinator,
    speech: &mut SpeechCoordinator,
    character: &mut CharacterCoordinator,
    audio: &mut dyn AudioDevice,
    asset_reload: &AssetReloadContext,
    feedback: &mut VoiceFeedback,
    manual_light: &mut Option<LampProgram>,
    voice_speech_run: &mut Option<(u64, String)>,
    rest: &mut crate::expression::rest::RestCoordinator,
    routines: &mut crate::expression::routines::Routines,
) -> String {
    use crate::expression::routines::Request;
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    if command.starts_with("gaze surroundings ") {
        // A remark session keeps the noticed reaction moving until speech starts.
        character.set_curiosity_available(
            rest.reactions_ready()
                && (!feedback.session_active() || feedback.remark())
                && !speech.is_active()
                && !scenes.is_active()
                && !routines.curiosity_quiet(),
        );
        let local = chrono::Local::now();
        let lamp = owner_lamp_status(
            manual_light,
            routines,
            character,
            scenes,
            speech,
            feedback,
            rest,
            now_seconds,
        );
        character.set_conditions(crate::expression::situation::Conditions {
            busy: !rest.reactions_ready()
                || feedback.session_active()
                || speech.is_active()
                || scenes.is_active()
                || routines.curiosity_quiet()
                || audio.music_playing(),
            hour: f64::from(chrono::Timelike::hour(&local))
                + f64::from(chrono::Timelike::minute(&local)) / 60.0,
            day: i64::from(chrono::Datelike::num_days_from_ce(&local)),
            lamp: crate::expression::situation::LampView {
                on: lamp["on"].as_bool().unwrap_or(false),
                brightness: lamp["brightness"].as_f64().unwrap_or(0.0),
                mood: lamp["mood"].as_str().unwrap_or("custom").into(),
            },
        });
        if command == "gaze surroundings status" {
            return serde_json::json!({"ok":true,"ready":character.curiosity_ready(core)})
                .to_string();
        }
        if command == "gaze surroundings clear" {
            return match character.clear_surroundings(now_seconds, core) {
                Ok(()) => serde_json::json!({"ok":true}).to_string(),
                Err(error) => serde_json::json!({"ok":false,"error":error.to_string()}).to_string(),
            };
        }
        let result = serde_json::from_str::<crate::expression::character::Surroundings>(
            command.strip_prefix("gaze surroundings ").unwrap(),
        )
        .map_err(|error| error.to_string())
        .and_then(|value| {
            character
                .observe_surroundings(value, now_seconds, core)
                .map_err(|error| error.to_string())
        });
        return match result {
            Ok(()) => serde_json::json!({"ok":true}).to_string(),
            Err(error) => serde_json::json!({"ok":false,"error":error}).to_string(),
        };
    }
    if command == "lamp-status" {
        return serde_json::json!({"ok":true,"lamp":owner_lamp_status(manual_light, routines, character, scenes, speech, feedback, rest, now_seconds)}).to_string();
    }
    if command == "routines status" {
        // The voice listener polls this; a due remark rides along with alerts.
        return serde_json::json!({"ok":true,"routines":routines.status(wall, now_seconds),
            "remark":character.ready_remark(now_seconds)})
        .to_string();
    }
    if let Some(seconds) = command.strip_prefix("character remarks quiet ") {
        return match seconds
            .parse::<f64>()
            .map_err(|_| "Invalid quiet time")
            .and_then(|seconds| character.remarks.quiet(seconds, now_seconds))
        {
            Ok(_) => serde_json::json!({"ok":true,"remarks":character.remarks.status(now_seconds)})
                .to_string(),
            Err(error) => serde_json::json!({"ok":false,"error":error}).to_string(),
        };
    }
    if let Some(body) = command.strip_prefix("routines ") {
        let result = (|| -> Result<serde_json::Value, String> {
            let request: Request = serde_json::from_str(body).map_err(|e| e.to_string())?;
            match request {
                Request::StartLightRoutine { routine, minutes } => {
                    let bounds = match routine {
                        crate::expression::routines::LightRoutine::WindDown => 10..=60,
                        crate::expression::routines::LightRoutine::Focus => 5..=120,
                    };
                    if !bounds.contains(&minutes) {
                        return Err("Invalid light routine duration".into());
                    }
                    if rest.failed()
                        || scenes.is_active()
                        || speech.is_active()
                        || routines.ringing()
                    {
                        return Err(
                            "Finish the scene, speech or alert before starting a light routine"
                                .into(),
                        );
                    }
                    if !character.status().enabled {
                        character
                            .start(now_seconds, core)
                            .map_err(|e| e.to_string())?;
                        rest.started();
                    }
                    let result = routines.start_light(
                        routine,
                        minutes,
                        wall,
                        now_seconds,
                        manual_light,
                        audio,
                    )?;
                    rest.set_mode(routines.mode(), now_seconds);
                    return Ok(result);
                }
                Request::StopLightRoutine => {
                    let prior_mode = routines.mode();
                    routines.stop_light(wall, now_seconds, manual_light, audio)?;
                    if prior_mode != routines.mode() {
                        rest.set_mode(routines.mode(), now_seconds);
                    }
                    return Ok(routines.status(wall, now_seconds));
                }
                _ => {}
            }
            let mode = if let Request::SetMode { mode } = &request {
                Some(*mode)
            } else {
                None
            };
            if mode.is_some() && (rest.failed() || scenes.is_active() || routines.ringing()) {
                return Err(
                    "Finish the scene or alert, or recover the runtime before changing mode".into(),
                );
            }
            if mode.is_some() {
                routines.stop_light(wall, now_seconds, manual_light, audio)?;
                if !character.status().enabled {
                    character
                        .start(now_seconds, core)
                        .map_err(|e| e.to_string())?;
                    rest.started();
                }
            }
            let result = routines.request(request, wall, now_seconds, audio)?;
            if let Some(mode) = mode {
                if mode == crate::expression::routines::UserMode::Idle {
                    character.release_work_anchor();
                }
                rest.set_mode(mode, now_seconds);
            }
            Ok(result)
        })();
        return match result {
            Ok(value) => serde_json::json!({"ok":true,"result":value}).to_string(),
            Err(error) => serde_json::json!({"ok":false,"error":error}).to_string(),
        };
    }
    if let Some(session) = command.strip_prefix("sleep ") {
        if !feedback.owns(session)
            || !feedback.confirmed_activity()
            || !matches!(
                rest.status(now_seconds).state,
                crate::expression::rest::RestState::Awake
                    | crate::expression::rest::RestState::Waking
            )
            || routines.ringing()
        {
            return serde_json::json!({"ok":false,"error":"Sleep requires the current confirmed voice session"}).to_string();
        }
        rest.request_sleep(session);
        return serde_json::json!({"ok":true,"sleep_after_reply":true}).to_string();
    }
    if routines.ringing() {
        if matches!(
            command,
            "stop" | "disable" | "character stop" | "character rest"
        ) {
            if let Err(error) = routines.stop("dismissed", now_seconds, audio) {
                routines.error = Some(error);
            }
        } else if command.starts_with("voice ") && command != "voice status" {
            // Retired voice replies cannot replace or stop an alert. Wake dismissal
            // uses the listener's dedicated local routines stop command.
            return serde_json::json!({"ok":true,"ignored":"alert is ringing"}).to_string();
        } else if command.starts_with("speech file ")
            || command.starts_with("speech stream ")
            || command.starts_with("scene start ")
            || command.starts_with("scene preview ")
            || command.starts_with("character ") && command != "character status"
            || command.starts_with("goto ")
            || command.starts_with("play ")
            || command.starts_with("aim-light ")
        {
            return serde_json::json!({"ok":false,"error":"Dismiss the alert first"}).to_string();
        }
    }
    if command == "character status" {
        return serde_json::json!({"ok": true, "character": character.status(), "remarks": character.remarks.status(now_seconds), "rest": rest.status(now_seconds), "routines": routines.status(wall, now_seconds), "lamp":owner_lamp_status(manual_light,routines,character,scenes,speech,feedback,rest,now_seconds)}).to_string();
    }
    if let Some(fields) = command.strip_prefix("voice ") {
        if fields == "status" {
            return serde_json::json!({"ok": true, "voice": feedback,
                "vision_session": feedback.vision_session(now_seconds)})
            .to_string();
        }
        let parts: Vec<_> = fields.split_whitespace().collect();
        if parts.len() < 2 || parts.len() > 6 {
            return serde_json::json!({"ok": false, "error": "Expected voice SESSION EVENT"})
                .to_string();
        }
        if parts[1] == "remark" {
            // The listener opens a silent speaking session for the due remark.
            // Taking the remark first means a failed session never repeats it.
            let id = match parts.as_slice() {
                [_, _, id] => id.parse::<u64>().ok(),
                _ => None,
            };
            let Some(id) = id else {
                return serde_json::json!({"ok":false,"error":"Expected voice SESSION remark ID"})
                    .to_string();
            };
            if speech.is_active() || scenes.is_active() || routines.ringing() {
                return serde_json::json!({"ok":false,"error":"Orion is busy"}).to_string();
            }
            if character
                .ready_remark(now_seconds)
                .is_none_or(|remark| remark.id != id)
            {
                return serde_json::json!({"ok":false,"error":"The remark is no longer current"})
                    .to_string();
            }
            if let Err(error) = feedback.event(parts[0], "remark", now_seconds) {
                return serde_json::json!({"ok":false,"error":error}).to_string();
            }
            return match character.take_remark(id, now_seconds) {
                Ok(remark) => serde_json::json!({"ok":true,"remark":remark}).to_string(),
                Err(error) => {
                    feedback.clear();
                    serde_json::json!({"ok":false,"error":error}).to_string()
                }
            };
        }
        if parts[1] == "vision_begin"
            && (speech.is_active() || scenes.is_active() || !feedback.owns(parts[0]))
        {
            return serde_json::json!({"ok":false,"error":"Vision interval requires the current owner without scene or speech"}).to_string();
        }
        if matches!(parts[1], "attend_bearing" | "attend_scan") {
            if !character.gaze_status().enabled {
                return serde_json::json!({"ok":false,"error":"Gaze is unavailable"}).to_string();
            }
            let parsed = (|| -> Result<(Option<(f64, f64)>, f64, Option<u64>), &'static str> {
                let (bearing, age) = match parts[1] {
                    "attend_bearing" if matches!(parts.len(), 5 | 6) => {
                        let az = parts[2].parse::<f64>().map_err(|_| "Invalid bearing")?;
                        let el = parts[3].parse::<f64>().map_err(|_| "Invalid bearing")?;
                        if !az.is_finite()
                            || !el.is_finite()
                            || az.abs() > std::f64::consts::PI
                            || el.abs() > std::f64::consts::FRAC_PI_2
                        {
                            return Err("Invalid bearing");
                        }
                        (Some((az, el)), parts[4])
                    }
                    "attend_scan" if parts.len() == 3 => (None, parts[2]),
                    _ => {
                        return Err(
                            "Expected attend_bearing AZ EL AGE_MS [TRACK_ID] or attend_scan AGE_MS",
                        );
                    }
                };
                let age = age.parse::<f64>().map_err(|_| "Invalid direction age")?;
                if !age.is_finite() || age < 0.0 {
                    return Err("Invalid direction age");
                }
                let track_id = if parts.len() == 6 {
                    let id = parts[5]
                        .parse::<u64>()
                        .map_err(|_| "Invalid speaker track ID")?;
                    if id == 0 {
                        return Err("Invalid speaker track ID");
                    }
                    Some(id)
                } else {
                    None
                };
                Ok((bearing, age, track_id))
            })();
            let (bearing, age, track_id) = match parsed {
                Ok(value) => value,
                Err(error) => return serde_json::json!({"ok":false,"error":error}).to_string(),
            };
            if feedback.owns(parts[0]) && feedback.confirmed_activity() && age < 3000.0 {
                rest.queue_bearing_attention(
                    parts[0],
                    bearing,
                    track_id,
                    now_seconds + (3000.0 - age) / 1000.0,
                );
            }
            return serde_json::json!({"ok":true}).to_string();
        }
        if matches!(parts[1], "attend_left" | "attend_right") {
            if parts.len() > 3 {
                return serde_json::json!({"ok":false,"error":"Unexpected voice arguments"})
                    .to_string();
            }
            let age = parts.get(2).map_or(Ok(0.0), |value| value.parse::<f64>());
            let Ok(age) = age else {
                return serde_json::json!({"ok": false, "error": "Invalid direction age"})
                    .to_string();
            };
            if !age.is_finite() || age < 0.0 {
                return serde_json::json!({"ok": false, "error": "Invalid direction age"})
                    .to_string();
            }
            if feedback.owns(parts[0]) && feedback.confirmed_activity() && age < 3000.0 {
                let side = parts[1].strip_prefix("attend_").unwrap();
                rest.queue_attention(parts[0], side, now_seconds + (3000.0 - age) / 1000.0);
            }
            return serde_json::json!({"ok": true}).to_string();
        }
        if parts.len() != 2 {
            return serde_json::json!({"ok": false, "error": "Unexpected voice arguments"})
                .to_string();
        }
        let had_confirmed = feedback.confirmed_activity();
        let action = if parts[1] == "confirmed" {
            if feedback.confirm(parts[0], now_seconds) {
                // Ownership/confirmation must be accepted before clearing Stop.
                // Rearm attention before the listener's ordered bearing request.
                match character.resume_gaze_for_confirmed_wake(core) {
                    Ok(true) => rest.started(),
                    Ok(false) => {}
                    Err(error) => eprintln!("oriond: stopped gaze recovery failed: {error}"),
                }
                rest.confirmed(parts[0], now_seconds);
                Ok(Some((feedback.reaction(), None)))
            } else if !feedback.owns(parts[0]) || !feedback.confirmed_activity() {
                return serde_json::json!({"ok": false, "error": "No current wake awaits confirmation."}).to_string();
            } else {
                Ok(None)
            }
        } else {
            feedback.event(parts[0], parts[1], now_seconds)
        };
        if action.is_ok() && !had_confirmed && feedback.confirmed_activity() {
            character.mind.confirmed_turn();
            character.remark_engaged(now_seconds);
        }
        match action {
            Ok(Some((reaction, cue))) => {
                if cue == Some("error_muted")
                    && speech.active_status().is_some_and(|status| {
                        voice_speech_run
                            .as_ref()
                            .is_some_and(|(run, _)| *run == status.run_id)
                    })
                {
                    let _ = speech.cancel(audio);
                }
                if rest.reactions_ready() && cue != Some("voice_wake") {
                    let _ = character.set_reaction(reaction, now_seconds, core);
                }
                let cue_ready = rest.reactions_ready()
                    || (cue == Some("notice_warm")
                        && feedback.announcement()
                        && rest.wake_acknowledgment_ready())
                    || (cue == Some("voice_wake") && rest.wake_acknowledgment_ready());
                let wake_cue = cue == Some("voice_wake");
                if cue_ready
                    && (wake_cue
                        || (!speech.is_active() && !scenes.is_active() && !routines.ringing()))
                {
                    if let Some(cue) = cue {
                        if cue == "error_muted" {
                            let _ = audio.stop();
                        }
                        if audio.play(cue).is_ok() {
                            feedback.cue_started(cue, now_seconds);
                        }
                    } else if !feedback.acknowledging(now_seconds) {
                        let _ = audio.stop();
                    }
                }
                return serde_json::json!({"ok": true}).to_string();
            }
            Ok(None) => return serde_json::json!({"ok": true}).to_string(),
            Err(error) => return serde_json::json!({"ok": false, "error": error}).to_string(),
        }
    }
    if matches!(
        command,
        "stop" | "disable" | "character stop" | "character rest"
    ) {
        if matches!(command, "stop" | "disable" | "character stop") {
            rest.disable();
        }
        feedback.clear();
        if matches!(command, "stop" | "disable") {
            // The core command owns cancellation. Neutral must not cancel the
            // gaze run first and turn a successful Stop into "no movement".
            character.forget_gaze_for_external_stop(if command == "disable" {
                crate::expression::gaze::GazeStopCommand::Disable
            } else {
                crate::expression::gaze::GazeStopCommand::Stop
            });
            let _ = character.set_reaction("neutral", now_seconds, core);
        }
        if !speech.is_active() && !scenes.is_active() {
            let _ = audio.stop();
        }
    }
    if let Some(value) = command.strip_prefix("lamp-effect ") {
        let result = serde_json::from_str::<crate::lamp::LampPatch>(value)
            .map_err(|e| e.to_string())
            .and_then(|patch| {
                routines
                    .owner_lamp(manual_light)
                    .cloned()
                    .unwrap_or_default()
                    .updated_at(patch, now_seconds)
                    .map_err(|e| e.to_string())
            });
        return match result {
            Ok(program) => {
                let prior_mode = routines.mode();
                if let Err(error) = routines.stop_light(wall, now_seconds, manual_light, audio) {
                    return serde_json::json!({"ok":false,"error":error}).to_string();
                }
                if prior_mode != routines.mode() {
                    rest.set_mode(routines.mode(), now_seconds);
                }
                *manual_light = Some(program);
                serde_json::json!({"ok":true,"command":"lamp_effect"}).to_string()
            }
            Err(error) => serde_json::json!({"ok":false,"error":error}).to_string(),
        };
    }
    if let Some(values) = command.strip_prefix("lamp ") {
        let channels: Vec<_> = values.split_whitespace().collect();
        let result = if channels.len() == 4 {
            parse_rgbw(&mut channels.iter().map(|value| value.to_string()), "lamp")
        } else {
            Err(crate::Error::InvalidArgument(
                "Expected lamp R G B W".into(),
            ))
        };
        return match result {
            Ok(color) => {
                let prior_mode = routines.mode();
                if let Err(error) = routines.stop_light(wall, now_seconds, manual_light, audio) {
                    return serde_json::json!({"ok":false,"error":error}).to_string();
                }
                if prior_mode != routines.mode() {
                    rest.set_mode(routines.mode(), now_seconds);
                }
                let maximum = [color.red, color.green, color.blue, color.white]
                    .into_iter()
                    .max()
                    .unwrap();
                let base = manual_light.clone().unwrap_or_default();
                let patch = if maximum == 0 {
                    serde_json::json!({"on":false})
                } else {
                    let normalized = [color.red, color.green, color.blue, color.white]
                        .map(|c| (f64::from(c) * 255. / f64::from(maximum)).round() as u8);
                    serde_json::json!({"on":true,"brightness":f64::from(maximum)/255.,"effect":"solid","colors":[normalized]})
                };
                *manual_light = Some(
                    base.updated_at(serde_json::from_value(patch).unwrap(), now_seconds)
                        .expect("validated lamp channels"),
                );
                serde_json::json!({"ok": true, "command": "lamp"}).to_string()
            }
            Err(error) => serde_json::json!({"ok": false, "error": error.to_string()}).to_string(),
        };
    }
    if command == "character rest" {
        let result = (|| -> crate::Result<serde_json::Value> {
            if scenes.is_active() {
                scenes.cancel(now_seconds, core, audio)?;
            }
            if speech.is_active() {
                speech.cancel(audio)?;
            }
            rest.begin_rest(now_seconds, core, character)
        })();
        return match result {
            Ok(value) => value.to_string(),
            Err(error) => serde_json::json!({"ok":false,"error":error.to_string()}).to_string(),
        };
    }
    let (speech_command, cue_value) = speech_cue_argument(command);
    let fields: Vec<_> = speech_command.split_whitespace().collect();
    let scoped_speech =
        fields.len() == 4 && fields[0] == "speech" && matches!(fields[1], "stream" | "file");
    if scoped_speech && !feedback.owns(fields[3]) {
        return serde_json::json!({"ok": false, "error": "Stale voice session"}).to_string();
    }
    if rest.failed()
        && (command.starts_with("speech file ") || command.starts_with("speech stream "))
    {
        return serde_json::json!({"ok": false, "error": "Recover character startup before speech playback."}).to_string();
    }
    if rest.transitioning()
        && (command.starts_with("goto ")
            || command.starts_with("play ")
            || command.starts_with("scene start ")
            || command.starts_with("scene preview "))
    {
        return serde_json::json!({"ok": false, "error": "Start character mode or release movement before explicit motion."}).to_string();
    }
    let mut normalized = if scoped_speech {
        fields[..3].join(" ")
    } else {
        speech_command.to_owned()
    };
    if let Some(value) = cue_value {
        normalized.push_str(&format!(" cues={value}"));
    }
    if command == "character start"
        && !character.status().enabled
        && let Err(error) = routines.stop_light(wall, now_seconds, manual_light, audio)
    {
        return serde_json::json!({"ok":false,"error":error}).to_string();
    }
    let response = handle_daemon_command_with_character(
        &normalized,
        now_seconds,
        core,
        scenes,
        speech,
        character,
        audio,
        Some(asset_reload),
    );
    if scoped_speech {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response) {
            if value["ok"] == true {
                *voice_speech_run = value["run_id"].as_u64().map(|run| (run, fields[3].into()));
                let _ = feedback.event(fields[3], "first_chunk", now_seconds);
            }
        }
    }
    if command == "character start"
        && serde_json::from_str::<serde_json::Value>(&response)
            .is_ok_and(|value| value["ok"] == true)
    {
        rest.started();
        manual_light.get_or_insert_with(LampProgram::default);
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response) {
        if value["ok"] == true {
            if command == "character rest" {
                if let Err(error) = rest.track_rest(&value) {
                    return serde_json::json!({"ok": false, "error": error.to_string()})
                        .to_string();
                }
            } else if matches!(command, "enable" | "configure") && rest.transitioning() {
                rest.disable();
            }
        }
    }
    if matches!(command, "status" | "character status") {
        let mut value: serde_json::Value = serde_json::from_str(&response).expect("runtime status");
        value["lamp"] = owner_lamp_status(
            manual_light,
            routines,
            character,
            scenes,
            speech,
            feedback,
            rest,
            now_seconds,
        );
        return value.to_string();
    }
    response
}

fn owner_lamp_status(
    manual: &Option<LampProgram>,
    routines: &crate::expression::routines::Routines,
    character: &CharacterCoordinator,
    scenes: &SceneCoordinator,
    speech: &SpeechCoordinator,
    feedback: &VoiceFeedback,
    rest: &crate::expression::rest::RestCoordinator,
    now: f64,
) -> serde_json::Value {
    let takeover = if rest.dark() {
        Some("rest")
    } else if scenes.is_active() {
        Some("scene")
    } else if speech.is_active() {
        Some("speech")
    } else if feedback.light(now).is_some() {
        Some("voice")
    } else if routines.curiosity_quiet() {
        Some("routine")
    } else if manual.is_none() && character.status().enabled {
        Some("idle")
    } else {
        None
    };
    static FALLBACK: std::sync::OnceLock<LampProgram> = std::sync::OnceLock::new();
    let fallback = FALLBACK.get_or_init(LampProgram::default);
    let mut status = routines
        .owner_lamp(manual)
        .unwrap_or(fallback)
        .owner_status_at(now, takeover);
    if routines.owner_lamp(manual).is_none() && !character.status().enabled {
        status["on"] = serde_json::json!(false);
    }
    status
}
