use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::driver::{JointLimit, RuntimeDriver};
use crate::pose::JointPositions;
use crate::state::JointState;
use crate::{Error, ORION_JOINT_NAMES, Result};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SimulationMetrics {
    pub maximum_translation: f64,
    pub maximum_tilt: f64,
    pub maximum_height_change: f64,
    pub longest_contact_loss: f64,
    pub safe: bool,
    #[serde(default)]
    pub unsafe_reasons: Vec<String>,
}

/// MuJoCo-backed implementation of the same joint-space boundary used by the
/// physical STS3215 driver. The small Python worker owns MuJoCo because the
/// repository already has a reviewed native-simulation adapter in Python.
pub struct MujocoDriver {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    limits: BTreeMap<String, (f64, f64)>,
    configured: bool,
    active: bool,
    metrics: SimulationMetrics,
}

impl MujocoDriver {
    pub fn launch(
        python: impl AsRef<Path>,
        bridge: impl AsRef<Path>,
        scene: impl AsRef<Path>,
        start_positions: &JointPositions,
    ) -> Result<Self> {
        require_exact_joints(start_positions)?;
        let start_json = serde_json::to_string(start_positions)?;
        let mut child = Command::new(python.as_ref())
            .arg(bridge.as_ref())
            .arg("--scene")
            .arg(scene.as_ref())
            .arg("--start-json")
            .arg(start_json)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                Error::Runtime(format!(
                    "Could not start Orion MuJoCo bridge '{}': {error}",
                    bridge.as_ref().display()
                ))
            })?;
        let input = child.stdin.take().ok_or_else(|| {
            Error::Runtime("MuJoCo bridge did not provide a command stream.".into())
        })?;
        let output = child.stdout.take().ok_or_else(|| {
            Error::Runtime("MuJoCo bridge did not provide a response stream.".into())
        })?;
        let mut driver = Self {
            child,
            input: Some(input),
            output: BufReader::new(output),
            limits: BTreeMap::new(),
            configured: false,
            active: false,
            metrics: SimulationMetrics::default(),
        };
        let ready = driver.read_response()?;
        require_ok(&ready)?;
        let limit_values = ready
            .get("joint_limits")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::Runtime("MuJoCo bridge omitted joint limits.".into()))?;
        for name in ORION_JOINT_NAMES {
            let range = limit_values
                .get(name)
                .and_then(Value::as_array)
                .filter(|values| values.len() == 2)
                .ok_or_else(|| {
                    Error::Runtime(format!("MuJoCo bridge omitted limits for {name}."))
                })?;
            let minimum = range[0]
                .as_f64()
                .ok_or_else(|| Error::Runtime(format!("Invalid MuJoCo minimum for {name}.")))?;
            let maximum = range[1]
                .as_f64()
                .ok_or_else(|| Error::Runtime(format!("Invalid MuJoCo maximum for {name}.")))?;
            driver.limits.insert(name.to_owned(), (minimum, maximum));
        }
        Ok(driver)
    }

    pub fn metrics(&self) -> &SimulationMetrics {
        &self.metrics
    }

    fn exchange(&mut self, request: Value) -> Result<Value> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| Error::InvalidState("MuJoCo bridge is closed.".into()))?;
        serde_json::to_writer(&mut *input, &request)?;
        input.write_all(b"\n")?;
        input.flush()?;
        let response = self.read_response()?;
        require_ok(&response)?;
        Ok(response)
    }

    fn read_response(&mut self) -> Result<Value> {
        let mut line = String::new();
        if self.output.read_line(&mut line)? == 0 {
            let status = self.child.try_wait()?.map(|value| value.to_string());
            return Err(Error::Runtime(format!(
                "MuJoCo bridge closed its response stream{}.",
                status
                    .map(|value| format!(" with status {value}"))
                    .unwrap_or_default()
            )));
        }
        serde_json::from_str(&line).map_err(|error| {
            Error::Runtime(format!("Invalid MuJoCo bridge response: {error}: {line}"))
        })
    }

    fn decode_states(&mut self, response: &Value) -> Result<Vec<JointState>> {
        if let Some(metrics) = response.get("metrics") {
            self.metrics = serde_json::from_value(metrics.clone())?;
        }
        let values = response
            .get("joints")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Runtime("MuJoCo bridge omitted joint state.".into()))?;
        let states: Vec<JointState> = serde_json::from_value(Value::Array(values.clone()))?;
        if states.len() != ORION_JOINT_NAMES.len()
            || states
                .iter()
                .zip(ORION_JOINT_NAMES)
                .any(|(state, expected)| state.name != expected)
        {
            return Err(Error::Runtime(
                "MuJoCo bridge returned the wrong Orion joint contract.".into(),
            ));
        }
        Ok(states)
    }
}

impl RuntimeDriver for MujocoDriver {
    fn apply_servo_profile(&mut self) -> Result<()> {
        if self.active {
            return Err(Error::InvalidState(
                "MuJoCo driver must be inactive before configuration.".into(),
            ));
        }
        self.configured = true;
        Ok(())
    }

