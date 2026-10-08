//! Hardware selection preserves the Studio joint-key contract. The profile
//! names the physical mechanism; v2 head_roll is neck swivel, head_pitch wrist.
use crate::{Error, JointCalibration, ORION_JOINT_NAMES, Result, load_calibration_file};
use serde::Deserialize;
use std::{path::Path, str::FromStr, sync::OnceLock};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HardwareVersion {
    #[default]
    V1,
    V2,
}

#[derive(Debug, Deserialize)]
pub struct HardwareProfile {
    pub hardware: String,
    pub mechanical_revision: String,
    pub lighting: LightingProfile,
    pub audio: AudioProfile,
    pub calibration_file: String,
    pub poses: String,
    pub user_poses: String,
    pub motions: String,
    pub scenes: String,
    pub scene: String,
    pub joints: std::collections::BTreeMap<String, JointProfile>,
}
#[derive(Debug, Deserialize)]
pub struct LightingProfile {
    pub layout: String,
    pub pixel_count: usize,
    pub width: usize,
    pub height: usize,
    pub white_temperature_k: u16,
    pub gpio_bcm: u8,
    pub wire_order: String,
    pub frequency_hz: usize,
}
#[derive(Debug, Deserialize)]
pub struct AudioProfile {
    pub backend: String,
    pub card: String,
    pub pcm: String,
    pub capture_channels: usize,
    pub processed_channel: Option<usize>,
}
#[derive(Debug, Deserialize)]
pub struct JointProfile {
    pub servo_id: u8,
    pub physical_joint: String,
}

impl FromStr for HardwareVersion {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "v1" => Ok(Self::V1),
            "v2" => Ok(Self::V2),
            _ => Err(Error::InvalidArgument(
                "--hardware must be v1 or v2.".into(),
            )),
        }
    }
}
impl HardwareVersion {
    pub fn profile(self) -> &'static HardwareProfile {
        static V1: OnceLock<HardwareProfile> = OnceLock::new();
        static V2: OnceLock<HardwareProfile> = OnceLock::new();
        let (slot, source) = match self {
            Self::V1 => (&V1, include_str!("../../hardware/profiles/v1.json")),
            Self::V2 => (&V2, include_str!("../../hardware/profiles/v2.json")),
        };
        slot.get_or_init(|| {
            serde_json::from_str(source).expect("validated built-in hardware profile")
        })
    }
    pub fn load_calibration(self, path: impl AsRef<Path>) -> Result<Vec<JointCalibration>> {
        let document: serde_json::Value = serde_json::from_slice(&std::fs::read(path.as_ref())?)?;
        let version = document
            .get("hardware")
            .and_then(|v| v.as_str())
            .unwrap_or("v1");
        if version != self.profile().hardware
            || document.get("simulation_only") == Some(&serde_json::Value::Bool(true))
        {
            return Err(Error::Runtime(
                "Calibration hardware does not match --hardware, or is simulation-only.".into(),
            ));
        }
        let calibrations = load_calibration_file(path, &ORION_JOINT_NAMES)?;
        // Each servo ID must drive the mechanism its joint name describes; a
        // calibration taken under another ID map would move the wrong servo.
        for calibration in &calibrations {
            let expected = self.profile().joints[&calibration.name].servo_id;
            if calibration.servo_id != expected {
                return Err(Error::Runtime(format!(
                    "{} uses servo {} in the calibration, but the {} profile maps it to servo {expected}.",
                    calibration.name,
                    calibration.servo_id,
                    self.profile().hardware
                )));
            }
        }
        Ok(calibrations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v2_refuses_v1_calibration_before_opening_a_bus() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../simulation/mujoco/config/servo_calibration.json");
        assert!(HardwareVersion::V1.load_calibration(&source).is_ok());
        assert!(HardwareVersion::V2.load_calibration(&source).is_err());
        let mut document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(source).unwrap()).unwrap();
        document["hardware"] = serde_json::json!("v2");
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("calibration.json");
        // V1 IDs put the neck joint on servo 4; V2 measured it on servo 5.
        std::fs::write(&path, document.to_string()).unwrap();
        let error = HardwareVersion::V2.load_calibration(&path).unwrap_err();
        assert!(error.to_string().contains("maps it to servo 5"), "{error}");
        document["joints"]["head_roll_joint"]["servo_id"] = serde_json::json!(5);
        document["joints"]["head_pitch_joint"]["servo_id"] = serde_json::json!(4);
        std::fs::write(&path, document.to_string()).unwrap();
        assert!(HardwareVersion::V2.load_calibration(&path).is_ok());
        document["simulation_only"] = serde_json::json!(true);
        std::fs::write(&path, document.to_string()).unwrap();
        assert!(HardwareVersion::V2.load_calibration(&path).is_err());
    }
    #[test]
    fn profiles_preserve_contract_and_name_the_v2_physical_axes() {
        for version in [HardwareVersion::V1, HardwareVersion::V2] {
            let profile = version.profile();
            assert_eq!(
                profile.lighting.pixel_count,
                profile.lighting.width * profile.lighting.height
            );
            assert_eq!(profile.lighting.gpio_bcm, 12);
            assert_eq!(profile.lighting.wire_order, "GRBW");
            assert_eq!(profile.joints.len(), 5);
            for name in ORION_JOINT_NAMES {
                assert!(profile.joints.contains_key(name));
            }
        }
        let v2 = HardwareVersion::V2.profile();
        assert_eq!(v2.joints["head_pitch_joint"].physical_joint, "wrist_pitch");
        assert_eq!(v2.joints["head_roll_joint"].physical_joint, "neck_swivel");
        // Measured on the fitted V2 lamp: servo 4 tilts the head, servo 5 turns it.
        assert_eq!(v2.joints["head_pitch_joint"].servo_id, 4);
        assert_eq!(v2.joints["head_roll_joint"].servo_id, 5);
    }
}
