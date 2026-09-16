use std::env;
use std::path::PathBuf;

use crate::{ORION_AUDIO_CARD, ORION_AUDIO_PCM_DEVICE, PI5_NEOPIXEL_DEVICE_PATH, Rgbw8};

const DEFAULT_BAUD_RATE: i32 = 1_000_000;
const DEFAULT_SOCKET_PATH: &str = "/tmp/oriond.sock";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Operation {
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
    SpeechStatus,
    StopSpeech,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Backend {
    Hardware,
    Mujoco,
}

pub(super) struct Options {
    pub(super) operation: Operation,
    pub(super) backend: Backend,
    pub(super) help: bool,
    pub(super) character_on_start: bool,
    pub(super) rest_after_seconds: f64,
    pub(super) routines_file: Option<PathBuf>,
    pub(super) wait: bool,
    pub(super) port: String,
    pub(super) baud_rate: i32,
    pub(super) calibration_file: PathBuf,
    pub(super) socket_path: PathBuf,
    pub(super) poses_file: PathBuf,
    pub(super) user_poses_directory: PathBuf,
    pub(super) motions_directory: PathBuf,
    pub(super) audio_cues_directory: PathBuf,
    pub(super) audio_card: String,
    pub(super) audio_pcm_device: String,
    pub(super) cue_name: String,
    pub(super) pose_name: String,
    pub(super) motion_name: String,
    pub(super) duration_seconds: f64,
    pub(super) scene_file: PathBuf,
    pub(super) python: PathBuf,
    pub(super) start_pose: String,
    pub(super) lighting_device: PathBuf,
    pub(super) light_color: Rgbw8,
    pub(super) light_pixel: usize,
    pub(super) scenes_directory: PathBuf,
    pub(super) scene_name: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            operation: Operation::None,
            backend: Backend::Hardware,
            help: false,
            character_on_start: true,
            rest_after_seconds: crate::expression::rest::DEFAULT_REST_AFTER_SECONDS,
            routines_file: None,
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
        }
    }
}

pub(super) fn usage() -> &'static str {
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
  --speech-status    Show the active and most recent terminal speech run.\n\
  --stop-speech      Cancel the active speech run or playback.\n\
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
  --character-on-start on|off  Start character automatically (default: on).\n\
  --rest-after-seconds SECONDS  Idle-mode inactivity before rest (default: 1800).\n\
  --routines-file PATH  Saved user mode and alerts (hardware: ~/.config/orion/routines.json).\n\
  --start-pose POSE   MuJoCo initial pose (default: attentive).\n\
  --help              Show this help.\n\n\
Check never enables torque. Serve starts powered character mode unless --character-on-start off.\n"
}

pub(super) fn parse_options(arguments: impl Iterator<Item = String>) -> crate::Result<Options> {
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
                            crate::Error::InvalidArgument(
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
                        return Err(crate::Error::InvalidArgument(format!(
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
                            crate::Error::InvalidArgument("--baud-rate requires an integer.".into())
                        })?;
            }
            "--calibration" => {
                options.calibration_file = require_value(&mut arguments, &argument)?.into()
            }
            "--routines-file" => {
                options.routines_file = Some(require_value(&mut arguments, &argument)?.into())
            }
            "--socket" => options.socket_path = require_value(&mut arguments, &argument)?.into(),
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
            "--character-on-start" => {
                options.character_on_start =
                    match require_value(&mut arguments, &argument)?.as_str() {
                        "on" => true,
                        "off" => false,
                        _ => {
                            return Err(crate::Error::InvalidArgument(
                                "--character-on-start must be on or off".into(),
                            ));
                        }
                    };
            }
            "--rest-after-seconds" => {
                options.rest_after_seconds = require_value(&mut arguments, &argument)?
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .ok_or_else(|| {
                        crate::Error::InvalidArgument(
                            "--rest-after-seconds requires a finite positive number.".into(),
                        )
                    })?;
            }
            "--start-pose" => options.start_pose = require_value(&mut arguments, &argument)?,
            "--lighting-device" => {
                options.lighting_device = require_value(&mut arguments, &argument)?.into()
            }
            "--duration" => {
                options.duration_seconds = require_value(&mut arguments, &argument)?
                    .parse()
                    .map_err(|_| {
                        crate::Error::InvalidArgument("--duration requires a number.".into())
                    })?;
            }
            _ => {
                return Err(crate::Error::InvalidArgument(format!(
                    "Unknown option: {argument}"
                )));
            }
        }
    }
    if options.backend == Backend::Mujoco && options.operation == Operation::Check {
        return Err(crate::Error::InvalidArgument(
            "--check is a direct-hardware operation; use --serve --backend mujoco.".into(),
        ));
    }
    if options.wait
        && !matches!(
            options.operation,
            Operation::Goto | Operation::Play | Operation::RunScene
        )
    {
        return Err(crate::Error::InvalidArgument(
            "--wait is only valid with --goto, --play, or --run-scene.".into(),
        ));
    }
    if options.operation == Operation::LightPixel
        && options.light_pixel >= crate::ORION_LIGHT_PIXEL_COUNT
    {
        return Err(crate::Error::InvalidArgument(format!(
            "--light-pixel index must be between 0 and {}.",
            crate::ORION_LIGHT_PIXEL_COUNT - 1
        )));
    }
    if options.backend == Backend::Hardware
        && matches!(options.operation, Operation::Check | Operation::Serve)
        && options.calibration_file.as_os_str().is_empty()
    {
        let home = env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                crate::Error::Runtime(
                    "HOME is not set; pass --calibration with an absolute path.".into(),
                )
            })?;
        options.calibration_file = PathBuf::from(home).join(".config/orion/servo_calibration.json");
    }
    Ok(options)
}

pub(super) fn parse_rgbw(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> crate::Result<Rgbw8> {
    let mut channels = [0_u8; 4];
    for channel in &mut channels {
        *channel = require_value(arguments, option)?.parse().map_err(|_| {
            crate::Error::InvalidArgument(format!(
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

pub(super) fn require_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> crate::Result<String> {
    arguments
        .next()
        .ok_or_else(|| crate::Error::InvalidArgument(format!("{option} requires a value.")))
}

pub(super) fn select_operation(
    options: &mut Options,
    operation: Operation,
    argument: &str,
) -> crate::Result<()> {
    if options.operation != Operation::None {
        return Err(crate::Error::InvalidArgument(format!(
            "Select exactly one operation; repeated at {argument}."
        )));
    }
    options.operation = operation;
    Ok(())
}
