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
    configure_xvf3800_mixer,
};

use super::OBSERVE_PERIOD;

pub(super) fn routines_path(backend: Backend, home: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    if backend == Backend::Hardware {
        home.map(|home| PathBuf::from(home).join(".config/orion/routines.json"))
    } else {
        None
    }
}

pub(super) fn render_light(options: &Options, pixel: Option<usize>) -> crate::Result<i32> {
    let mut device = Pi5NeoPixelDevice::open(
        &options.lighting_device,
        options.hardware.profile().lighting.pixel_count,
    )?;
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

pub(super) fn servo_profiles(options: &Options) -> crate::ServoProfiles {
    let mut profiles = if options.hardware == crate::HardwareVersion::V1 {
        crate::make_orion_servo_profiles()
    } else {
        crate::make_orion_v2_servo_profiles()
    };
    if let Some(selection) = &options.servo_gain_override {
        let profile = profiles
            .get_mut(&selection.joint)
            .expect("validated arm joint");
        profile.p_coefficient = selection.p;
        profile.i_coefficient = selection.i;
    }
    profiles
}

pub(super) fn connect_driver(options: &Options) -> crate::Result<Sts3215Driver<RustypotTransport>> {
    let calibrations = options
        .hardware
        .load_calibration(&options.calibration_file)?;
    let profiles = servo_profiles(options);
    if let Some(selection) = &options.servo_gain_override {
        eprintln!(
            "{}",
            serde_json::json!({
                "event": "servo.experimental_gains_selected", "joint": selection.joint,
                "p": selection.p, "i": selection.i, "d": profiles[&selection.joint].d_coefficient,
                "persistent_eeprom": true,
            })
        );
    }
    let mut driver = Sts3215Driver::with_profiles(RustypotTransport::default(), profiles);
    if options.operation == super::Operation::Serve {
        driver.connect_daemon(&options.port, options.baud_rate, calibrations)?;
    } else {
        driver.connect(&options.port, options.baud_rate, calibrations)?;
    }
    Ok(driver)
}

fn configure_mixer(options: &Options) -> crate::Result<()> {
    match options.hardware {
        crate::HardwareVersion::V1 => configure_respeaker_v2_mixer(&options.audio_card),
        crate::HardwareVersion::V2 => configure_xvf3800_mixer(&options.audio_card),
    }
}

fn volume_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/orion/audio.json"))
}

