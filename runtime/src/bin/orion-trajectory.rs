use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use orion_runtime::{
    Error, JointLimit, JointPositions, MotionLibrary, MotionSequence, MotionSpace,
    ORION_JOINT_NAMES, PoseLibrary, Result, STS3215_MAX_SPEED_RAD_S, load_calibration_file,
};
use serde::{Deserialize, Serialize};

const DEFAULT_CONTROL_RATE_HZ: f64 = 50.0;

#[derive(Debug)]
struct Arguments {
    pose_file: PathBuf,
    motions_directory: PathBuf,
    calibration_file: PathBuf,
    motion_name: String,
    start_pose: Option<String>,
    anchor_pose: Option<String>,
    start_state: Option<PathBuf>,
    control_rate_hz: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartStateDocument {
    positions: JointPositions,
    velocities: JointPositions,
    anchor: JointPositions,
}

#[derive(Debug, Serialize)]
struct HardwareProfile<'a> {
    variant: &'a str,
    encoder_counts_per_revolution: u32,
    maximum_no_load_speed_rpm: u32,
    maximum_velocity_rad_s: f64,
    rated_torque_kg_cm: f64,
    stall_torque_kg_cm: f64,
    runtime_control_rate_hz: f64,
}

#[derive(Debug, Serialize)]
struct JointRange<'a> {
    name: &'a str,
    lower_rad: f64,
    upper_rad: f64,
}

#[derive(Debug, Serialize)]
struct MarkerExport {
    name: String,
    time_seconds: f64,
}

#[derive(Debug, Serialize)]
struct SampleExport {
    time_from_start: f64,
    positions: Vec<f64>,
    velocities: Vec<f64>,
    accelerations: Vec<f64>,
    keyframe_index: usize,
    keyframe: String,
    reached_markers: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TrajectoryExport<'a> {
    format_version: u32,
    compiler: &'a str,
    build_revision: &'a str,
    motion_name: &'a str,
    description: &'a str,
    space: &'a str,
    style: &'a str,
    joint_names: Vec<&'a str>,
    duration_seconds: f64,
    control_rate_hz: f64,
    peak_velocity_rad_s: f64,
    amplitude_scale: f64,
    hardware_profile: HardwareProfile<'a>,
    joint_ranges: Vec<JointRange<'a>>,
    markers: Vec<MarkerExport>,
    samples: Vec<SampleExport>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("orion-trajectory: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let arguments = parse_arguments(env::args().skip(1))?;
    let poses = PoseLibrary::load(&arguments.pose_file, &ORION_JOINT_NAMES)?;
    let motions = MotionLibrary::load(&arguments.motions_directory, &poses)?;
    let motion = motions.motion(&arguments.motion_name)?;
    let calibrations = load_calibration_file(&arguments.calibration_file, &ORION_JOINT_NAMES)?;
    let limits: Vec<JointLimit> = calibrations
        .iter()
        .map(|calibration| {
            let (lower_rad, upper_rad) = calibration.safe_range_radians();
            JointLimit {
                name: calibration.name.clone(),
                lower_rad,
                upper_rad,
            }
        })
        .collect();

    let (start, start_velocity, anchor) = load_start_state(&arguments, &poses)?;
    validate_state_against_calibration("start", &start, &limits)?;
    validate_state_against_calibration("anchor", &anchor, &limits)?;
    let amplitude_scale = motion.uniform_amplitude_scale(&anchor, &limits)?;
    let sequence = MotionSequence::compile_scaled_calibrated(
        motion,
        start,
        start_velocity,
        anchor,
        amplitude_scale,
        &limits,
    )?;

    let sample_times =
        fixed_rate_sample_times(sequence.duration_seconds(), arguments.control_rate_hz);
    let mut samples = Vec::with_capacity(sample_times.len());
    for elapsed in sample_times {
        let state = sequence.sample_state(elapsed)?;
        validate_state_against_calibration("compiled trajectory", &state.positions, &limits)?;
        samples.push(SampleExport {
            time_from_start: elapsed,
            positions: ordered_values(&state.positions)?,
            velocities: ordered_values(&state.velocities)?,
            accelerations: ordered_values(&state.accelerations)?,
            keyframe_index: sequence.keyframe_index(elapsed)?,
            keyframe: sequence.keyframe_name(elapsed)?.to_owned(),
            reached_markers: sequence.reached_markers(elapsed),
        });
    }

    let markers = motion
        .markers()
        .iter()
        .filter_map(|name| {
            sequence.marker_time(name).map(|time_seconds| MarkerExport {
                name: name.clone(),
                time_seconds,
            })
        })
        .collect();
    let space = match motion.space {
        MotionSpace::Absolute => "absolute",
        MotionSpace::AnchorRelative => "anchor_relative",
    };
    let export = TrajectoryExport {
        format_version: 2,
        compiler: "orion-runtime",
        build_revision: orion_runtime::BUILD_REVISION,
        motion_name: &motion.name,
        description: &motion.description,
        space,
        style: motion.style.name,
        joint_names: ORION_JOINT_NAMES.to_vec(),
        duration_seconds: sequence.duration_seconds(),
        control_rate_hz: arguments.control_rate_hz,
        peak_velocity_rad_s: sequence.peak_velocity_rad_s(),
        amplitude_scale,
        hardware_profile: HardwareProfile {
            variant: "7.4 V STS3215",
            encoder_counts_per_revolution: 4096,
            maximum_no_load_speed_rpm: 52,
            maximum_velocity_rad_s: STS3215_MAX_SPEED_RAD_S,
            rated_torque_kg_cm: 5.0,
            stall_torque_kg_cm: 19.5,
            runtime_control_rate_hz: DEFAULT_CONTROL_RATE_HZ,
        },
        joint_ranges: limits
            .iter()
            .map(|limit| JointRange {
                name: &limit.name,
                lower_rad: limit.lower_rad,
                upper_rad: limit.upper_rad,
            })
            .collect(),
        markers,
        samples,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&export)
            .map_err(|error| Error::Runtime(format!("Could not serialize trajectory: {error}")))?
    );
    Ok(())
}

