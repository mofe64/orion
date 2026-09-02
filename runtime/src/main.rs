use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use orion_runtime::{
    AlsaAudioDevice, AudioDevice, CharacterCoordinator, CueLibrary, DEFAULT_TTS_SOCKET_PATH,
    LightingDevice, MAX_SPEECH_TEXT_BYTES, MotionLibrary, MujocoDriver, ORION_APLAY_PATH,
    ORION_AUDIO_CARD, ORION_AUDIO_PCM_DEVICE, ORION_JOINT_NAMES, PI5_NEOPIXEL_DEVICE_PATH,
    Pi5NeoPixelDevice, PoseLibrary, RecordingAudioDevice, RecordingLightingDevice, Rgbw8,
    RuntimeCore, RuntimeDriver, RustypotTransport, SceneCoordinator, SceneLibrary,
    SpeechCoordinator, Sts3215Driver, UnixCommandServer, configure_respeaker_v2_mixer,
    load_calibration_file, parse_scene_document, render_effect, request_daemon,
};

const DEFAULT_BAUD_RATE: i32 = 1_000_000;
const DEFAULT_SOCKET_PATH: &str = "/tmp/oriond.sock";
const OBSERVE_PERIOD: Duration = Duration::from_millis(20);
const EXIT_DAEMON_REJECTED: i32 = 3;
const EXIT_MOVEMENT_TIMED_OUT: i32 = 4;
const EXIT_MOVEMENT_CANCELLED: i32 = 5;
const EXIT_SCENE_FAILED: i32 = 6;
const EXIT_SPEECH_FAILED: i32 = 7;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    None,
    Check,
    Serve,
    Status,
    Configure,
    Enable,
    Disable,
    Goto,
    Play,
    PlayCue,
    Stop,
    Light,
    LightPixel,
    LightsOff,
    RunScene,
    SceneStatus,
    StopScene,
    Speak,
    SpeechStatus,
    StopSpeech,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Backend {
    Hardware,
    Mujoco,
}

struct Options {
    operation: Operation,
    backend: Backend,
    help: bool,
    wait: bool,
    port: String,
    baud_rate: i32,
    calibration_file: PathBuf,
    socket_path: PathBuf,
    poses_file: PathBuf,
    user_poses_directory: PathBuf,
    motions_directory: PathBuf,
    audio_cues_directory: PathBuf,
    audio_card: String,
    audio_pcm_device: String,
    cue_name: String,
    pose_name: String,
    motion_name: String,
    duration_seconds: f64,
    scene_file: PathBuf,
    python: PathBuf,
    start_pose: String,
    lighting_device: PathBuf,
    light_color: Rgbw8,
    light_pixel: usize,
    scenes_directory: PathBuf,
    scene_name: String,
    speech_text: String,
    tts_socket_path: PathBuf,
}

#[derive(Clone)]
struct AssetReloadContext {
    poses_file: PathBuf,
    user_poses_directory: PathBuf,
    motions_directory: PathBuf,
    scenes_directory: PathBuf,
    cues: CueLibrary,
}

impl AssetReloadContext {
    fn load_poses(&self) -> orion_runtime::Result<PoseLibrary> {
        PoseLibrary::load_with_user_directory(
            &self.poses_file,
            &self.user_poses_directory,
            &ORION_JOINT_NAMES,
        )
    }

    fn load_motions(&self, poses: &PoseLibrary) -> orion_runtime::Result<MotionLibrary> {
        MotionLibrary::load(&self.motions_directory, poses)
    }

    fn load_scenes(
        &self,
        poses: &PoseLibrary,
        motions: &MotionLibrary,
    ) -> orion_runtime::Result<SceneLibrary> {
        let library = SceneLibrary::load(&self.scenes_directory, poses, motions)?;
        library.validate_audio_cues(&self.cues)?;
        Ok(library)
    }

    fn load_all(&self) -> orion_runtime::Result<(PoseLibrary, MotionLibrary, SceneLibrary)> {
        let poses = self.load_poses()?;
        let motions = self.load_motions(&poses)?;
        let scenes = self.load_scenes(&poses, &motions)?;
        Ok((poses, motions, scenes))
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            operation: Operation::None,
            backend: Backend::Hardware,
            help: false,
            wait: false,
            port: "/dev/ttyACM0".into(),
            baud_rate: DEFAULT_BAUD_RATE,
            calibration_file: PathBuf::new(),
            socket_path: DEFAULT_SOCKET_PATH.into(),
            poses_file: "motion/config/poses.yaml".into(),
            user_poses_directory: "motion/user/poses".into(),
            motions_directory: "motion/motions".into(),
            audio_cues_directory: "audio/cues".into(),
            audio_card: ORION_AUDIO_CARD.into(),
            audio_pcm_device: ORION_AUDIO_PCM_DEVICE.into(),
            cue_name: String::new(),
            pose_name: String::new(),
            motion_name: String::new(),
            duration_seconds: 3.0,
            scene_file: "simulation/mujoco/scene.xml".into(),
            python: ".venv/bin/python".into(),
            start_pose: "attentive".into(),
            lighting_device: PI5_NEOPIXEL_DEVICE_PATH.into(),
            light_color: Rgbw8::OFF,
            light_pixel: 0,
            scenes_directory: "scenes".into(),
            scene_name: String::new(),
            speech_text: String::new(),
            tts_socket_path: DEFAULT_TTS_SOCKET_PATH.into(),
        }
    }
}

