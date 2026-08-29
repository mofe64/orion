use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::{Error, Result};

pub const ENCODER_RESOLUTION: i32 = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JointCalibration {
    pub name: String,
    pub servo_id: u8,
    pub neutral_raw: i32,
    pub encoder_direction: i32,
    pub safe_min_delta_raw: i32,
    pub safe_max_delta_raw: i32,
}

#[derive(Debug, Deserialize)]
struct CalibrationDocument {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    robot: String,
    #[serde(default)]
    servo_model: String,
    #[serde(default)]
    encoder_resolution: i32,
    writes_servo_eeprom: Option<bool>,
    #[serde(default)]
    joints: BTreeMap<String, CalibrationEntry>,
}

#[derive(Debug, Deserialize)]
struct CalibrationEntry {
    servo_id: i32,
    neutral_raw: i32,
    encoder_direction: i32,
    safe_min_delta_raw: i32,
    safe_max_delta_raw: i32,
}

pub fn load_calibration_file(
    path: impl AsRef<Path>,
    expected_joint_names: &[impl AsRef<str>],
) -> Result<Vec<JointCalibration>> {
    let path = path.as_ref();
    let contents = fs::read_to_string(path).map_err(|error| {
        Error::Runtime(format!(
            "Could not parse calibration '{}': {error}",
            path.display()
        ))
    })?;
    let document: CalibrationDocument = serde_json::from_str(&contents).map_err(|error| {
        Error::Runtime(format!(
            "Could not parse calibration '{}': {error}",
            path.display()
        ))
    })?;

    if document.schema_version != 1 {
        return Err(Error::Runtime(
            "Calibration must use schema_version 1.".into(),
        ));
    }
    if document.robot != "orion" || document.servo_model != "sts3215" {
        return Err(Error::Runtime(
            "Calibration is not for Orion STS3215 hardware.".into(),
        ));
    }
    if document.encoder_resolution != ENCODER_RESOLUTION {
        return Err(Error::Runtime(
            "Calibration must use the STS3215 4096-count encoder.".into(),
        ));
    }
    if document.writes_servo_eeprom != Some(false) {
        return Err(Error::Runtime(
            "Calibration must retain software-only EEPROM provenance.".into(),
        ));
    }

    let expected: BTreeSet<String> = expected_joint_names
        .iter()
        .map(|name| name.as_ref().to_owned())
        .collect();
    let present: BTreeSet<String> = document.joints.keys().cloned().collect();
    if present != expected {
        return Err(Error::Runtime(
            "Calibration joint names do not match Orion's configured joints.".into(),
        ));
    }

    let mut servo_ids = BTreeSet::new();
    let mut calibrations = Vec::with_capacity(expected_joint_names.len());
    for joint_name in expected_joint_names {
        let name = joint_name.as_ref();
        let joint = &document.joints[name];
        if !(1..=252).contains(&joint.servo_id) || !servo_ids.insert(joint.servo_id) {
            return Err(Error::Runtime(format!(
                "{name} has an invalid or duplicate servo ID."
            )));
        }
        if !(0..ENCODER_RESOLUTION).contains(&joint.neutral_raw) {
            return Err(Error::Runtime(format!(
                "{name} neutral_raw is outside 0..4095."
            )));
        }
        if !matches!(joint.encoder_direction, -1 | 1) {
            return Err(Error::Runtime(format!(
                "{name} encoder_direction must be -1 or +1."
            )));
        }
        if joint.safe_min_delta_raw >= 0
            || joint.safe_max_delta_raw <= 0
            || joint.safe_min_delta_raw <= -2048
            || joint.safe_max_delta_raw >= 2048
        {
            return Err(Error::Runtime(format!(
                "{name} safe range must contain zero and stay inside one half-turn."
            )));
        }
        calibrations.push(JointCalibration {
            name: name.to_owned(),
            servo_id: joint.servo_id as u8,
            neutral_raw: joint.neutral_raw,
            encoder_direction: joint.encoder_direction,
            safe_min_delta_raw: joint.safe_min_delta_raw,
            safe_max_delta_raw: joint.safe_max_delta_raw,
        });
    }
    Ok(calibrations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ORION_JOINT_NAMES;

    #[test]
    fn loads_tracked_orion_calibration_in_joint_order() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../simulation/mujoco/config/servo_calibration.json"
        );
        let calibrations = load_calibration_file(path, &ORION_JOINT_NAMES).unwrap();

        assert_eq!(calibrations.len(), 5);
        assert_eq!(calibrations[0].name, "base_yaw_joint");
        assert_eq!(calibrations[0].servo_id, 1);
        assert_eq!(calibrations[4].name, "head_pitch_joint");
        assert_eq!(calibrations[4].servo_id, 5);
    }
}
