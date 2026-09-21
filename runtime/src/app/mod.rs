mod client;
mod commands;
mod options;
mod server;

use std::env;

use crate::request_daemon;
use client::{error_exit_code, print_response, print_states, request_movement, request_scene};
use options::{Operation, parse_options, usage};
use server::{connect_driver, play_cue, render_light, serve};

const OBSERVE_PERIOD: std::time::Duration = std::time::Duration::from_millis(20);

pub fn run() -> i32 {
    match execute() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("oriond: {error}");
            error_exit_code(&error)
        }
    }
}

fn execute() -> crate::Result<i32> {
    let options = parse_options(env::args().skip(1))?;
    if options.help {
        print!("{}", usage());
        return Ok(0);
    }
    match options.operation {
        Operation::Check => {
            let mut driver = connect_driver(&options)?;
            print_states(&driver.read()?);
            Ok(0)
        }
        Operation::Serve => serve(options),
        Operation::Status => print_response(request_daemon(&options.socket_path, "status")?),
        Operation::Configure => print_response(request_daemon(&options.socket_path, "configure")?),
        Operation::Enable => print_response(request_daemon(&options.socket_path, "enable")?),
        Operation::Disable => print_response(request_daemon(&options.socket_path, "disable")?),
        Operation::PlayCue => play_cue(&options),
        Operation::Goto => request_movement(
            &options,
            &format!("goto {} {:.6}", options.pose_name, options.duration_seconds),
        ),
        Operation::Play => request_movement(&options, &format!("play {}", options.motion_name)),
        Operation::Stop => print_response(request_daemon(&options.socket_path, "stop")?),
        Operation::Light => render_light(&options, None),
        Operation::LightPixel => render_light(&options, Some(options.light_pixel)),
        Operation::LightsOff => render_light(&options, None),
        Operation::RunScene => request_scene(&options),
        Operation::SceneStatus => {
            print_response(request_daemon(&options.socket_path, "scene status")?)
        }
        Operation::StopScene => print_response(request_daemon(&options.socket_path, "scene stop")?),
        Operation::SpeechStatus => {
            print_response(request_daemon(&options.socket_path, "speech status")?)
        }
        Operation::StopSpeech => {
            print_response(request_daemon(&options.socket_path, "speech stop")?)
        }
        Operation::None => {
            eprint!("{}", usage());
            Ok(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{client::*, commands::*, options::*, server::*};
    use crate::SpeechPhase;
    use crate::expression::{rest::RestCoordinator, voice_feedback::VoiceFeedback};
    use crate::*;
    use crate::{
        AudioCommand, JointPositions, JointState, RecordingAudioDevice, UnavailableAudioDevice,
    };
    use std::path::PathBuf;

    use super::*;

    struct TestDriver;

    impl TestDriver {
        fn states() -> Vec<JointState> {
            ORION_JOINT_NAMES
                .iter()
                .map(|name| JointState {
                    name: (*name).to_owned(),
                    position: 0.0,
                    velocity: 0.0,
                    current_ma: 0.0,
                    voltage_v: 5.0,
                    temperature_c: 25.0,
                    status: 0,
                })
                .collect()
        }
    }

    impl RuntimeDriver for TestDriver {
        fn apply_servo_profile(&mut self) -> crate::Result<()> {
            Ok(())
        }

        fn activate(&mut self) -> crate::Result<Vec<JointState>> {
            Ok(Self::states())
        }

        fn deactivate(&mut self) -> crate::Result<()> {
            Ok(())
        }

        fn read(&mut self) -> crate::Result<Vec<JointState>> {
            Ok(Self::states())
        }

        fn write(&mut self, _positions_radians: &JointPositions) -> crate::Result<()> {
            Ok(())
        }

        fn joint_limits(&self) -> crate::Result<Vec<crate::JointLimit>> {
            Ok(ORION_JOINT_NAMES
                .iter()
                .map(|name| crate::JointLimit {
                    name: (*name).to_owned(),
                    lower_rad: -3.0,
                    upper_rad: 3.0,
                })
                .collect())
        }

        fn validate_positions(&self, _positions_radians: &JointPositions) -> crate::Result<()> {
            Ok(())
        }

        fn clamp_positions_to_safe_range(
            &self,
            positions_radians: &JointPositions,
        ) -> crate::Result<JointPositions> {
            Ok(positions_radians.clone())
        }
    }

    fn parse(values: &[&str]) -> crate::Result<Options> {
        parse_options(values.iter().map(|value| (*value).to_owned()))
    }

    fn write_test_wav(path: &std::path::Path) {
        let mut bytes = Vec::from(&b"RIFF"[..]);
        bytes.extend_from_slice(&38_u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&24_000_u32.to_le_bytes());
        bytes.extend_from_slice(&48_000_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&0_i16.to_le_bytes());
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn rest_timeout_defaults_to_thirty_minutes_and_rejects_invalid_values() {
        assert_eq!(
            parse(&["--serve", "--backend", "mujoco"])
                .unwrap()
                .rest_after_seconds,
            1800.0
        );
        assert_eq!(
            parse(&[
                "--serve",
                "--backend",
                "mujoco",
                "--rest-after-seconds",
                "4.5"
            ])
            .unwrap()
            .rest_after_seconds,
            4.5
        );
        for value in ["0", "-1", "NaN", "inf", "hello"] {
            assert!(
                parse(&["--serve", "--rest-after-seconds", value]).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn character_startup_defaults_on_and_supports_maintenance_override() {
        assert!(
            parse(&["--serve", "--backend", "mujoco"])
                .unwrap()
                .character_on_start
        );
        assert!(
            !parse(&[
                "--serve",
                "--backend",
                "mujoco",
                "--character-on-start",
                "off"
            ])
            .unwrap()
            .character_on_start
        );
        assert!(parse(&["--serve", "--character-on-start", "maybe"]).is_err());
    }

    #[test]
    fn parses_cpp_compatible_client_commands() {
        let options = parse(&[
            "--goto",
            "home",
            "--duration",
            "1.25",
            "--socket",
            "/tmp/test.sock",
            "--wait",
        ])
        .unwrap();
        assert_eq!(options.operation, Operation::Goto);
        assert_eq!(options.pose_name, "home");
        assert_eq!(options.duration_seconds, 1.25);
        assert_eq!(options.socket_path, PathBuf::from("/tmp/test.sock"));
        assert!(options.wait);
    }

    #[test]
    fn rejects_multiple_operations_and_missing_values() {
        assert!(parse(&["--status", "--enable"]).is_err());
        assert!(parse(&["--goto"]).is_err());
        assert!(parse(&["--duration", "fast"]).is_err());
        assert!(parse(&["--status", "--wait"]).is_err());
    }

    #[test]
    fn parses_mujoco_backend_without_requiring_hardware_calibration() {
        let options = parse(&["--serve", "--backend", "mujoco", "--start-pose", "home"]).unwrap();
        assert_eq!(options.backend, Backend::Mujoco);
        assert_eq!(options.start_pose, "home");
        assert!(options.calibration_file.as_os_str().is_empty());
    }

    #[test]
    fn parses_direct_rgbw_lighting_commands() {
        let options = parse(&["--light", "1", "2", "3", "4"]).unwrap();
        assert_eq!(options.operation, Operation::Light);
        assert_eq!(options.light_color, Rgbw8::new(1, 2, 3, 4));

        let options = parse(&["--light-pixel", "39", "5", "6", "7", "8"]).unwrap();
        assert_eq!(options.operation, Operation::LightPixel);
        assert_eq!(options.light_pixel, 39);
        assert_eq!(options.light_color, Rgbw8::new(5, 6, 7, 8));

        assert!(parse(&["--light", "256", "0", "0", "0"]).is_err());
        assert!(parse(&["--light-pixel", "40", "0", "0", "0", "1"]).is_err());
    }

    #[test]
    fn parses_scene_clients_and_wait() {
        let options = parse(&[
            "--run-scene",
            "acknowledge_left",
            "--wait",
            "--socket",
            "/tmp/orion-test.sock",
        ])
        .unwrap();
        assert_eq!(options.operation, Operation::RunScene);
        assert_eq!(options.scene_name, "acknowledge_left");
        assert!(options.wait);

        assert_eq!(
            parse(&["--scene-status"]).unwrap().operation,
            Operation::SceneStatus
        );
        assert_eq!(
            parse(&["--stop-scene"]).unwrap().operation,
            Operation::StopScene
        );
        assert!(parse(&["--scene-status", "--wait"]).is_err());
    }

    #[test]
    fn parses_direct_named_audio_cue() {
        let options = parse(&[
            "--play-cue",
            "acknowledge",
            "--cues",
            "/tmp/orion-cues",
            "--audio-device",
            "plughw:CARD=test,DEV=0",
        ])
        .unwrap();
        assert_eq!(options.operation, Operation::PlayCue);
        assert_eq!(options.cue_name, "acknowledge");
        assert_eq!(
            options.audio_cues_directory,
            PathBuf::from("/tmp/orion-cues")
        );
        assert_eq!(options.audio_pcm_device, "plughw:CARD=test,DEV=0");
        assert!(parse(&["--play-cue", "acknowledge", "--wait"]).is_err());
    }

    #[test]
    fn accepts_playback_controls_and_rejects_pi_synthesis() {
        assert!(parse(&["--speak", "Hello from Orion."]).is_err());
        assert!(parse(&["--tts-socket", "/tmp/tts.sock"]).is_err());
        assert_eq!(
            parse(&["--speech-status"]).unwrap().operation,
            Operation::SpeechStatus
        );
        assert_eq!(
            parse(&["--stop-speech"]).unwrap().operation,
            Operation::StopSpeech
        );
        assert!(parse(&["--speech-status", "--wait"]).is_err());
    }

    #[test]
    fn maps_scene_terminal_states_to_exit_codes() {
        assert_eq!(scene_state_exit_code("executing").unwrap(), None);
        assert_eq!(scene_state_exit_code("completed").unwrap(), Some(0));
        assert_eq!(
            scene_state_exit_code("timed_out").unwrap(),
            Some(EXIT_MOVEMENT_TIMED_OUT)
        );
        assert_eq!(
            scene_state_exit_code("cancelled").unwrap(),
            Some(EXIT_MOVEMENT_CANCELLED)
        );
        assert_eq!(
            scene_state_exit_code("failed").unwrap(),
            Some(EXIT_SCENE_FAILED)
        );
        assert!(scene_state_exit_code("mystery").is_err());
    }

    #[test]
    fn speaking_light_smooths_attack_and_release_below_full_scale() {
        let attack = smooth_speaking_light(0.0, 0.30);
        assert!(attack > 0.0 && attack < 0.72);

        let release = smooth_speaking_light(attack, 0.0);
        assert!(release > 0.06 && release < attack);

        let sustained = (0..100).fold(0.0, |level, _| smooth_speaking_light(level, 0.30));
        assert!((sustained - 0.72).abs() < 1e-6);
    }

    #[test]
    fn only_the_character_enabled_to_off_transition_clears_its_latched_light() {
        assert!(character_just_stopped(true, false));
        assert!(!character_just_stopped(true, true));
        assert!(!character_just_stopped(false, false));
        assert!(!character_just_stopped(false, true));
    }

    #[test]
    fn daemon_scene_commands_assign_status_and_retain_cancellation() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let poses =
            PoseLibrary::load(root.join("motion/config/poses.yaml"), &ORION_JOINT_NAMES).unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        let library = SceneLibrary::load(root.join("scenes"), &poses, &motions).unwrap();
        let reload = AssetReloadContext {
            poses_file: root.join("motion/config/poses.yaml"),
            user_poses_directory: root.join("motion/user/poses"),
            motions_directory: root.join("motion/motions"),
            scenes_directory: root.join("scenes"),
            cues: CueLibrary::load(root.join("audio/cues")).unwrap(),
        };
        let mut core = RuntimeCore::new(TestDriver, poses, motions).unwrap();
        let mut scenes = SceneCoordinator::new(library, Rgbw8::OFF);
        let mut speech = SpeechCoordinator::new(DEFAULT_SPEECH_SPOOL_PATH);
        let mut audio = UnavailableAudioDevice;

        let rejected: serde_json::Value = serde_json::from_str(&handle_daemon_command(
            "speech start Hello from Orion.",
            0.0,
            &mut core,
            &mut scenes,
            &mut speech,
            &mut audio,
        ))
        .unwrap();
        assert_eq!(rejected["ok"], false);
        assert!(!speech.is_active());

        let accepted: serde_json::Value = serde_json::from_str(&handle_daemon_command(
            "scene start acknowledge_left",
            0.0,
            &mut core,
            &mut scenes,
            &mut speech,
            &mut audio,
        ))
        .unwrap();
        assert_eq!(accepted["ok"], true);
        assert_eq!(accepted["run_id"], 1);

        let busy: serde_json::Value = serde_json::from_str(&handle_daemon_command(
            "scene start acknowledge_left",
            0.0,
            &mut core,
            &mut scenes,
            &mut speech,
            &mut audio,
        ))
        .unwrap();
        assert_eq!(busy["ok"], false);

        let status: serde_json::Value = serde_json::from_str(&handle_daemon_command(
            "scene status",
            0.0,
            &mut core,
            &mut scenes,
            &mut speech,
            &mut audio,
        ))
        .unwrap();
        assert_eq!(status["scene"]["run_id"], 1);

        let stopped: serde_json::Value = serde_json::from_str(&handle_daemon_command(
            "scene stop",
            0.1,
            &mut core,
            &mut scenes,
            &mut speech,
            &mut audio,
        ))
        .unwrap();
        assert_eq!(stopped["last_scene"]["state"], "cancelled");

        let status: serde_json::Value = serde_json::from_str(&handle_daemon_command(
            "scene status",
            0.2,
            &mut core,
            &mut scenes,
            &mut speech,
            &mut audio,
        ))
        .unwrap();
        assert!(status["scene"].is_null());
        assert_eq!(status["last_scene"]["run_id"], 1);

        let reloaded: serde_json::Value = serde_json::from_str(&handle_daemon_command_with_reload(
            "scene reload",
            0.3,
            &mut core,
            &mut scenes,
            &mut speech,
            &mut audio,
            Some(&reload),
        ))
        .unwrap();
        assert_eq!(reloaded["ok"], true);
        assert!(
            reloaded["scenes"]
                .as_array()
                .is_some_and(|names| names.iter().any(|name| name == "acknowledge_left"))
        );

        let assets: serde_json::Value = serde_json::from_str(&handle_daemon_command_with_reload(
            "asset reload",
            0.4,
            &mut core,
            &mut scenes,
            &mut speech,
            &mut audio,
            Some(&reload),
        ))
        .unwrap();
        assert_eq!(assets["ok"], true);
        assert!(
            assets["poses"]
                .as_array()
                .is_some_and(|names| names.iter().any(|name| name == "home"))
        );
        assert!(
            assets["motions"]
                .as_array()
                .is_some_and(|names| names.iter().any(|name| name == "look_at_left"))
        );

        let preview_document = serde_json::json!({
            "format_version": 2,
            "scene": {
                "name": "studio_preview",
                "description": "Ephemeral test preview.",
                "motion": [],
                "lighting": [{"at": 0.0, "effect": "acknowledge_pulse", "duration": 0.1}],
                "audio": [],
                "finish": {"anchor": "final_pose", "lighting": "pose_default"},
            },
        });
        let preview_command = format!(
            "scene preview {}",
            serde_json::to_string(&preview_document).unwrap()
        );
        let preview: serde_json::Value = serde_json::from_str(&handle_daemon_command_with_reload(
            &preview_command,
            0.5,
            &mut core,
            &mut scenes,
            &mut speech,
            &mut audio,
            Some(&reload),
        ))
        .unwrap();
        assert_eq!(preview["ok"], true);
        assert_eq!(preview["command"], "scene_preview");
        assert_eq!(preview["run_id"], 2);
        assert_eq!(preview["persisted"], false);
    }

    #[test]
    fn explicit_scene_preempts_active_speech_and_cleans_its_spool() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let poses =
            PoseLibrary::load(root.join("motion/config/poses.yaml"), &ORION_JOINT_NAMES).unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        let library = SceneLibrary::load(root.join("scenes"), &poses, &motions).unwrap();
        let mut core = RuntimeCore::new(TestDriver, poses, motions).unwrap();
        let mut scenes = SceneCoordinator::new(library, Rgbw8::OFF);
        let spool = tempfile::tempdir().unwrap();
        let speech_path = spool.path().join("foreground.wav");
        write_test_wav(&speech_path);
        let mut speech = SpeechCoordinator::new(spool.path());
        speech.start_spooled("foreground").unwrap();
        let mut audio = RecordingAudioDevice::blocking();
        speech.tick(&mut audio);
        assert!(speech.is_active());

        let accepted: serde_json::Value = serde_json::from_str(&handle_daemon_command(
            "scene start acknowledge_left",
            1.0,
            &mut core,
            &mut scenes,
            &mut speech,
            &mut audio,
        ))
        .unwrap();

        assert_eq!(accepted["ok"], true);
        assert!(!speech.is_active());
        assert_eq!(speech.last_status().unwrap().state, SpeechPhase::Cancelled);
        assert!(!speech_path.exists());
        assert_eq!(audio.commands().last(), Some(&AudioCommand::Stop));
    }

    #[test]
    fn character_rest_disables_character_and_starts_calibrated_rest() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let poses =
            PoseLibrary::load(root.join("motion/config/poses.yaml"), &ORION_JOINT_NAMES).unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        let library = SceneLibrary::load(root.join("scenes"), &poses, &motions).unwrap();
        let mut core = RuntimeCore::new(TestDriver, poses, motions).unwrap();
        let mut scenes = SceneCoordinator::new(library, Rgbw8::OFF);
        let spool = tempfile::tempdir().unwrap();
        let mut speech = SpeechCoordinator::new(spool.path());
        let mut character = CharacterCoordinator::new(42);
        let mut audio = RecordingAudioDevice::blocking();
        character.start(0.0, &mut core).unwrap();
        assert!(character.status().enabled);
        write_test_wav(&spool.path().join("rest-test.wav"));
        speech.start_spooled("rest-test").unwrap();
        speech.tick(&mut audio);
        assert!(speech.is_active());
        let response: serde_json::Value =
            serde_json::from_str(&handle_daemon_command_with_character(
                "character rest",
                0.0,
                &mut core,
                &mut scenes,
                &mut speech,
                &mut character,
                &mut audio,
                None,
            ))
            .unwrap();
        assert_eq!(response["ok"], true, "{response}");
        assert!(!speech.is_active());
        assert_eq!(speech.last_status().unwrap().state, SpeechPhase::Cancelled);
        assert!(!character.status().enabled);
        assert_eq!(core.snapshot().motion.as_ref().unwrap().name, "rest");
    }

    #[test]
    fn character_stop_cancels_active_speech_and_scene_before_returning_home() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let poses =
            PoseLibrary::load(root.join("motion/config/poses.yaml"), &ORION_JOINT_NAMES).unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        let library = SceneLibrary::load(root.join("scenes"), &poses, &motions).unwrap();

        let mut core = RuntimeCore::new(TestDriver, poses.clone(), motions.clone()).unwrap();
        let mut scenes = SceneCoordinator::new(library.clone(), Rgbw8::OFF);
        let spool = tempfile::tempdir().unwrap();
        let speech_path = spool.path().join("shutdown.wav");
        write_test_wav(&speech_path);
        let mut speech = SpeechCoordinator::new(spool.path());
        let mut character = CharacterCoordinator::new(42);
        let mut audio = RecordingAudioDevice::blocking();

        let started: serde_json::Value =
            serde_json::from_str(&handle_daemon_command_with_character(
                "character start",
                0.0,
                &mut core,
                &mut scenes,
                &mut speech,
                &mut character,
                &mut audio,
                None,
            ))
            .unwrap();
        assert_eq!(started["ok"], true);
        speech.start_spooled("shutdown").unwrap();
        speech.tick(&mut audio);
        assert!(speech.is_active());

        let stopped: serde_json::Value =
            serde_json::from_str(&handle_daemon_command_with_character(
                "character stop",
                0.2,
                &mut core,
                &mut scenes,
                &mut speech,
                &mut character,
                &mut audio,
                None,
            ))
            .unwrap();
        assert_eq!(stopped["ok"], true);
        assert_eq!(stopped["character"]["state"], "shutting_down");
        assert!(!speech.is_active());
        assert_eq!(speech.last_status().unwrap().state, SpeechPhase::Cancelled);
        assert!(!speech_path.exists());
        assert!(audio.commands().contains(&AudioCommand::Stop));

        let mut core = RuntimeCore::new(TestDriver, poses, motions).unwrap();
        let mut scenes = SceneCoordinator::new(library, Rgbw8::OFF);
        let mut speech = SpeechCoordinator::new(DEFAULT_SPEECH_SPOOL_PATH);
        let mut character = CharacterCoordinator::new(43);
        let mut audio = RecordingAudioDevice::blocking();
        handle_daemon_command_with_character(
            "character start",
            1.0,
            &mut core,
            &mut scenes,
            &mut speech,
            &mut character,
            &mut audio,
            None,
        );
        handle_daemon_command_with_character(
            "stop",
            1.1,
            &mut core,
            &mut scenes,
            &mut speech,
            &mut character,
            &mut audio,
            None,
        );
        let scene_started: serde_json::Value =
            serde_json::from_str(&handle_daemon_command_with_character(
                "scene start thinking",
                1.2,
                &mut core,
                &mut scenes,
                &mut speech,
                &mut character,
                &mut audio,
                None,
            ))
            .unwrap();
        assert_eq!(scene_started["ok"], true);
        assert!(scenes.is_active());

        let stopped: serde_json::Value =
            serde_json::from_str(&handle_daemon_command_with_character(
                "character stop",
                1.3,
                &mut core,
                &mut scenes,
                &mut speech,
                &mut character,
                &mut audio,
                None,
            ))
            .unwrap();
        assert_eq!(stopped["ok"], true);
        assert!(!scenes.is_active());
        assert_eq!(
            scenes.last_status().unwrap().state,
            crate::ScenePhase::Cancelled
        );
        assert_eq!(stopped["character"]["state"], "shutting_down");
    }

    #[test]
    fn empty_motion_library_preserves_runtime_failure_exit_code() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let poses =
            PoseLibrary::load(root.join("motion/config/poses.yaml"), &ORION_JOINT_NAMES).unwrap();
        let empty = tempfile::tempdir().unwrap();
        let error = MotionLibrary::load(empty.path(), &poses).unwrap_err();
        assert!(matches!(&error, crate::Error::Runtime(message)
            if message == &format!("Motion library contains no YAML files: {}", empty.path().display())));
        assert_eq!(error_exit_code(&error), 1);
    }

    #[test]
    fn maps_daemon_rejections_to_a_nonzero_exit_code() {
        assert_eq!(
            daemon_response_exit_code(r#"{"ok":false,"error":"busy"}"#).unwrap(),
            EXIT_DAEMON_REJECTED
        );
        assert_eq!(
            daemon_response_exit_code(r#"{"schema_version":2,"mode":"holding"}"#).unwrap(),
            0
        );
        assert_eq!(
            error_exit_code(&crate::Error::InvalidArgument("bad option".into())),
            2
        );
    }

    struct DispatchFixture {
        routines: crate::expression::routines::Routines,
        now: f64,
        core: RuntimeCore<TestDriver>,
        character: CharacterCoordinator,
        rest: RestCoordinator,
        feedback: VoiceFeedback,
        scenes: SceneCoordinator,
        speech: SpeechCoordinator,
        audio: RecordingAudioDevice,
        manual: Option<crate::lamp::LampProgram>,
        voice_run: Option<(u64, String)>,
        assets: AssetReloadContext,
        _spool: tempfile::TempDir,
    }
    impl DispatchFixture {
        fn new(timeout: f64) -> Self {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap();
            let poses =
                PoseLibrary::load(root.join("motion/config/poses.yaml"), &ORION_JOINT_NAMES)
                    .unwrap();
            let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
            let scenes = SceneLibrary::load(root.join("scenes"), &poses, &motions).unwrap();
            let spool = tempfile::tempdir().unwrap();
            Self {
                routines: crate::expression::routines::Routines::load(None, 0., 0.).unwrap(),
                now: 0.0,
                core: RuntimeCore::new(TestDriver, poses, motions).unwrap(),
                character: CharacterCoordinator::new(42),
                rest: RestCoordinator::new(timeout),
                feedback: VoiceFeedback::default(),
                scenes: SceneCoordinator::new(scenes, Rgbw8::OFF),
                speech: SpeechCoordinator::new(spool.path()),
                audio: RecordingAudioDevice::blocking(),
                manual: None,
                voice_run: None,
                assets: AssetReloadContext {
                    poses_file: root.join("motion/config/poses.yaml"),
                    user_poses_directory: root.join("motion/user/poses"),
                    motions_directory: root.join("motion/motions"),
                    scenes_directory: root.join("scenes"),
                    cues: CueLibrary::load(root.join("audio/cues")).unwrap(),
                },
                _spool: spool,
            }
        }
        fn command(&mut self, command: &str) -> serde_json::Value {
            serde_json::from_str(&dispatch_command(
                command,
                self.now,
                &mut self.core,
                &mut self.scenes,
                &mut self.speech,
                &mut self.character,
                &mut self.audio,
                &self.assets,
                &mut self.feedback,
                &mut self.manual,
                &mut self.voice_run,
                &mut self.rest,
                &mut self.routines,
            ))
            .unwrap()
        }
        fn ok(&mut self, command: &str) {
            let result = self.command(command);
            assert_eq!(result["ok"], true, "{command}: {result}");
        }
    }

    #[test]
    fn wake_cues_wait_for_confirmation_and_rejected_candidates_do_not_stop_audio() {
        let mut h = DispatchFixture::new(600.0);
        let rejected = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        for event in ["wake", "verify", "endpoint", "reject"] {
            h.ok(&format!("voice {rejected} {event}"));
            assert!(h.audio.commands().is_empty());
        }
        let accepted = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        for event in ["wake", "verify"] {
            h.ok(&format!("voice {accepted} {event}"));
        }
        assert!(h.audio.commands().is_empty());
        h.ok(&format!("voice {accepted} confirmed"));
        let commands = h.audio.commands().to_vec();
        assert!(!commands.is_empty());
        let history = h.command("voice status");
        assert_eq!(
            history["voice"]["history"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e[1] == "acknowledgment_start")
                .count(),
            1
        );
        h.now = 0.1;
        for event in ["confirmed", "endpoint", "followup"] {
            h.ok(&format!("voice {accepted} {event}"));
        }
        assert_eq!(h.audio.commands(), commands.as_slice());
    }

    #[test]
    fn confirmation_requires_current_endpointed_wake_and_resets_deadline_once() {
        let mut h = DispatchFixture::new(600.0);
        h.rest.started();
        let session = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let other = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        assert_eq!(
            h.command(&format!("voice {session} confirmed"))["ok"],
            false
        );
        h.ok(&format!("voice {session} wake"));
        assert_eq!(
            h.command(&format!("voice {session} confirmed"))["ok"],
            false
        );
        h.ok(&format!("voice {session} endpoint"));
        assert_eq!(h.command(&format!("voice {other} confirmed"))["ok"], false);
        assert_eq!(h.rest.status(h.now).last_confirmed_at, None);
        h.now = 10.0;
        h.ok(&format!("voice {session} confirmed"));
        assert_eq!(h.rest.status(h.now).last_confirmed_at, Some(10.0));
        h.now = 11.0;
        h.ok(&format!("voice {session} confirmed"));
        assert_eq!(h.rest.status(h.now).last_confirmed_at, Some(10.0));
        assert_eq!(h.rest.status(h.now).remaining_seconds, Some(599.0));
        h.ok(&format!("voice {session} finish"));
        assert_eq!(
            h.command(&format!("voice {session} confirmed"))["ok"],
            false
        );
        h.ok(&format!("voice {other} wake"));
        h.ok(&format!("voice {other} endpoint"));
        h.ok(&format!("voice {other} reject"));
        assert_eq!(h.command(&format!("voice {other} confirmed"))["ok"], false);
        assert_eq!(h.rest.status(h.now).last_confirmed_at, Some(10.0));
        assert_eq!(h.rest.status(h.now).remaining_seconds, Some(599.0));
    }

    #[test]
    fn dark_commands_retain_session_and_lamp_preferences_without_cues() {
        let mut h = DispatchFixture::new(600.0);
        h.rest
            .track_rest(&serde_json::json!({"run_id": 1}))
            .unwrap();
        let session = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        h.ok("lamp 1 2 3 40");
        h.ok(&format!("voice {session} wake"));
        h.ok(&format!("voice {session} endpoint"));
        assert!(h.feedback.owns(session));
        assert!(h.audio.commands().is_empty());
        assert!(!h.character.status().enabled);
        h.ok("lamp 20 30 40 50");
        let frame = h.manual.as_ref().unwrap().render(h.now).unwrap();
        assert!(
            frame
                .iter()
                .all(|pixel| *pixel == Rgbw8::new(20, 30, 40, 50))
        );
        let status = h.command("character status");
        assert_eq!(status["rest"]["state"], "going_to_rest");
        assert_eq!(status["rest"]["light_on"], false);
        assert_eq!(status["character"]["enabled"], false);
    }

    #[test]
    fn scoped_reply_rejects_stale_session_and_records_current_owner() {
        let mut h = DispatchFixture::new(600.0);
        let session = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let other = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        write_test_wav(&h._spool.path().join("reply.wav"));
        h.ok(&format!("voice {session} wake"));
        assert_eq!(
            h.command(&format!("speech file reply {other}"))["ok"],
            false
        );
        assert!(h.voice_run.is_none());
        assert!(!h.speech.is_active());
        let result = h.command(&format!("speech file reply {session}"));
        assert_eq!(result["ok"], true);
        assert_eq!(
            h.voice_run,
            Some((result["run_id"].as_u64().unwrap(), session.into()))
        );
        assert_eq!(h.speech.active_status().unwrap().state, SpeechPhase::Queued);
    }
}