    fn activate(&mut self) -> Result<Vec<JointState>> {
        if !self.configured || self.active {
            return Err(Error::InvalidState(
                "MuJoCo driver must be configured and inactive before activation.".into(),
            ));
        }
        let response = self.exchange(json!({"command": "activate"}))?;
        self.active = true;
        self.decode_states(&response)
    }

    fn deactivate(&mut self) -> Result<()> {
        if self.active {
            self.exchange(json!({"command": "deactivate"}))?;
        }
        self.active = false;
        Ok(())
    }

    fn read(&mut self) -> Result<Vec<JointState>> {
        let response = self.exchange(json!({"command": "read", "advance": self.active}))?;
        self.decode_states(&response)
    }

    fn write(&mut self, positions_radians: &JointPositions) -> Result<()> {
        if !self.active {
            return Err(Error::InvalidState("MuJoCo driver is not active.".into()));
        }
        self.validate_positions(positions_radians)?;
        self.exchange(json!({"command": "write", "positions": positions_radians}))?;
        Ok(())
    }

    fn joint_limits(&self) -> Result<Vec<JointLimit>> {
        Ok(ORION_JOINT_NAMES
            .iter()
            .map(|name| {
                let (lower_rad, upper_rad) = self.limits[*name];
                JointLimit {
                    name: (*name).to_owned(),
                    lower_rad,
                    upper_rad,
                }
            })
            .collect())
    }

    fn validate_positions(&self, positions_radians: &JointPositions) -> Result<()> {
        require_exact_joints(positions_radians)?;
        for (name, value) in positions_radians {
            let (minimum, maximum) = self.limits[name];
            if !value.is_finite() || value < &minimum || value > &maximum {
                return Err(Error::OutOfRange(format!(
                    "{name} command is outside its MuJoCo joint range."
                )));
            }
        }
        Ok(())
    }

    fn clamp_positions_to_safe_range(
        &self,
        positions_radians: &JointPositions,
    ) -> Result<JointPositions> {
        require_exact_joints(positions_radians)?;
        positions_radians
            .iter()
            .map(|(name, value)| {
                if !value.is_finite() {
                    return Err(Error::InvalidArgument(format!(
                        "{name} position must be finite."
                    )));
                }
                let (minimum, maximum) = self.limits[name];
                Ok((name.clone(), value.clamp(minimum, maximum)))
            })
            .collect()
    }
}

impl Drop for MujocoDriver {
    fn drop(&mut self) {
        if let Some(mut input) = self.input.take() {
            let _ = serde_json::to_writer(&mut input, &json!({"command": "shutdown"}));
            let _ = input.write_all(b"\n");
            let _ = input.flush();
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn require_exact_joints(positions: &JointPositions) -> Result<()> {
    if positions.len() != ORION_JOINT_NAMES.len()
        || ORION_JOINT_NAMES
            .iter()
            .any(|name| !positions.contains_key(*name))
    {
        return Err(Error::InvalidArgument(
            "A position is required for every Orion joint.".into(),
        ));
    }
    Ok(())
}

fn require_ok(response: &Value) -> Result<()> {
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    Err(Error::Runtime(
        response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("MuJoCo bridge request failed.")
            .to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MotionLibrary, PoseLibrary, RuntimeCore, RuntimeMode};

    #[test]
    fn rust_runtime_executes_and_settles_in_native_mujoco() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let poses =
            PoseLibrary::load(root.join("motion/config/poses.yaml"), &ORION_JOINT_NAMES).unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions"), &poses).unwrap();
        let python = root.join(".venv/bin/python");
        let bridge = root.join("runtime/mujoco_bridge.py");
        let scene = root.join("simulation/mujoco/scene.xml");
        let start = poses.pose("attentive").unwrap();
        let driver = MujocoDriver::launch(python, bridge, scene, start).unwrap();
        let mut core = RuntimeCore::new(driver, poses.clone(), motions).unwrap();

        assert!(
            core.handle_command("configure", 0.0)
                .contains("\"ok\":true")
        );
        assert!(core.handle_command("enable", 0.0).contains("\"ok\":true"));
        assert!(
            core.handle_command("play look_at_left", 0.0)
                .contains("\"ok\":true")
        );
        for step in 0..=225 {
            core.tick(step as f64 * 0.02).unwrap();
        }

        assert_eq!(core.mode(), RuntimeMode::Holding);
        let target = poses.pose("look_left").unwrap();
        for joint in &core.snapshot().joints {
            assert!(
                (joint.position - target[&joint.name]).abs() <= 0.05,
                "{} failed to settle: target={} measured={}",
                joint.name,
                target[&joint.name],
                joint.position
            );
            assert!(joint.velocity.abs() <= 0.05);
        }
        let metrics = core.driver().metrics();
        assert!(
            metrics.safe,
            "MuJoCo stability failure: {:?}",
            metrics.unsafe_reasons
        );
        assert!(metrics.maximum_translation <= 0.01);
        assert!(metrics.maximum_tilt <= 0.0872664626);
    }
}