fn parse_arguments(arguments: impl Iterator<Item = String>) -> Result<Arguments> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("runtime has a repository parent");
    let mut pose_file = root.join("motion/config/poses.yaml");
    let mut motions_directory = root.join("motion/motions");
    let mut calibration_file = root.join("simulation/mujoco/config/servo_calibration.json");
    let mut motion_name = None;
    let mut start_pose = None;
    let mut anchor_pose = None;
    let mut start_state = None;
    let mut control_rate_hz = DEFAULT_CONTROL_RATE_HZ;
    let mut arguments = arguments.peekable();
    while let Some(flag) = arguments.next() {
        let value = || Error::InvalidArgument(format!("{flag} requires a value."));
        match flag.as_str() {
            "--pose-file" => pose_file = PathBuf::from(arguments.next().ok_or_else(value)?),
            "--motions-directory" => {
                motions_directory = PathBuf::from(arguments.next().ok_or_else(value)?)
            }
            "--calibration" => {
                calibration_file = PathBuf::from(arguments.next().ok_or_else(value)?)
            }
            "--motion" => motion_name = Some(arguments.next().ok_or_else(value)?),
            "--start-pose" => start_pose = Some(arguments.next().ok_or_else(value)?),
            "--anchor-pose" => anchor_pose = Some(arguments.next().ok_or_else(value)?),
            "--start-state" => {
                start_state = Some(PathBuf::from(arguments.next().ok_or_else(value)?))
            }
            "--control-rate-hz" => {
                control_rate_hz = arguments.next().ok_or_else(value)?.parse().map_err(|_| {
                    Error::InvalidArgument("--control-rate-hz must be a number.".into())
                })?;
            }
            "--help" | "-h" => return Err(Error::InvalidArgument(usage().into())),
            _ => {
                return Err(Error::InvalidArgument(format!(
                    "Unknown argument '{flag}'.\n{}",
                    usage()
                )));
            }
        }
    }
    if !control_rate_hz.is_finite() || control_rate_hz <= 0.0 || control_rate_hz > 1000.0 {
        return Err(Error::InvalidArgument(
            "Control rate must be finite and in 0..=1000 Hz.".into(),
        ));
    }
    if start_state.is_some() == start_pose.is_some() {
        return Err(Error::InvalidArgument(
            "Provide exactly one of --start-pose or --start-state.".into(),
        ));
    }
    if start_state.is_some() && anchor_pose.is_some() {
        return Err(Error::InvalidArgument(
            "--start-state already contains an anchor; do not combine it with --anchor-pose."
                .into(),
        ));
    }
    Ok(Arguments {
        pose_file,
        motions_directory,
        calibration_file,
        motion_name: motion_name
            .ok_or_else(|| Error::InvalidArgument("--motion is required.".into()))?,
        start_pose,
        anchor_pose,
        start_state,
        control_rate_hz,
    })
}