fn usage() -> &'static str {
    "Usage:\n\
  oriond --check  [--port DEVICE] [--baud-rate RATE] [--calibration FILE]\n\
  oriond --serve  [--backend hardware|mujoco] [--socket PATH]\n\
  oriond --status [--socket PATH]\n\n\
  oriond --configure [--socket PATH]\n\
  oriond --enable    [--socket PATH]\n\
  oriond --disable   [--socket PATH]\n\n\
  oriond --goto POSE [--duration SECONDS] [--wait] [--socket PATH]\n\n\
  oriond --play MOTION [--wait] [--socket PATH]\n\
  oriond --stop        [--socket PATH]\n\n\
  oriond --play-cue CUE [--cues DIR] [--audio-device DEVICE]\n\n\
  oriond --light RED GREEN BLUE WHITE [--lighting-device PATH]\n\
  oriond --light-pixel INDEX RED GREEN BLUE WHITE [--lighting-device PATH]\n\
  oriond --lights-off [--lighting-device PATH]\n\n\
  oriond --run-scene SCENE [--wait] [--socket PATH]\n\
  oriond --scene-status [--socket PATH]\n\
  oriond --stop-scene [--socket PATH]\n\n\
  oriond --speak TEXT [--wait] [--socket PATH]\n\
  oriond --speech-status [--socket PATH]\n\
  oriond --stop-speech [--socket PATH]\n\n\
  --check             Print one direct hardware state snapshot and exit.\n\
  --serve             Sample the selected backend at 50 Hz and serve status JSON.\n\
  --backend NAME      Use hardware (default) or the native MuJoCo bridge.\n\
  --status            Request the latest JSON snapshot from the daemon.\n\
  --configure         Apply and verify Orion's servo profile, torque off.\n\
  --enable            Seed measured positions, then enable holding torque.\n\
  --disable           Disable holding torque.\n\
  --goto POSE         Move all five joints to a named Orion pose.\n\
  --play MOTION       Play an authored multi-keyframe Orion motion.\n\
  --play-cue CUE      Play one named local WAV cue and wait for completion.\n\
  --stop              Stop movement at the current commanded position.\n\
  --light RGBW        Immediately set all 40 shield pixels (four values, 0-255).\n\
  --light-pixel ...   Light one zero-based pixel and turn the other 39 off.\n\
  --lights-off        Immediately turn all 40 shield pixels off.\n\
  --lighting-device   Pi 5 RP1 PWM device (default: /dev/ws281x_pwm).\n\
  --run-scene SCENE  Submit a named lighting/motion scene to the daemon.\n\
  --scene-status     Show the active and most recent terminal scene.\n\
  --stop-scene       Cancel the active scene and its movement.\n\
  --speak TEXT       Synthesize and play text through the persistent TTS worker.\n\
  --speech-status    Show the active and most recent terminal speech run.\n\
  --stop-speech      Cancel the active speech run or playback.\n\
  --tts-socket PATH  TTS worker socket used by --serve (default: /tmp/orion-tts.sock).\n\
  --scenes DIR       Scene library used by --serve (default: scenes).\n\
  --cues DIR         WAV cue library used by --serve and --play-cue (default: audio/cues).\n\
  --audio-card CARD  ALSA mixer card (default: seeed2micvoicec).\n\
  --audio-device PCM ALSA playback PCM (default: plughw:CARD=seeed2micvoicec,DEV=0).\n\
  --duration SECONDS  Quintic move duration (default: 3.0).\n\
  --wait              Follow the submitted run ID through completion.\n\
  --port DEVICE       Servo serial device (default: /dev/ttyACM0).\n\
  --baud-rate RATE    Servo bus rate (default: 1000000).\n\
  --calibration FILE  Orion calibration JSON file.\n\
  --socket PATH       Local API socket (default: /tmp/oriond.sock).\n\
  --poses FILE        Pose library used by --serve.\n\
  --user-poses DIR    Studio user-pose directory used by --serve.\n\
  --motions DIR       Motion-library directory used by --serve.\n\
  --scene FILE        MuJoCo scene (default: simulation/mujoco/scene.xml).\n\
  --python FILE       Python with MuJoCo installed (default: .venv/bin/python).\n\
  --start-pose POSE   MuJoCo initial pose (default: attentive).\n\
  --help              Show this help.\n\n\
Check and serve startup never enable torque or write servo registers.\n"
}

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("oriond: {error}");
            error_exit_code(&error)
        }
    };
    std::process::exit(code);
}

