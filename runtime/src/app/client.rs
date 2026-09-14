use super::OBSERVE_PERIOD;
use std::io::{self, Write};
use std::thread;

use super::options::Options;
use crate::request_daemon;

pub(super) const EXIT_DAEMON_REJECTED: i32 = 3;
pub(super) const EXIT_MOVEMENT_TIMED_OUT: i32 = 4;
pub(super) const EXIT_MOVEMENT_CANCELLED: i32 = 5;
pub(super) const EXIT_SCENE_FAILED: i32 = 6;

pub(super) fn request_scene(options: &Options) -> crate::Result<i32> {
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
            crate::Error::Runtime("Accepted Orion scene response did not include a run_id.".into())
        })?;
    wait_for_scene(&options.socket_path, run_id)
}

pub(super) fn wait_for_scene(socket_path: &std::path::Path, run_id: u64) -> crate::Result<i32> {
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
                    crate::Error::Runtime("Orion scene status did not include a state.".into())
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
            return Err(crate::Error::Runtime(format!(
                "Orion scene run {run_id} is no longer the active or most recent result."
            )));
        }
    }
}

pub(super) fn scene_state_exit_code(state: &str) -> crate::Result<Option<i32>> {
    match state {
        "executing" => Ok(None),
        "completed" => Ok(Some(0)),
        "timed_out" => Ok(Some(EXIT_MOVEMENT_TIMED_OUT)),
        "cancelled" => Ok(Some(EXIT_MOVEMENT_CANCELLED)),
        "failed" => Ok(Some(EXIT_SCENE_FAILED)),
        value => Err(crate::Error::Runtime(format!(
            "Unknown Orion scene state: {value}"
        ))),
    }
}

pub(super) fn print_response(response: String) -> crate::Result<i32> {
    let exit_code = daemon_response_exit_code(&response)?;
    print!("{response}");
    io::stdout().flush()?;
    Ok(exit_code)
}

pub(super) fn request_movement(options: &Options, command: &str) -> crate::Result<i32> {
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
            crate::Error::Runtime(
                "Accepted Orion movement response did not include a run_id.".into(),
            )
        })?;
    wait_for_movement(&options.socket_path, run_id)
}

pub(super) fn wait_for_movement(socket_path: &std::path::Path, run_id: u64) -> crate::Result<i32> {
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
                    crate::Error::Runtime("Orion movement status did not include a state.".into())
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
                    return Err(crate::Error::Runtime(format!(
                        "Unknown Orion movement state: {value}"
                    )));
                }
            }
        }
        if !found {
            return Err(crate::Error::Runtime(format!(
                "Orion movement run {run_id} is no longer the active or most recent result."
            )));
        }
    }
}

pub(super) fn daemon_response_exit_code(response: &str) -> crate::Result<i32> {
    let value: serde_json::Value = serde_json::from_str(response.trim())?;
    Ok(daemon_value_exit_code(&value))
}

pub(super) fn daemon_value_exit_code(value: &serde_json::Value) -> i32 {
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
        EXIT_DAEMON_REJECTED
    } else {
        0
    }
}

pub(super) fn error_exit_code(error: &crate::Error) -> i32 {
    if matches!(error, crate::Error::InvalidArgument(_)) {
        2
    } else {
        1
    }
}

pub(super) fn print_states(states: &[crate::JointState]) {
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
