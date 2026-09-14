use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use super::commands::{AssetReloadContext, dispatch_command};
use super::options::{Backend, Options};
use crate::{
    AlsaAudioDevice, AudioDevice, CharacterCoordinator, CueLibrary, DEFAULT_SPEECH_SPOOL_PATH,
    LightingDevice, MotionLibrary, MujocoDriver, ORION_APLAY_PATH, ORION_JOINT_NAMES,
    Pi5NeoPixelDevice, PoseLibrary, RecordingAudioDevice, RecordingLightingDevice, Rgbw8,
    RuntimeCore, RuntimeDriver, RustypotTransport, SceneCoordinator, SceneLibrary, ScenePhase,
    SpeechCoordinator, Sts3215Driver, UnixCommandServer, configure_respeaker_v2_mixer,
    load_calibration_file, render_effect,
};

use super::OBSERVE_PERIOD;

pub(super) fn render_light(options: &Options, pixel: Option<usize>) -> crate::Result<i32> {
    let mut device =
        Pi5NeoPixelDevice::open(&options.lighting_device, crate::ORION_LIGHT_PIXEL_COUNT)?;
    if let Some(index) = pixel {
        let mut frame = vec![Rgbw8::OFF; device.pixel_count()];
        frame[index] = options.light_color;
        device.render(&frame)?;
    } else {
        device.render_uniform(options.light_color)?;
    }
    println!(
        "{{\"ok\":true,\"command\":\"light\",\"red\":{},\"green\":{},\"blue\":{},\"white\":{},\"pixel\":{}}}",
        options.light_color.red,
        options.light_color.green,
        options.light_color.blue,
        options.light_color.white,
        pixel.map_or_else(|| "null".to_owned(), |value| value.to_string())
    );
    Ok(0)
}

pub(super) fn connect_driver(options: &Options) -> crate::Result<Sts3215Driver<RustypotTransport>> {
    let calibrations = load_calibration_file(&options.calibration_file, &ORION_JOINT_NAMES)?;
    let mut driver = Sts3215Driver::new(RustypotTransport::default());
    driver.connect(&options.port, options.baud_rate, calibrations)?;
    Ok(driver)
}

pub(super) fn play_cue(options: &Options) -> crate::Result<i32> {
    let cues = CueLibrary::load(&options.audio_cues_directory)?;
    configure_respeaker_v2_mixer(&options.audio_card)?;
    let mut audio = AlsaAudioDevice::new(
        cues,
        &options.audio_pcm_device,
        PathBuf::from(ORION_APLAY_PATH),
    )?;
    audio.play(&options.cue_name)?;
    while audio.is_playing() {
        thread::sleep(Duration::from_millis(10));
        audio.update()?;
    }
    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "command": "play_cue",
            "cue": options.cue_name,
            "state": "completed",
        })
    );
    Ok(0)
}

pub(super) fn serve(options: Options) -> crate::Result<i32> {
    let poses = PoseLibrary::load_with_user_directory(
        &options.poses_file,
        &options.user_poses_directory,
        &ORION_JOINT_NAMES,
    )?;
    let motions = MotionLibrary::load(&options.motions_directory, &poses)?;
    let scenes = SceneLibrary::load(&options.scenes_directory, &poses, &motions)?;
    let cues = CueLibrary::load(&options.audio_cues_directory)?;
    scenes.validate_audio_cues(&cues)?;
    let asset_reload = AssetReloadContext {
        poses_file: options.poses_file.clone(),
        user_poses_directory: options.user_poses_directory.clone(),
        motions_directory: options.motions_directory.clone(),
        scenes_directory: options.scenes_directory.clone(),
        cues: cues.clone(),
    };
    match options.backend {
        Backend::Hardware => {
            let driver = connect_driver(&options)?;
            let lighting = Box::new(Pi5NeoPixelDevice::open(
                &options.lighting_device,
                crate::ORION_LIGHT_PIXEL_COUNT,
            )?);
            configure_respeaker_v2_mixer(&options.audio_card)?;
            let audio = Box::new(AlsaAudioDevice::new(
                cues,
                &options.audio_pcm_device,
                PathBuf::from(ORION_APLAY_PATH),
            )?);
            serve_driver(
                driver,
                poses,
                motions,
                scenes,
                asset_reload,
                lighting,
                audio,
                &options,
                "hardware",
            )
        }
        Backend::Mujoco => {
            let start = poses.pose(&options.start_pose)?;
            let bridge = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("mujoco_bridge.py");
            let driver = MujocoDriver::launch(&options.python, bridge, &options.scene_file, start)?;
            let lighting = Box::new(RecordingLightingDevice::orion());
            let audio = Box::new(RecordingAudioDevice::default());
            serve_driver(
                driver,
                poses,
                motions,
                scenes,
                asset_reload,
                lighting,
                audio,
                &options,
                "mujoco",
            )
        }
    }
}

