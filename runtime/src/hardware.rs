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
    #[serde(default)]
    pub calibration_file: String,
    #[serde(default)]
    pub joint_profile: String,
    pub poses: String,
    pub user_poses: String,
    pub motions: String,
    pub scenes: String,
    pub scene: String,
    pub joints: std::collections::BTreeMap<String, JointProfile>,
    #[serde(default)]
    pub comfort_box: Option<crate::motion::ik::ComfortBoxConfig>,
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
    #[serde(default)]
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
            let mut profile: HardwareProfile =
                serde_json::from_str(source).expect("validated built-in hardware profile");
            if self == Self::V2 {
                #[derive(Deserialize)]
                struct Ids {
                    joints: std::collections::BTreeMap<String, Id>,
                }
                #[derive(Deserialize)]
                struct Id {
                    servo_id: u8,
                }
                let ids: Ids =
                    serde_json::from_str(include_str!("../../hardware/profiles/v2-joints.json"))
                        .expect("validated V2 joint profile");
                for (name, joint) in &mut profile.joints {
                    joint.servo_id = ids.joints[name].servo_id;
                }
            }
            profile
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
        let calibrations = if self == Self::V2 {
            crate::motion::calibration::load_joint_profile(path)?
        } else {
            load_calibration_file(path, &ORION_JOINT_NAMES)?
        };
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

/// Find immutable hardware resources beside this executable, including after
/// installation at a path unrelated to the builder's checkout.
pub fn resource_root() -> Result<std::path::PathBuf> {
    let executable = std::env::current_exe()?;
    if let Some(path) = executable
        .ancestors()
        .find(|path| path.join("hardware/profiles/v2.json").is_file())
    {
        return Ok(path.to_path_buf());
    }
    // Cargo can put test/build executables in a shared target directory outside
    // the source tree. This fallback is usable only while that source exists;
    // installed binaries find their own release above, independent of /src.
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    if source.join("hardware/profiles/v2.json").is_file() {
        return Ok(source.to_path_buf());
    }
    Err(Error::Runtime(
        "Cannot locate Orion release resources beside the executable.".into(),
    ))
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
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../hardware/profiles/v2-joints.json");
        let profile = HardwareVersion::V2.load_calibration(source).unwrap();
        assert!(profile.iter().all(|joint| joint.neutral_raw == 2047));
        assert_eq!(profile[3].servo_id, 5);
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