fn run() -> orion_runtime::Result<i32> {
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
        Operation::Speak => request_speech(&options),
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

fn request_speech(options: &Options) -> orion_runtime::Result<i32> {
    let response = request_daemon(
        &options.socket_path,
        &format!("speech start {}", options.speech_text),
    )?;
    let value: serde_json::Value = serde_json::from_str(response.trim())?;
    let exit_code = daemon_value_exit_code(&value);
    print!("{response}");
    io::stdout().flush()?;
    if exit_code != 0 || !options.wait {
        return Ok(exit_code);
    }
    let run_id = value
        .get("run_id")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            orion_runtime::Error::Runtime(
                "Accepted Orion speech response did not include a run_id.".into(),
            )
        })?;
    wait_for_speech(&options.socket_path, run_id)
}

fn wait_for_speech(socket_path: &std::path::Path, run_id: u64) -> orion_runtime::Result<i32> {
    loop {
        thread::sleep(OBSERVE_PERIOD);
        let response = request_daemon(socket_path, "speech status")?;
        let status: serde_json::Value = serde_json::from_str(response.trim())?;
        let mut found = false;
        for field in ["speech", "last_speech"] {
            let Some(speech) = status.get(field).filter(|value| !value.is_null()) else {
                continue;
            };
            if speech.get("run_id").and_then(serde_json::Value::as_u64) != Some(run_id) {
                continue;
            }
            found = true;
            let state = speech
                .get("state")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    orion_runtime::Error::Runtime(
                        "Orion speech status did not include a state.".into(),
                    )
                })?;
            match speech_state_exit_code(state)? {
                None => break,
                Some(exit_code) => {
                    println!("{}", serde_json::to_string(speech)?);
                    return Ok(exit_code);
                }
            }
        }
        if !found {
            return Err(orion_runtime::Error::Runtime(format!(
                "Orion speech run {run_id} is no longer the active or most recent result."
            )));
        }
    }
}

fn speech_state_exit_code(state: &str) -> orion_runtime::Result<Option<i32>> {
    match state {
        "queued" | "synthesizing" | "playing" => Ok(None),
        "completed" => Ok(Some(0)),
        "cancelled" => Ok(Some(EXIT_MOVEMENT_CANCELLED)),
        "failed" => Ok(Some(EXIT_SPEECH_FAILED)),
        value => Err(orion_runtime::Error::Runtime(format!(
            "Unknown Orion speech state: {value}"
        ))),
    }
}

fn request_scene(options: &Options) -> orion_runtime::Result<i32> {
    let response = request_daemon(
        &options.socket_path,
        &format!("scene start {}", options.scene_name),
    )?;
    let value: serde_json::Value = serde_json::from_str(response.trim())?;
    let exit_code = daemon_value_exit_code(&value);
    print!("{response}");
    io::stdout().flush()?;
    if exit_code != 0 || !options.wait {
        return Ok(exit_code);
    }
    let run_id = value
        .get("run_id")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            orion_runtime::Error::Runtime(
                "Accepted Orion scene response did not include a run_id.".into(),
            )
        })?;
    wait_for_scene(&options.socket_path, run_id)
}

fn wait_for_scene(socket_path: &std::path::Path, run_id: u64) -> orion_runtime::Result<i32> {
    loop {
        thread::sleep(OBSERVE_PERIOD);
        let response = request_daemon(socket_path, "scene status")?;
        let status: serde_json::Value = serde_json::from_str(response.trim())?;
        let mut found = false;
        for field in ["scene", "last_scene"] {
            let Some(scene) = status.get(field).filter(|value| !value.is_null()) else {
                continue;
            };
            if scene.get("run_id").and_then(serde_json::Value::as_u64) != Some(run_id) {
                continue;
            }
            found = true;
            let state = scene
                .get("state")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    orion_runtime::Error::Runtime(
                        "Orion scene status did not include a state.".into(),
                    )
                })?;
            match scene_state_exit_code(state)? {
                None => break,
                Some(exit_code) => {
                    println!("{}", serde_json::to_string(scene)?);
                    return Ok(exit_code);
                }
            }
        }
        if !found {
            return Err(orion_runtime::Error::Runtime(format!(
                "Orion scene run {run_id} is no longer the active or most recent result."
            )));
        }
    }
}

