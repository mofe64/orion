use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use orion_runtime::{
    MotionLibrary, MujocoDriver, ORION_JOINT_NAMES, PoseLibrary, RuntimeCore, RuntimeDriver,
    RustypotTransport, Sts3215Driver, UnixCommandServer, load_calibration_file, request_daemon,
};

const DEFAULT_BAUD_RATE: i32 = 1_000_000;
const DEFAULT_SOCKET_PATH: &str = "/tmp/oriond.sock";
const OBSERVE_PERIOD: Duration = Duration::from_millis(20);

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
    Stop,
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
    port: String,
    baud_rate: i32,
    calibration_file: PathBuf,
    socket_path: PathBuf,
    poses_file: PathBuf,
    motions_directory: PathBuf,
    pose_name: String,
    motion_name: String,
    duration_seconds: f64,
    scene_file: PathBuf,
    python: PathBuf,
    start_pose: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            operation: Operation::None,
            backend: Backend::Hardware,
            help: false,
            port: "/dev/ttyACM0".into(),
            baud_rate: DEFAULT_BAUD_RATE,
            calibration_file: PathBuf::new(),
            socket_path: DEFAULT_SOCKET_PATH.into(),
            poses_file: "motion/config/poses.yaml".into(),
            motions_directory: "motion/motions".into(),
            pose_name: String::new(),
            motion_name: String::new(),
            duration_seconds: 3.0,
            scene_file: "simulation/mujoco/scene.xml".into(),
            python: ".venv/bin/python".into(),
            start_pose: "attentive".into(),
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
  oriond --goto POSE [--duration SECONDS] [--socket PATH]\n\n\
  oriond --play MOTION [--socket PATH]\n\
  oriond --stop        [--socket PATH]\n\n\
  --check             Print one direct hardware state snapshot and exit.\n\
  --serve             Sample the selected backend at 50 Hz and serve status JSON.\n\
  --backend NAME      Use hardware (default) or the native MuJoCo bridge.\n\
  --status            Request the latest JSON snapshot from the daemon.\n\
  --configure         Apply and verify Orion's servo profile, torque off.\n\
  --enable            Seed measured positions, then enable holding torque.\n\
  --disable           Disable holding torque.\n\
  --goto POSE         Move all five joints to a named Orion pose.\n\
  --play MOTION       Play an authored multi-keyframe Orion motion.\n\
  --stop              Stop movement at the current commanded position.\n\
  --duration SECONDS  Quintic move duration (default: 3.0).\n\
  --port DEVICE       Servo serial device (default: /dev/ttyACM0).\n\
  --baud-rate RATE    Servo bus rate (default: 1000000).\n\
  --calibration FILE  Orion calibration JSON file.\n\
  --socket PATH       Local API socket (default: /tmp/oriond.sock).\n\
  --poses FILE        Pose library used by --serve.\n\
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
            1
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
        Operation::Goto => print_response(request_daemon(
            &options.socket_path,
            &format!("goto {} {:.6}", options.pose_name, options.duration_seconds),
        )?),
        Operation::Play => print_response(request_daemon(
            &options.socket_path,
            &format!("play {}", options.motion_name),
        )?),
        Operation::Stop => print_response(request_daemon(&options.socket_path, "stop")?),
        Operation::None => {
            eprint!("{}", usage());
            Ok(2)
        }
    }
}

fn print_response(response: String) -> orion_runtime::Result<i32> {
    print!("{response}");
    io::stdout().flush()?;
    Ok(0)
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
            "--goto" => {
                select_operation(&mut options, Operation::Goto, &argument)?;
                options.pose_name = require_value(&mut arguments, &argument)?;
            }
            "--play" => {
                select_operation(&mut options, Operation::Play, &argument)?;
                options.motion_name = require_value(&mut arguments, &argument)?;
            }
            "--help" | "-h" => options.help = true,
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
            "--poses" => options.poses_file = require_value(&mut arguments, &argument)?.into(),
            "--motions" => {
                options.motions_directory = require_value(&mut arguments, &argument)?.into()
            }
            "--scene" => options.scene_file = require_value(&mut arguments, &argument)?.into(),
            "--python" => options.python = require_value(&mut arguments, &argument)?.into(),
            "--start-pose" => options.start_pose = require_value(&mut arguments, &argument)?,
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

fn serve(options: Options) -> orion_runtime::Result<i32> {
    let poses = PoseLibrary::load(&options.poses_file, &ORION_JOINT_NAMES)?;
    let motions = MotionLibrary::load(&options.motions_directory, &poses)?;
    match options.backend {
        Backend::Hardware => {
            let driver = connect_driver(&options)?;
            serve_driver(driver, poses, motions, &options, "hardware")
        }
        Backend::Mujoco => {
            let start = poses.pose(&options.start_pose)?;
            let bridge = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("mujoco_bridge.py");
            let driver = MujocoDriver::launch(&options.python, bridge, &options.scene_file, start)?;
            serve_driver(driver, poses, motions, &options, "mujoco")
        }
    }
}

fn serve_driver<D: RuntimeDriver>(
    driver: D,
    poses: PoseLibrary,
    motions: MotionLibrary,
    options: &Options,
    backend: &str,
) -> orion_runtime::Result<i32> {
    let mut core = RuntimeCore::new(driver, poses, motions)?;
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
        core.tick(started_at.elapsed().as_secs_f64())?;
        server.serve_pending(|command| {
            core.handle_command(command, started_at.elapsed().as_secs_f64())
        })?;
        thread::sleep(next_sample.saturating_duration_since(Instant::now()));
    }
    Ok(0)
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
    use super::*;

    fn parse(values: &[&str]) -> orion_runtime::Result<Options> {
        parse_options(values.iter().map(|value| (*value).to_owned()))
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
        ])
        .unwrap();
        assert_eq!(options.operation, Operation::Goto);
        assert_eq!(options.pose_name, "home");
        assert_eq!(options.duration_seconds, 1.25);
        assert_eq!(options.socket_path, PathBuf::from("/tmp/test.sock"));
    }

    #[test]
    fn rejects_multiple_operations_and_missing_values() {
        assert!(parse(&["--status", "--enable"]).is_err());
        assert!(parse(&["--goto"]).is_err());
        assert!(parse(&["--duration", "fast"]).is_err());
    }

    #[test]
    fn parses_mujoco_backend_without_requiring_hardware_calibration() {
        let options = parse(&["--serve", "--backend", "mujoco", "--start-pose", "home"]).unwrap();
        assert_eq!(options.backend, Backend::Mujoco);
        assert_eq!(options.start_pose, "home");
        assert!(options.calibration_file.as_os_str().is_empty());
    }
}