pub(super) fn serve_driver<D: RuntimeDriver>(
    driver: D,
    poses: PoseLibrary,
    motions: MotionLibrary,
    scene_library: SceneLibrary,
    asset_reload: AssetReloadContext,
    lighting: Box<dyn LightingDevice>,
    mut audio: Box<dyn AudioDevice>,
    options: &Options,
    backend: &str,
) -> crate::Result<i32> {
    let mut core = RuntimeCore::new(driver, poses, motions)?;
    let mut lighting = crate::expression::rest::RestLighting::new(lighting);
    let mut rest = crate::expression::rest::RestCoordinator::new(options.rest_after_seconds);
    lighting.clear()?;
    let mut scenes = SceneCoordinator::new(scene_library, Rgbw8::OFF);
    let mut speech = SpeechCoordinator::new(DEFAULT_SPEECH_SPOOL_PATH);
    let mut character = CharacterCoordinator::new(0x4f52_494f_4e);
    let mut feedback = crate::voice_feedback::VoiceFeedback::default();
    let mut playing_run = None;
    let mut voice_speech_run: Option<(u64, String)> = None;
    let mut feedback_was_lit = false;
    let mut server = UnixCommandServer::bind(&options.socket_path)?;
    let stopping = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&stopping))?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&stopping))?;

    println!(
        "oriond: serving {backend} at 50 Hz on {}",
        options.socket_path.display()
    );
    let started_at = Instant::now();
    if options.character_on_start {
        if let Err(error) = character.start(0.0, &mut core) {
            eprintln!(
                "oriond: character startup failed; remaining available for recovery: {error}"
            );
        } else {
            rest.started();
        }
    }
    let mut next_sample = started_at;
    let mut speaking_light_intensity = 0.0;
    let mut manual_light: Option<crate::lamp::LampProgram> = None;
    while !stopping.load(Ordering::Relaxed) {
        next_sample += OBSERVE_PERIOD;
        let now_seconds = started_at.elapsed().as_secs_f64();
        core.tick(now_seconds)?;
        lighting.update(rest.dark(), now_seconds)?;
        scenes.tick(now_seconds, &mut core, &mut lighting, audio.as_mut())?;
        if !scenes.is_active() && !speech.is_active() {
            let _ = audio.update();
        }
        // Expire ownership before queued audio can start on this tick.
        if feedback.expire(now_seconds) {
            let _ = character.set_reaction("neutral", now_seconds, &mut core);
        }
        if rest.failed()
            || voice_speech_run.as_ref().is_some_and(|(run, session)| {
                speech
                    .active_status()
                    .is_some_and(|active| active.run_id == *run)
                    && !feedback.owns(session)
            })
        {
            let _ = speech.cancel(audio.as_mut());
        }
        let scoped_voice = speech.active_status().is_some_and(|active| {
            voice_speech_run
                .as_ref()
                .is_some_and(|(run, _)| *run == active.run_id)
        });
        speech.tick_when_ready(
            audio.as_mut(),
            rest.speech_ready(&character) && (!scoped_voice || feedback.confirmed_activity()),
        );
        let current_playing = speech
            .active_status()
            .filter(|status| status.state == crate::speech::SpeechPhase::Playing)
            .map(|status| status.run_id);
        if current_playing.is_some() && current_playing != playing_run {
            if current_playing == voice_speech_run.as_ref().map(|(run, _)| *run) {
                feedback.playback_started(now_seconds);
            }
            character.note_speech_started(now_seconds);
        }
        playing_run = current_playing;
        if let Some(energy) = speech.active_energy() {
            speaking_light_intensity = smooth_speaking_light(speaking_light_intensity, energy);
            lighting.render(&render_effect(
                "speaking_energy",
                now_seconds,
                speaking_light_intensity,
            )?)?;
        } else {
            speaking_light_intensity = 0.0;
        }
        let character_was_enabled = character.status().enabled;
        character.tick(
            now_seconds,
            &mut core,
            scenes.is_active(),
            scenes
                .last_status()
                .map(|status| (status.run_id, status.state == ScenePhase::Completed)),
            current_playing.is_some(),
            speech.active_analysis(),
            speech.active_energy_frame(),
        )?;
        rest.tick(
            now_seconds,
            &mut core,
            &mut character,
            &feedback,
            scenes.is_active() || speech.is_active(),
        );
        lighting.update(rest.dark(), now_seconds)?;
        if character_just_stopped(character_was_enabled, character.status().enabled) {
            lighting.clear()?;
            speaking_light_intensity = 0.0;
        }
        if !scenes.is_active()
            && current_playing.is_none()
            && manual_light.is_none()
            && let Some(effect) = character.background_lighting_effect(&core)
        {
            lighting.render(&render_effect(&effect, now_seconds, 0.55)?)?;
        }
        if !scenes.is_active() && !speech.is_active() {
            if let Some(program) = &manual_light {
                lighting.render(&program.render(now_seconds)?)?;
            }
        }
        if feedback_was_lit
            && feedback.light(now_seconds).is_none()
            && !scenes.is_active()
            && current_playing.is_none()
            && manual_light.is_none()
            && !character.status().enabled
        {
            lighting.clear()?;
        }
        feedback_was_lit = feedback.light(now_seconds).is_some();
        if !scenes.is_active() && current_playing.is_none() {
            if let Some(color) = feedback.light(now_seconds) {
                lighting.render_uniform(color)?;
            }
        }
        server.serve_pending(|command| {
            dispatch_command(
                command,
                now_seconds,
                &mut core,
                &mut scenes,
                &mut speech,
                &mut character,
                audio.as_mut(),
                &asset_reload,
                &mut feedback,
                &mut manual_light,
                &mut voice_speech_run,
                &mut rest,
            )
        })?;
        rest.light_on = lighting.is_on();
        thread::sleep(next_sample.saturating_duration_since(Instant::now()));
    }
    Ok(0)
}

pub(super) fn smooth_speaking_light(previous: f64, energy: f64) -> f64 {
    let target = (energy / 0.30).clamp(0.06, 0.72);
    let response = if target > previous { 0.18 } else { 0.07 };
    previous + (target - previous) * response
}

pub(super) fn character_just_stopped(was_enabled: bool, is_enabled: bool) -> bool {
    was_enabled && !is_enabled
}