fn scene_state_exit_code(state: &str) -> orion_runtime::Result<Option<i32>> {
    match state {
        "executing" => Ok(None),
        "completed" => Ok(Some(0)),
        "timed_out" => Ok(Some(EXIT_MOVEMENT_TIMED_OUT)),
        "cancelled" => Ok(Some(EXIT_MOVEMENT_CANCELLED)),
        "failed" => Ok(Some(EXIT_SCENE_FAILED)),
        value => Err(orion_runtime::Error::Runtime(format!(
            "Unknown Orion scene state: {value}"
        ))),
    }
}

fn print_response(response: String) -> orion_runtime::Result<i32> {
    let exit_code = daemon_response_exit_code(&response)?;
    print!("{response}");
    io::stdout().flush()?;
    Ok(exit_code)
}

fn request_movement(options: &Options, command: &str) -> orion_runtime::Result<i32> {
    let response = request_daemon(&options.socket_path, command)?;
    let value: serde_json::Value = serde_json::from_str(response.trim())?;
    let exit_code = daemon_value_exit_code(&value);
    print!("{response}");
    io::stdout().flush()?;
    if exit_code != 0 || !options.wait {
        return Ok(exit_code);
    }

    let run_id = value
        .get("run_id")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            orion_runtime::Error::Runtime(
                "Accepted Orion movement response did not include a run_id.".into(),
            )
        })?;
    wait_for_movement(&options.socket_path, run_id)
}

fn wait_for_movement(socket_path: &std::path::Path, run_id: u64) -> orion_runtime::Result<i32> {
    loop {
        thread::sleep(OBSERVE_PERIOD);
        let response = request_daemon(socket_path, "status")?;
        let status: serde_json::Value = serde_json::from_str(response.trim())?;
        let mut found = false;
        for field in ["motion", "last_motion"] {
            let Some(movement) = status.get(field).filter(|value| !value.is_null()) else {
                continue;
            };
            if movement.get("run_id").and_then(serde_json::Value::as_u64) != Some(run_id) {
                continue;
            }
            found = true;
            let state = movement
                .get("state")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    orion_runtime::Error::Runtime(
                        "Orion movement status did not include a state.".into(),
                    )
                })?;
            match state {
                "executing" | "settling" => break,
                "completed" => {
                    println!("{}", serde_json::to_string(movement)?);
                    return Ok(0);
                }
                "timed_out" => {
                    println!("{}", serde_json::to_string(movement)?);
                    return Ok(EXIT_MOVEMENT_TIMED_OUT);
                }
                "cancelled" => {
                    println!("{}", serde_json::to_string(movement)?);
                    return Ok(EXIT_MOVEMENT_CANCELLED);
                }
                value => {
                    return Err(orion_runtime::Error::Runtime(format!(
                        "Unknown Orion movement state: {value}"
                    )));
                }
            }
        }
        if !found {
            return Err(orion_runtime::Error::Runtime(format!(
                "Orion movement run {run_id} is no longer the active or most recent result."
            )));
        }
    }
}

fn daemon_response_exit_code(response: &str) -> orion_runtime::Result<i32> {
    let value: serde_json::Value = serde_json::from_str(response.trim())?;
    Ok(daemon_value_exit_code(&value))
}

fn daemon_value_exit_code(value: &serde_json::Value) -> i32 {
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
        EXIT_DAEMON_REJECTED
    } else {
        0
    }
}

fn error_exit_code(error: &orion_runtime::Error) -> i32 {
    if matches!(error, orion_runtime::Error::InvalidArgument(_)) {
        2
    } else {
        1
    }
}