fn load_start_state(
    arguments: &Arguments,
    poses: &PoseLibrary,
) -> Result<(JointPositions, JointPositions, JointPositions)> {
    if let Some(path) = &arguments.start_state {
        let bytes = fs::read(path).map_err(|error| {
            Error::Runtime(format!(
                "Could not read start state '{}': {error}",
                path.display()
            ))
        })?;
        let state: StartStateDocument = serde_json::from_slice(&bytes).map_err(|error| {
            Error::Runtime(format!(
                "Could not parse start state '{}': {error}",
                path.display()
            ))
        })?;
        return Ok((state.positions, state.velocities, state.anchor));
    }
    let start_name = arguments
        .start_pose
        .as_deref()
        .expect("validated by argument parser");
    let start = poses.pose(start_name)?.clone();
    let anchor = poses
        .pose(arguments.anchor_pose.as_deref().unwrap_or(start_name))?
        .clone();
    let velocity = start.keys().map(|name| (name.clone(), 0.0)).collect();
    Ok((start, velocity, anchor))
}

fn validate_state_against_calibration(
    label: &str,
    positions: &JointPositions,
    limits: &[JointLimit],
) -> Result<()> {
    if positions.len() != ORION_JOINT_NAMES.len() {
        return Err(Error::InvalidArgument(format!(
            "{label} must contain every Orion joint."
        )));
    }
    for limit in limits {
        let value = positions
            .get(&limit.name)
            .ok_or_else(|| Error::InvalidArgument(format!("{label} omits {}.", limit.name)))?;
        if !value.is_finite() || *value < limit.lower_rad - 1e-9 || *value > limit.upper_rad + 1e-9
        {
            return Err(Error::InvalidArgument(format!(
                "{label} {} position {value:.6} is outside calibrated range {:.6}..{:.6}.",
                limit.name, limit.lower_rad, limit.upper_rad
            )));
        }
    }
    Ok(())
}

fn ordered_values(values: &BTreeMap<String, f64>) -> Result<Vec<f64>> {
    ORION_JOINT_NAMES
        .iter()
        .map(|name| {
            values
                .get(*name)
                .copied()
                .ok_or_else(|| Error::InvalidArgument(format!("Compiled sample omits {name}.")))
        })
        .collect()
}

fn fixed_rate_sample_times(duration_seconds: f64, control_rate_hz: f64) -> Vec<f64> {
    let period = 1.0 / control_rate_hz;
    let intervals = (duration_seconds / period).floor() as usize;
    let mut times: Vec<f64> = (0..=intervals).map(|index| index as f64 * period).collect();
    if times
        .last()
        .is_none_or(|last| duration_seconds - last > 1e-9)
    {
        times.push(duration_seconds);
    } else if let Some(last) = times.last_mut() {
        *last = duration_seconds;
    }
    times
}

fn usage() -> &'static str {
    "Usage: orion-trajectory --motion NAME (--start-pose POSE [--anchor-pose POSE] | --start-state FILE) [--pose-file FILE] [--motions-directory DIR] [--calibration FILE] [--control-rate-hz HZ]"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_rate_samples_include_exact_final_time() {
        assert_eq!(
            fixed_rate_sample_times(0.045, 50.0),
            vec![0.0, 0.02, 0.04, 0.045]
        );
        assert_eq!(fixed_rate_sample_times(0.04, 50.0), vec![0.0, 0.02, 0.04]);
    }
}