pub(super) fn play_cue(options: &Options) -> crate::Result<i32> {
    let cues = CueLibrary::load(&options.audio_cues_directory)?;
    configure_mixer(options)?;
    let mut audio = AlsaAudioDevice::new(
        cues,
        &options.audio_pcm_device,
        PathBuf::from(ORION_APLAY_PATH),
    )?
    .with_volume_path(volume_path())?;
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
                options.hardware.profile().lighting.pixel_count,
            )?);
            configure_mixer(&options)?;
            let audio = Box::new(
                AlsaAudioDevice::new(
                    cues,
                    &options.audio_pcm_device,
                    PathBuf::from(ORION_APLAY_PATH),
                )?
                .with_volume_path(volume_path())?
                .with_music_socket(
                    std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| PathBuf::from("/tmp"))
                        .join(".local/share/orion/music.sock"),
                )?,
            );
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
            let driver = MujocoDriver::launch_for_hardware(
                &options.python,
                bridge,
                &options.scene_file,
                start,
                options.hardware,
            )?;
            let lighting = Box::new(RecordingLightingDevice::new(
                options.hardware.profile().lighting.pixel_count,
            )?);
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
    if let Some(config) = options.hardware.profile().comfort_box.clone() {
        core.configure_comfort_box(config)?;
    }
    let camera_error = if options.hardware == crate::HardwareVersion::V2 {
        // Deployed geometry belongs to the release, independently of the live
        // catalog working directory. Missing geometry never blocks startup.
        crate::control::camera_rays::SystemBoottimeClock::new()
            .and_then(|clock| core.configure_camera_geometry(&options.camera_urdf, Box::new(clock)))
            .err()
            .map(|error| {
                eprintln!("oriond: camera-rays disabled: {error}");
                error.to_string()
            })
    } else {
        None
    };
    if let Some(path) = &options.tracking_log {
        core.enable_tracking(crate::control::tracking::TrackingTelemetry::create(
            path, backend,
        )?)?;
    }
    let mut lighting = crate::expression::rest::RestLighting::new(lighting);
    let mut rest = crate::expression::rest::RestCoordinator::new(options.rest_after_seconds);
    let wall_time = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    };
    let routines_path = routines_path(options.backend, std::env::var_os("HOME").as_deref());
    let mut routines = crate::expression::routines::Routines::load(routines_path, wall_time(), 0.0)
        .map_err(crate::Error::Runtime)?;
    lighting.clear()?;
    let mut scenes = SceneCoordinator::new(scene_library, Rgbw8::OFF);
    let mut speech = SpeechCoordinator::new(DEFAULT_SPEECH_SPOOL_PATH);
    let behaviour_writer = (options.backend == Backend::Hardware
        || std::env::var_os("ORION_VOICE_HISTORY_DIR").is_some())
    .then(orion_history::directory)
    .and_then(Result::ok)
    .and_then(|root| {
        orion_history::BehaviourWriter::start(root)
            .map_err(|error| eprintln!("oriond: behaviour log unavailable: {error}"))
            .ok()
    });
    let behaviour_log = behaviour_writer
        .as_ref()
        .map(crate::expression::behaviour::BehaviourLog::new);
    let mut character = CharacterCoordinator::new(0x4f52_494f_4e);
    let mind_path = (options.backend == Backend::Hardware)
        .then(|| std::env::var_os("HOME"))
        .flatten()
        .map(|home| PathBuf::from(home).join(".local/share/orion/mind.json"));
    let settings_path = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".config/orion/behaviours.json"));
    let (mind_store, mind) = crate::expression::mind::MindStore::start(mind_path, settings_path);
    character.mind = mind;
    let mut mind_snapshot_at = 0.;
    let mut mind_speech_run = None;
    let mut mind_cue_index = 0;
    character.behaviour_log = behaviour_log.clone();
    character.tracking_file = (options.backend == Backend::Hardware)
        .then(|| std::env::var_os("HOME"))
        .flatten()
        .map(|home| PathBuf::from(home).join(".config/orion/tracking.json"));
    character.set_gaze_enabled(options.gaze_on);
    if options.gaze_on
        && let Some(error) = camera_error
    {
        character.set_gaze_unavailable(error);
    }
    let mut feedback = crate::voice_feedback::VoiceFeedback::default();
    let mut act_sounds = crate::expression::act_sound::ActSounds::default();
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
    rest.set_mode(routines.mode(), 0.0);
    let mut next_sample = started_at;
    let mut speaking_light_intensity = 0.0;
    let mut manual_light = character
        .status()
        .enabled
        .then(crate::lamp::LampProgram::default);
    let mut light_mode = routines.mode();
    let mut camera_viewer = super::camera_viewer::CameraViewer::default();
    let mut loop_stats = super::loop_stats::LoopStats::default();
    while !stopping.load(Ordering::Relaxed) {
        let begun = Instant::now();
        let delay_ms = begun.saturating_duration_since(next_sample).as_secs_f64() * 1000.0;
        next_sample += OBSERVE_PERIOD;
        let now_seconds = started_at.elapsed().as_secs_f64();
        core.tick(now_seconds)?;
        if core.take_bus_reconnected() {
            loop_stats.bus_reconnected();
            character.resume_after_bus_reconnect(now_seconds, &mut core);
        }
        if let Some(log) = &behaviour_log {
            log.observe_motion(&core.snapshot(), now_seconds);
        }
        match routines.due(wall_time(), now_seconds, audio.as_mut()) {
            Ok(true) => {
                if scenes.is_active() {
                    scenes.cancel(now_seconds, &mut core, audio.as_mut())?;
                }
                if speech.is_active() {
                    let _ = speech.cancel(audio.as_mut());
                }
                feedback.clear();
                let _ = character.set_reaction("neutral", now_seconds, &mut core);
                let _ = audio.stop();
            }
            Err(error) => {
                if routines.error.as_ref() != Some(&error) {
                    eprintln!("oriond: alert scheduling failed: {error}");
                }
                routines.error = Some(error);
            }
            _ => {}
        }
        let wind_down_complete = match routines.light_tick(
            wall_time(),
            now_seconds,
            &mut manual_light,
            !scenes.is_active() && !speech.is_active(),
        ) {
            Ok(done) => done,
            Err(error) => {
                routines.error = Some(error);
                false
            }
        };
        if routines.mode() != light_mode {
            light_mode = routines.mode();
            rest.set_mode(light_mode, now_seconds);
        }
        if wind_down_complete {
            rest.request_sleep("light_routine");
        }
        if let Err(error) = routines.tick_audio(now_seconds, audio.as_mut()) {
            eprintln!("oriond: {error}");
        }
        lighting.set_camera_viewer(
            camera_viewer
                .pixel(now_seconds)
                .filter(|_| rest.reactions_ready()),
        );
        lighting.set_wake_pulse(if rest.dark() && rest.wake_acknowledgment_ready() {
            feedback.candidate_pulse(now_seconds)
        } else {
            None
        });
        lighting.update(rest.dark() && !routines.sunrise_active(), now_seconds)?;
        scenes.tick(now_seconds, &mut core, &mut lighting, audio.as_mut())?;
        if !routines.ringing() && !scenes.is_active() && !speech.is_active() {
            let _ = audio.update();
        }
        // Expire ownership before queued audio can start on this tick.
        let had_confirmed_feedback = feedback.confirmed_activity();
        if feedback.expire(now_seconds) && had_confirmed_feedback {
            let _ = character.set_reaction("neutral", now_seconds, &mut core);
        }
        audio.set_music_ducking(if feedback.capturing() {
            0.10
        } else if feedback.session_active() || speech.is_active() || routines.ringing() {
            0.25
        } else {
            1.
        });
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
            !routines.ringing()
                && (rest.speech_ready(&character)
                    || feedback.announcement() && rest.wake_acknowledgment_ready())
                && (!scoped_voice || feedback.playback_authorized()),
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
        if current_playing != mind_speech_run {
            mind_speech_run = current_playing;
            mind_cue_index = 0;
        }
        if current_playing.is_some()
            && let (Some(analysis), Some(frame)) =
                (speech.active_analysis(), speech.active_energy_frame())
        {
            while let Some((at, cue)) = analysis.cues.get(mind_cue_index) {
                if *at > frame {
                    break;
                }
                character.mind.cue(*cue);
                mind_cue_index += 1;
            }
        }
        character.mind.advance(wall_time(), mind_store.hour());
        character.mind.attend(character.mind_attention());
        if now_seconds >= mind_snapshot_at {
            mind_store.snapshot(&character.mind);
            mind_snapshot_at = now_seconds + 1.;
        }
        if let Some(energy) = speech.active_energy() {
            speaking_light_intensity = smooth_speaking_light(speaking_light_intensity, energy);
            lighting.render_effect("speaking_energy", now_seconds, speaking_light_intensity)?;
        } else {
            speaking_light_intensity = 0.0;
        }
        let character_was_enabled = character.status().enabled;
        if let Some(settings) = mind_store.settings() {
            character.set_act_settings(settings);
        }
        let rest_status = rest.status(now_seconds);
        character.set_music_input(
            audio.music_pulse(),
            rest.reactions_ready()
                && !rest_status.sleep_requested
                && !feedback.session_active()
                && !speech.is_active()
                && !scenes.is_active()
                && !routines.ringing()
                && !routines.curiosity_quiet(),
        );
        character.set_act_context(
            rest_status.state == crate::expression::rest::RestState::Awake
                && !rest_status.sleep_requested
                && !feedback.session_active()
                && !speech.is_active()
                && !scenes.is_active()
                && !routines.curiosity_quiet(),
            rest_status
                .remaining_seconds
                .map(|remaining| now_seconds + remaining),
        );
        character.set_curiosity_available(
            rest.reactions_ready()
                && !feedback.session_active()
                && !speech.is_active()
                && !scenes.is_active()
                && !routines.curiosity_quiet(),
        );
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
        if character.take_goodnight_rest(now_seconds) {
            rest.rest_soon(now_seconds);
        }
        rest.tick(
            now_seconds,
            &mut core,
            &mut character,
            &feedback,
            routines.ringing() || scenes.is_active() || speech.is_active(),
        );
        act_sounds.tick(
            &core.snapshot(),
            crate::expression::act_sound::SoundContext {
                enabled: character.act_sounds_enabled(),
                speech: speech.is_active(),
                alert: routines.ringing(),
                quiet_routine: routines.curiosity_quiet(),
                hour: mind_store.hour(),
            },
            audio.as_mut(),
        );
        if let Some(log) = &behaviour_log {
            log.observe_state(
                &character.gaze_status(),
                &rest.status(now_seconds),
                character.held_gaze_track(),
                now_seconds,
            );
        }
        // Rest can change on this tick. Sunrise may bypass darkness, but must
        // never expose a stale viewer lease during rest or waking.
        lighting.set_camera_viewer(
            camera_viewer
                .pixel(now_seconds)
                .filter(|_| rest.reactions_ready()),
        );
        lighting.set_wake_pulse(if rest.dark() && rest.wake_acknowledgment_ready() {
            feedback.candidate_pulse(now_seconds)
        } else {
            None
        });
        lighting.update(rest.dark() && !routines.sunrise_active(), now_seconds)?;
        if character_just_stopped(character_was_enabled, character.status().enabled) {
            lighting.clear()?;
            speaking_light_intensity = 0.0;
        }
        if !scenes.is_active()
            && current_playing.is_none()
            && manual_light.is_none()
            && let Some(effect) = character.background_lighting_effect(&core)
        {
            let frame = character.background_frame(
                &effect,
                now_seconds,
                lighting.pixel_count(),
                character.curiosity_light_intensity(now_seconds),
                &core,
            )?;
            lighting.render(&frame)?;
        }
        if !scenes.is_active() && !speech.is_active() {
            if let Some(program) = &manual_light {
                let mut frame = program.render_for_pixels(now_seconds, lighting.pixel_count())?;
                let gain = character.sleep_light_gain(&core);
                for color in &mut frame {
                    color.red = (color.red as f64 * gain) as u8;
                    color.green = (color.green as f64 * gain) as u8;
                    color.blue = (color.blue as f64 * gain) as u8;
                    color.white = (color.white as f64 * gain) as u8;
                }
                lighting.render(&frame)?;
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
            if let Some(response) = camera_viewer.command(
                command,
                now_seconds,
                options.hardware == crate::HardwareVersion::V2,
            ) {
                return response;
            }
            let response = dispatch_command(
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
                &mut routines,
            );
            if command == "status" {
                let mut value: serde_json::Value =
                    serde_json::from_str(&response).expect("runtime status JSON");
                value["hardware"] = serde_json::json!(options.hardware.profile().hardware);
                value["mind"] = serde_json::json!(character.mind);
                value["physical_joints"] = serde_json::json!(
                    options
                        .hardware
                        .profile()
                        .joints
                        .iter()
                        .map(|(name, joint)| (name, &joint.physical_joint))
                        .collect::<std::collections::BTreeMap<_, _>>()
                );
                value["lighting_pixels"] = serde_json::json!(lighting.pixel_count());
                value["loop"] = loop_stats.status();
                value.to_string()
            } else {
                response
            }
        })?;
        rest.render_failure_light(&mut lighting, now_seconds)?;
        rest.light_on = lighting.is_on();
        loop_stats.record(
            started_at.elapsed().as_secs(),
            delay_ms,
            begun.elapsed().as_secs_f64() * 1000.0,
        );
        thread::sleep(next_sample.saturating_duration_since(Instant::now()));
    }
    mind_store.shutdown(&character.mind);
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