fn parse_options(arguments: impl Iterator<Item = String>) -> orion_runtime::Result<Options> {
    let mut options = Options::default();
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--check" => select_operation(&mut options, Operation::Check, &argument)?,
            "--serve" => select_operation(&mut options, Operation::Serve, &argument)?,
            "--status" => select_operation(&mut options, Operation::Status, &argument)?,
            "--configure" => select_operation(&mut options, Operation::Configure, &argument)?,
            "--enable" => select_operation(&mut options, Operation::Enable, &argument)?,
            "--disable" => select_operation(&mut options, Operation::Disable, &argument)?,
            "--stop" => select_operation(&mut options, Operation::Stop, &argument)?,
            "--run-scene" => {
                select_operation(&mut options, Operation::RunScene, &argument)?;
                options.scene_name = require_value(&mut arguments, &argument)?;
            }
            "--scene-status" => select_operation(&mut options, Operation::SceneStatus, &argument)?,
            "--stop-scene" => select_operation(&mut options, Operation::StopScene, &argument)?,
            "--speak" => {
                select_operation(&mut options, Operation::Speak, &argument)?;
                options.speech_text = require_value(&mut arguments, &argument)?;
            }
            "--speech-status" => {
                select_operation(&mut options, Operation::SpeechStatus, &argument)?
            }
            "--stop-speech" => select_operation(&mut options, Operation::StopSpeech, &argument)?,
            "--lights-off" => {
                select_operation(&mut options, Operation::LightsOff, &argument)?;
                options.light_color = Rgbw8::OFF;
            }
            "--light" => {
                select_operation(&mut options, Operation::Light, &argument)?;
                options.light_color = parse_rgbw(&mut arguments, &argument)?;
            }
            "--light-pixel" => {
                select_operation(&mut options, Operation::LightPixel, &argument)?;
                options.light_pixel =
                    require_value(&mut arguments, &argument)?
                        .parse()
                        .map_err(|_| {
                            orion_runtime::Error::InvalidArgument(
                                "--light-pixel requires an integer pixel index.".into(),
                            )
                        })?;
                options.light_color = parse_rgbw(&mut arguments, &argument)?;
            }
            "--goto" => {
                select_operation(&mut options, Operation::Goto, &argument)?;
                options.pose_name = require_value(&mut arguments, &argument)?;
            }
            "--play" => {
                select_operation(&mut options, Operation::Play, &argument)?;
                options.motion_name = require_value(&mut arguments, &argument)?;
            }
            "--play-cue" => {
                select_operation(&mut options, Operation::PlayCue, &argument)?;
                options.cue_name = require_value(&mut arguments, &argument)?;
            }
            "--help" | "-h" => options.help = true,
            "--wait" => options.wait = true,
            "--backend" => {
                options.backend = match require_value(&mut arguments, &argument)?.as_str() {
                    "hardware" => Backend::Hardware,
                    "mujoco" => Backend::Mujoco,
                    value => {
                        return Err(orion_runtime::Error::InvalidArgument(format!(
                            "Unknown Orion runtime backend: {value}"
                        )));
                    }
                }
            }
            "--port" => options.port = require_value(&mut arguments, &argument)?,
            "--baud-rate" => {
                options.baud_rate =
                    require_value(&mut arguments, &argument)?
                        .parse()
                        .map_err(|_| {
                            orion_runtime::Error::InvalidArgument(
                                "--baud-rate requires an integer.".into(),
                            )
                        })?;
            }
            "--calibration" => {
                options.calibration_file = require_value(&mut arguments, &argument)?.into()
            }
            "--socket" => options.socket_path = require_value(&mut arguments, &argument)?.into(),
            "--tts-socket" => {
                options.tts_socket_path = require_value(&mut arguments, &argument)?.into()
            }
            "--poses" => options.poses_file = require_value(&mut arguments, &argument)?.into(),
            "--user-poses" => {
                options.user_poses_directory = require_value(&mut arguments, &argument)?.into()
            }
            "--motions" => {
                options.motions_directory = require_value(&mut arguments, &argument)?.into()
            }
            "--cues" => {
                options.audio_cues_directory = require_value(&mut arguments, &argument)?.into()
            }
            "--audio-card" => options.audio_card = require_value(&mut arguments, &argument)?,
            "--audio-device" => {
                options.audio_pcm_device = require_value(&mut arguments, &argument)?
            }
            "--scenes" => {
                options.scenes_directory = require_value(&mut arguments, &argument)?.into()
            }
            "--scene" => options.scene_file = require_value(&mut arguments, &argument)?.into(),
            "--python" => options.python = require_value(&mut arguments, &argument)?.into(),
            "--start-pose" => options.start_pose = require_value(&mut arguments, &argument)?,
            "--lighting-device" => {
                options.lighting_device = require_value(&mut arguments, &argument)?.into()
            }
            "--duration" => {
                options.duration_seconds = require_value(&mut arguments, &argument)?
                    .parse()
                    .map_err(|_| {
                        orion_runtime::Error::InvalidArgument(
                            "--duration requires a number.".into(),
                        )
                    })?;
            }
            _ => {
                return Err(orion_runtime::Error::InvalidArgument(format!(
                    "Unknown option: {argument}"
                )));
            }
        }
    }
    if options.backend == Backend::Mujoco && options.operation == Operation::Check {
        return Err(orion_runtime::Error::InvalidArgument(
            "--check is a direct-hardware operation; use --serve --backend mujoco.".into(),
        ));
    }
    if options.wait
        && !matches!(
            options.operation,
            Operation::Goto | Operation::Play | Operation::RunScene | Operation::Speak
        )
    {
        return Err(orion_runtime::Error::InvalidArgument(
            "--wait is only valid with --goto, --play, --run-scene, or --speak.".into(),
        ));
    }
    if options.operation == Operation::LightPixel
        && options.light_pixel >= orion_runtime::ORION_LIGHT_PIXEL_COUNT
    {
        return Err(orion_runtime::Error::InvalidArgument(format!(
            "--light-pixel index must be between 0 and {}.",
            orion_runtime::ORION_LIGHT_PIXEL_COUNT - 1
        )));
    }
    if options.operation == Operation::Speak {
        let text = options.speech_text.trim();
        if text.is_empty() || text.contains('\n') || text.contains('\r') {
            return Err(orion_runtime::Error::InvalidArgument(
                "--speak requires non-empty text without line breaks.".into(),
            ));
        }
        if text.len() > MAX_SPEECH_TEXT_BYTES {
            return Err(orion_runtime::Error::InvalidArgument(format!(
                "--speak text cannot exceed {MAX_SPEECH_TEXT_BYTES} UTF-8 bytes."
            )));
        }
        options.speech_text = text.to_owned();
    }
    if options.backend == Backend::Hardware
        && matches!(options.operation, Operation::Check | Operation::Serve)
        && options.calibration_file.as_os_str().is_empty()
    {
        let home = env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                orion_runtime::Error::Runtime(
                    "HOME is not set; pass --calibration with an absolute path.".into(),
                )
            })?;
        options.calibration_file = PathBuf::from(home).join(".config/orion/servo_calibration.json");
    }
    Ok(options)
}

fn parse_rgbw(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> orion_runtime::Result<Rgbw8> {
    let mut channels = [0_u8; 4];
    for channel in &mut channels {
        *channel = require_value(arguments, option)?.parse().map_err(|_| {
            orion_runtime::Error::InvalidArgument(format!(
                "{option} RGBW values must be integers from 0 through 255."
            ))
        })?;
    }
    Ok(Rgbw8::new(
        channels[0],
        channels[1],
        channels[2],
        channels[3],
    ))
}

fn render_light(options: &Options, pixel: Option<usize>) -> orion_runtime::Result<i32> {
    let mut device = Pi5NeoPixelDevice::open(
        &options.lighting_device,
        orion_runtime::ORION_LIGHT_PIXEL_COUNT,
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

fn require_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> orion_runtime::Result<String> {
    arguments
        .next()
        .ok_or_else(|| orion_runtime::Error::InvalidArgument(format!("{option} requires a value.")))
}

fn select_operation(
    options: &mut Options,
    operation: Operation,
    argument: &str,
) -> orion_runtime::Result<()> {
    if options.operation != Operation::None {
        return Err(orion_runtime::Error::InvalidArgument(format!(
            "Select exactly one operation; repeated at {argument}."
        )));
    }
    options.operation = operation;
    Ok(())
}

fn connect_driver(options: &Options) -> orion_runtime::Result<Sts3215Driver<RustypotTransport>> {
    let calibrations = load_calibration_file(&options.calibration_file, &ORION_JOINT_NAMES)?;
    let mut driver = Sts3215Driver::new(RustypotTransport::default());
    driver.connect(&options.port, options.baud_rate, calibrations)?;
    Ok(driver)
}

fn play_cue(options: &Options) -> orion_runtime::Result<i32> {
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

fn serve(options: Options) -> orion_runtime::Result<i32> {
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
                orion_runtime::ORION_LIGHT_PIXEL_COUNT,
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

fn serve_driver<D: RuntimeDriver>(
    driver: D,
    poses: PoseLibrary,
    motions: MotionLibrary,
    scene_library: SceneLibrary,
    asset_reload: AssetReloadContext,
    mut lighting: Box<dyn LightingDevice>,
    mut audio: Box<dyn AudioDevice>,
    options: &Options,
    backend: &str,
) -> orion_runtime::Result<i32> {
    let mut core = RuntimeCore::new(driver, poses, motions)?;
    lighting.clear()?;
    let mut scenes = SceneCoordinator::new(scene_library, Rgbw8::OFF);
    let mut speech = SpeechCoordinator::new(&options.tts_socket_path)?;
    let mut character = CharacterCoordinator::new(0x4f52_494f_4e);
    let server = UnixCommandServer::bind(&options.socket_path)?;
    let stopping = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&stopping))?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&stopping))?;

    println!(
        "oriond: observing {backend} at 50 Hz on {}",
        options.socket_path.display()
    );
    let started_at = Instant::now();
    let mut next_sample = started_at;
    while !stopping.load(Ordering::Relaxed) {
        next_sample += OBSERVE_PERIOD;
        let now_seconds = started_at.elapsed().as_secs_f64();
        core.tick(now_seconds)?;
        scenes.tick(now_seconds, &mut core, lighting.as_mut(), audio.as_mut())?;
        speech.tick(audio.as_mut());
        if let Some(energy) = speech.active_energy() {
            let normalized = (energy / 0.22).clamp(0.08, 1.0);
            lighting.render(&render_effect("speaking_energy", now_seconds, normalized)?)?;
        }
        character.tick(
            now_seconds,
            &mut core,
            scenes.is_active(),
            speech.is_active(),
            speech.active_analysis(),
            speech.active_energy_frame(),
        )?;
        if !scenes.is_active()
            && !speech.is_active()
            && let Some(effect) = character.background_lighting_effect(&core)
        {
            lighting.render(&render_effect(&effect, now_seconds, 0.55)?)?;
        }
        server.serve_pending(|command| {
            handle_daemon_command_with_character(
                command,
                started_at.elapsed().as_secs_f64(),
                &mut core,
                &mut scenes,
                &mut speech,
                &mut character,
                audio.as_mut(),
                Some(&asset_reload),
            )
        })?;
        thread::sleep(next_sample.saturating_duration_since(Instant::now()));
    }
    Ok(0)
}

fn handle_daemon_command_with_character<D: RuntimeDriver, A: AudioDevice + ?Sized>(
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
fn handle_daemon_command<D: RuntimeDriver, A: AudioDevice + ?Sized>(
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
fn handle_daemon_command_with_reload<D: RuntimeDriver, A: AudioDevice + ?Sized>(
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

fn handle_daemon_command_inner<D: RuntimeDriver, A: AudioDevice + ?Sized>(
    command: &str,
    now_seconds: f64,
    core: &mut RuntimeCore<D>,
    scenes: &mut SceneCoordinator,
    speech: &mut SpeechCoordinator,
    audio: &mut A,
    asset_reload: Option<&AssetReloadContext>,
    mut character: Option<&mut CharacterCoordinator>,
) -> orion_runtime::Result<String> {
    if command == "character status" {
        let status = character.as_deref().map(CharacterCoordinator::status);
        return Ok(serde_json::json!({"ok": true, "character": status}).to_string());
    }
    if command == "character start" {
        let coordinator = character.as_deref_mut().ok_or_else(|| {
            orion_runtime::Error::InvalidState("Character coordinator is not configured.".into())
        })?;
        let status = coordinator.start(now_seconds, core)?;
        return Ok(
            serde_json::json!({"ok": true, "command": "character_start", "character": status})
                .to_string(),
        );
    }
    if command == "character stop" {
        if scenes.is_active() {
            scenes.cancel(now_seconds, core, audio)?;
        }
        if speech.is_active() {
            speech.cancel(audio)?;
        }
        let coordinator = character.as_deref_mut().ok_or_else(|| {
            orion_runtime::Error::InvalidState("Character coordinator is not configured.".into())
        })?;
        let status = coordinator.stop(now_seconds, core)?;
        return Ok(
            serde_json::json!({"ok": true, "command": "character_stop", "character": status})
                .to_string(),
        );
    }
    if let Some(state) = command.strip_prefix("character state ") {
        let coordinator = character.as_deref_mut().ok_or_else(|| {
            orion_runtime::Error::InvalidState("Character coordinator is not configured.".into())
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
    if let Some(identifier) = command.strip_prefix("speech file ") {
        if scenes.is_active() {
            return Ok(serde_json::json!({"ok": false, "command": "speech_file", "error": "scene already active"}).to_string());
        }
        if let Some(coordinator) = character.as_deref_mut() {
            coordinator.preempt_idle(now_seconds, core)?;
            coordinator.note_foreground_started(now_seconds);
        }
        let status = speech.start_spooled(identifier.trim())?;
        return Ok(serde_json::json!({"ok": true, "command": "speech_file", "run_id": status.run_id, "state": status.state}).to_string());
    }
    if let Some(text) = command.strip_prefix("speech start ") {
        if let Some(coordinator) = character.as_deref_mut() {
            coordinator.preempt_idle(now_seconds, core)?;
            coordinator.note_foreground_started(now_seconds);
        }
        if scenes.is_active() {
            return Ok(serde_json::json!({
                "ok": false,
                "command": "speech_start",
                "error": "scene already active",
                "active_scene_run_id": scenes.active_status().map(|status| status.run_id),
            })
            .to_string());
        }
        let status = speech.start(text)?;
        return Ok(serde_json::json!({
            "ok": true,
            "command": "speech_start",
            "run_id": status.run_id,
            "state": status.state,
        })
        .to_string());
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
        let context = asset_reload.ok_or_else(|| {
            orion_runtime::Error::InvalidState("Scene reload is not configured.".into())
        })?;
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
        let context = asset_reload.ok_or_else(|| {
            orion_runtime::Error::InvalidState("Asset reload is not configured.".into())
        })?;
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
            coordinator.preempt_idle(now_seconds, core)?;
        }
        let context = asset_reload.ok_or_else(|| {
            orion_runtime::Error::InvalidState("Scene preview is not configured.".into())
        })?;
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
            coordinator.note_foreground_started(now_seconds);
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
            coordinator.preempt_idle(now_seconds, core)?;
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
            coordinator.note_foreground_started(now_seconds);
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
        coordinator.preempt_idle(now_seconds, core)?;
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

fn print_states(states: &[orion_runtime::JointState]) {
    println!(
        "{:<25}{:>13}{:>13}{:>12}{:>10}{:>8}{:>8}",
        "joint", "position", "velocity", "current", "voltage", "temp", "status"
    );
    for state in states {
        println!(
            "{:<25}{:>13.3}{:>13.3}{:>12.3}{:>10.3}{:>8.3}{:>8}",
            state.name,
            state.position,
            state.velocity,
            state.current_ma,
            state.voltage_v,
            state.temperature_c,
            state.status
        );
    }
}

#[cfg(test)]
mod tests {
    use orion_runtime::SpeechPhase;
    use orion_runtime::{
        AudioCommand, JointPositions, JointState, RecordingAudioDevice, UnavailableAudioDevice,
    };

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
        fn apply_servo_profile(&mut self) -> orion_runtime::Result<()> {
            Ok(())
        }

        fn activate(&mut self) -> orion_runtime::Result<Vec<JointState>> {
            Ok(Self::states())
        }

        fn deactivate(&mut self) -> orion_runtime::Result<()> {
            Ok(())
        }

        fn read(&mut self) -> orion_runtime::Result<Vec<JointState>> {
            Ok(Self::states())
        }

        fn write(&mut self, _positions_radians: &JointPositions) -> orion_runtime::Result<()> {
            Ok(())
        }

        fn joint_limits(&self) -> orion_runtime::Result<Vec<orion_runtime::JointLimit>> {
            Ok(ORION_JOINT_NAMES
                .iter()
                .map(|name| orion_runtime::JointLimit {
                    name: (*name).to_owned(),
                    lower_rad: -3.0,
                    upper_rad: 3.0,
                })
                .collect())
        }

        fn validate_positions(
            &self,
            _positions_radians: &JointPositions,
        ) -> orion_runtime::Result<()> {
            Ok(())
        }

        fn clamp_positions_to_safe_range(
            &self,
            positions_radians: &JointPositions,
        ) -> orion_runtime::Result<JointPositions> {
            Ok(positions_radians.clone())
        }
    }

    fn parse(values: &[&str]) -> orion_runtime::Result<Options> {
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
    fn parses_speech_clients_and_wait() {
        let options = parse(&[
            "--speak",
            "Hello from Orion.",
            "--wait",
            "--tts-socket",
            "/tmp/test-tts.sock",
        ])
        .unwrap();
        assert_eq!(options.operation, Operation::Speak);
        assert_eq!(options.speech_text, "Hello from Orion.");
        assert_eq!(options.tts_socket_path, PathBuf::from("/tmp/test-tts.sock"));
        assert!(options.wait);
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
    fn maps_speech_terminal_states_to_exit_codes() {
        assert_eq!(speech_state_exit_code("synthesizing").unwrap(), None);
        assert_eq!(speech_state_exit_code("playing").unwrap(), None);
        assert_eq!(speech_state_exit_code("completed").unwrap(), Some(0));
        assert_eq!(
            speech_state_exit_code("cancelled").unwrap(),
            Some(EXIT_MOVEMENT_CANCELLED)
        );
        assert_eq!(
            speech_state_exit_code("failed").unwrap(),
            Some(EXIT_SPEECH_FAILED)
        );
        assert!(speech_state_exit_code("mystery").is_err());
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
        let mut speech = SpeechCoordinator::new("/tmp/orion-test-tts.sock").unwrap();
        let mut audio = UnavailableAudioDevice;

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
        let mut speech = SpeechCoordinator::with_spool("/tmp/not-used.sock", spool.path()).unwrap();
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
        let mut speech = SpeechCoordinator::with_spool("/tmp/not-used.sock", spool.path()).unwrap();
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
        let mut speech = SpeechCoordinator::new("/tmp/not-used.sock").unwrap();
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
            orion_runtime::ScenePhase::Cancelled
        );
        assert_eq!(stopped["character"]["state"], "shutting_down");
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
            error_exit_code(&orion_runtime::Error::InvalidArgument("bad option".into())),
            2
        );
    }
}
