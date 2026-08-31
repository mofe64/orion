use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

use serde::Serialize;

use crate::calibration::{ENCODER_RESOLUTION, JointCalibration};
use crate::pose::JointPositions;
use crate::state::JointState;
use crate::transport::{Register, Sts3215RawState, Sts3215Transport};
use crate::{Error, Result};

pub const STS3215_MODEL_NUMBER: i32 = 777;
pub const VELOCITY_RAW_TO_RADIANS_PER_SECOND: f64 = 0.732 * 2.0 * PI / 60.0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JointServoProfile {
    pub return_delay_time: i32,
    pub operating_mode: i32,
    pub drive_mode: i32,
    pub p_coefficient: i32,
    pub i_coefficient: i32,
    pub d_coefficient: i32,
    pub maximum_acceleration: i32,
    pub acceleration: i32,
}

impl Default for JointServoProfile {
    fn default() -> Self {
        Self {
            return_delay_time: 0,
            operating_mode: 0,
            drive_mode: 0,
            p_coefficient: 16,
            i_coefficient: 0,
            d_coefficient: 32,
            maximum_acceleration: 254,
            acceleration: 254,
        }
    }
}

pub type ServoProfiles = BTreeMap<String, JointServoProfile>;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JointLimit {
    pub name: String,
    pub lower_rad: f64,
    pub upper_rad: f64,
}

/// The hardware-facing operations used by the daemon state machine.
///
/// Keeping this boundary in joint-space lets the same daemon logic drive the
/// real STS3215 bus, deterministic tests, and the MuJoCo validation backend.
pub trait RuntimeDriver {
    fn apply_servo_profile(&mut self) -> Result<()>;
    fn activate(&mut self) -> Result<Vec<JointState>>;
    fn deactivate(&mut self) -> Result<()>;
    fn read(&mut self) -> Result<Vec<JointState>>;
    fn write(&mut self, positions_radians: &JointPositions) -> Result<()>;
    fn joint_limits(&self) -> Result<Vec<JointLimit>>;
    fn validate_positions(&self, positions_radians: &JointPositions) -> Result<()>;
    fn clamp_positions_to_safe_range(
        &self,
        positions_radians: &JointPositions,
    ) -> Result<JointPositions>;
}

pub fn make_orion_servo_profiles() -> ServoProfiles {
    let mut profiles: ServoProfiles = crate::ORION_JOINT_NAMES
        .iter()
        .map(|name| ((*name).to_owned(), JointServoProfile::default()))
        .collect();
    profiles.get_mut("elbow_pitch_joint").unwrap().p_coefficient = 32;
    profiles
}

pub struct Sts3215Driver<T: Sts3215Transport> {
    transport: T,
    servo_profiles: ServoProfiles,
    calibrations: Vec<JointCalibration>,
    configured: bool,
    profile_applied: bool,
    active: bool,
}

impl<T: Sts3215Transport> Sts3215Driver<T> {
    pub fn new(transport: T) -> Self {
        Self::with_profiles(transport, make_orion_servo_profiles())
    }

    pub fn with_profiles(transport: T, servo_profiles: ServoProfiles) -> Self {
        Self {
            transport,
            servo_profiles,
            calibrations: Vec::new(),
            configured: false,
            profile_applied: false,
            active: false,
        }
    }

    pub fn configure(
        &mut self,
        port: &str,
        baud_rate: i32,
        calibrations: Vec<JointCalibration>,
    ) -> Result<()> {
        self.connect(port, baud_rate, calibrations)?;
        self.apply_servo_profile()
    }

    pub fn connect(
        &mut self,
        port: &str,
        baud_rate: i32,
        calibrations: Vec<JointCalibration>,
    ) -> Result<()> {
        if port.is_empty() || baud_rate <= 0 || calibrations.is_empty() {
            return Err(Error::InvalidArgument(
                "Port, baud rate, and calibrations are required.".into(),
            ));
        }
        let mut names = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for joint in &calibrations {
            if !names.insert(joint.name.clone()) || !ids.insert(joint.servo_id) {
                return Err(Error::InvalidArgument(
                    "Joint names and servo IDs must be unique.".into(),
                ));
            }
        }

        self.close();
        self.transport.open(port, baud_rate)?;
        let validation = (|| {
            let mut firmware = None;
            for joint in &calibrations {
                let id = joint.servo_id;
                if self.transport.read_register(id, Register::ModelNumber)? != STS3215_MODEL_NUMBER
                {
                    return Err(Error::Runtime(format!("Servo {id} is not an STS3215.")));
                }
                if self.transport.read_register(id, Register::TorqueEnable)? != 0 {
                    return Err(Error::Runtime(format!(
                        "Servo {id} must have torque off during configuration."
                    )));
                }
                if self.transport.read_register(id, Register::Status)? != 0 {
                    return Err(Error::Runtime(format!(
                        "Servo {id} reported a fault before configuration."
                    )));
                }
                let version = (
                    self.transport
                        .read_register(id, Register::FirmwareMajorVersion)?,
                    self.transport
                        .read_register(id, Register::FirmwareMinorVersion)?,
                );
                if firmware.is_some_and(|expected| expected != version) {
                    return Err(Error::Runtime(
                        "All five STS3215 servos must use the same firmware.".into(),
                    ));
                }
                firmware = Some(version);
            }
            Ok(())
        })();
        if let Err(error) = validation {
            self.close();
            return Err(error);
        }
        self.calibrations = calibrations;
        self.configured = true;
        Ok(())
    }

    pub fn apply_servo_profile(&mut self) -> Result<()> {
        if !self.configured || self.active {
            return Err(Error::InvalidState(
                "STS3215 driver must be connected and inactive before applying its profile.".into(),
            ));
        }
        let result = self.apply_servo_profile_inner();
        if result.is_err() {
            self.close();
        }
        result
    }

    fn apply_servo_profile_inner(&mut self) -> Result<()> {
        let ids = self.servo_ids();
        let mut persistent_writes = Vec::new();
        for joint in &self.calibrations {
            let Some(profile) = self.servo_profiles.get(&joint.name) else {
                return Err(Error::Runtime(format!(
                    "Missing STS3215 servo profile for {}.",
                    joint.name
                )));
            };
            if !matches!(profile.drive_mode, 0 | 1) {
                return Err(Error::Runtime(format!(
                    "STS3215 drive mode must be 0 or 1 for {}.",
                    joint.name
                )));
            }
            let desired = [
                (Register::ReturnDelayTime, profile.return_delay_time),
                (Register::OperatingMode, profile.operating_mode),
                (Register::PCoefficient, profile.p_coefficient),
                (Register::ICoefficient, profile.i_coefficient),
                (Register::DCoefficient, profile.d_coefficient),
                (Register::MaximumAcceleration, profile.maximum_acceleration),
            ];
            for (register, value) in desired {
                if self.transport.read_register(joint.servo_id, register)? != value {
                    persistent_writes.push((joint.servo_id, register, value));
                }
            }
            let phase = self
                .transport
                .read_register(joint.servo_id, Register::Phase)?;
            let desired_phase = if profile.drive_mode == 0 {
                phase & !0x10
            } else {
                phase | 0x10
            };
            if phase != desired_phase {
                persistent_writes.push((joint.servo_id, Register::Phase, desired_phase));
            }
        }

        if !persistent_writes.is_empty() {
            self.transport.set_eeprom_lock(&ids, false)?;
            let writes = (|| {
                for (id, register, value) in &persistent_writes {
                    self.transport.write_register(*id, *register, *value)?;
                    if self.transport.read_register(*id, *register)? != *value {
                        return Err(Error::Runtime(
                            "STS3215 persistent configuration verification failed.".into(),
                        ));
                    }
                }
                Ok(())
            })();
            if writes.is_err() {
                let _ = self.transport.set_eeprom_lock(&ids, true);
                return writes;
            }
            self.transport.set_eeprom_lock(&ids, true)?;
        }

        for joint in &self.calibrations {
            let acceleration = self.servo_profiles[&joint.name].acceleration;
            if self
                .transport
                .read_register(joint.servo_id, Register::Acceleration)?
                != acceleration
            {
                self.transport.write_register(
                    joint.servo_id,
                    Register::Acceleration,
                    acceleration,
                )?;
            }
            if self
                .transport
                .read_register(joint.servo_id, Register::Acceleration)?
                != acceleration
            {
                return Err(Error::Runtime(
                    "STS3215 runtime acceleration verification failed.".into(),
                ));
            }
        }
        self.profile_applied = true;
        Ok(())
    }

    pub fn activate(&mut self) -> Result<Vec<JointState>> {
        if !self.configured || !self.profile_applied || self.active {
            return Err(Error::InvalidState(
                "STS3215 driver must have its profile applied and be inactive before activation."
                    .into(),
            ));
        }
        let ids = self.servo_ids();
        let raw_states = self.transport.read_states(&ids)?;
        let mut initial_goals = BTreeMap::new();
        for joint in &self.calibrations {
            let Some(raw) = raw_states.get(&joint.servo_id) else {
                return Err(Error::Runtime(
                    "Missing STS3215 state during activation.".into(),
                ));
            };
            if raw.status != 0 {
                return Err(Error::Runtime(
                    "STS3215 reported a fault during activation.".into(),
                ));
            }
            initial_goals.insert(joint.servo_id, raw.position);
        }
        self.transport.write_positions(&initial_goals)?;
        for (&id, &expected_goal) in &initial_goals {
            if self.transport.read_register(id, Register::GoalPosition)? != expected_goal {
                return Err(Error::Runtime(format!(
                    "STS3215 initial goal verification failed for servo {id}."
                )));
            }
        }
        if let Err(error) = self.transport.set_torque(&ids, true) {
            let _ = self.transport.set_torque(&ids, false);
            return Err(error);
        }
        self.active = true;
        self.convert_states(&raw_states)
    }

    pub fn deactivate(&mut self) -> Result<()> {
        if self.configured && self.transport.is_open() {
            self.transport.set_torque(&self.servo_ids(), false)?;
        }
        self.active = false;
        Ok(())
    }

    pub fn close(&mut self) {
        if self.transport.is_open() && self.active {
            let _ = self.transport.set_torque(&self.servo_ids(), false);
        }
        self.transport.close();
        self.configured = false;
        self.profile_applied = false;
        self.active = false;
    }

    pub fn read(&mut self) -> Result<Vec<JointState>> {
        if !self.configured {
            return Err(Error::InvalidState(
                "STS3215 driver is not configured.".into(),
            ));
        }
        let raw = self.transport.read_states(&self.servo_ids())?;
        self.convert_states(&raw)
    }

    pub fn write(&mut self, positions_radians: &JointPositions) -> Result<()> {
        if !self.active {
            return Err(Error::InvalidState("STS3215 driver is not active.".into()));
        }
        let encoded = self.encode_positions(positions_radians)?;
        self.transport.write_positions(&encoded)
    }

    pub fn validate_positions(&self, positions_radians: &JointPositions) -> Result<()> {
        if !self.configured {
            return Err(Error::InvalidState(
                "STS3215 driver is not configured.".into(),
            ));
        }
        self.encode_positions(positions_radians).map(|_| ())
    }

    pub fn joint_limits(&self) -> Result<Vec<JointLimit>> {
        if !self.configured {
            return Err(Error::InvalidState(
                "STS3215 driver is not configured.".into(),
            ));
        }
        let radians_per_step = 2.0 * PI / ENCODER_RESOLUTION as f64;
        Ok(self
            .calibrations
            .iter()
            .map(|joint| {
                let first = joint.safe_min_delta_raw as f64 * radians_per_step
                    / joint.encoder_direction as f64;
                let second = joint.safe_max_delta_raw as f64 * radians_per_step
                    / joint.encoder_direction as f64;
                JointLimit {
                    name: joint.name.clone(),
                    lower_rad: first.min(second),
                    upper_rad: first.max(second),
                }
            })
            .collect())
    }

    pub fn clamp_positions_to_safe_range(
        &self,
        positions_radians: &JointPositions,
    ) -> Result<JointPositions> {
        if !self.configured {
            return Err(Error::InvalidState(
                "STS3215 driver is not configured.".into(),
            ));
        }
        if positions_radians.len() != self.calibrations.len() {
            return Err(Error::InvalidArgument(
                "A position is required for every Orion joint.".into(),
            ));
        }
        let radians_per_step = 2.0 * PI / ENCODER_RESOLUTION as f64;
        self.calibrations
            .iter()
            .map(|joint| {
                let value = positions_radians.get(&joint.name).ok_or_else(|| {
                    Error::InvalidArgument(format!("Missing position for {}.", joint.name))
                })?;
                if !value.is_finite() {
                    return Err(Error::InvalidArgument(format!(
                        "{} position must be finite.",
                        joint.name
                    )));
                }
                let first = joint.safe_min_delta_raw as f64 * radians_per_step
                    / joint.encoder_direction as f64;
                let second = joint.safe_max_delta_raw as f64 * radians_per_step
                    / joint.encoder_direction as f64;
                Ok((
                    joint.name.clone(),
                    value.clamp(first.min(second), first.max(second)),
                ))
            })
            .collect()
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn calibrations(&self) -> &[JointCalibration] {
        &self.calibrations
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    fn servo_ids(&self) -> Vec<u8> {
        self.calibrations
            .iter()
            .map(|joint| joint.servo_id)
            .collect()
    }

    fn encode_positions(&self, positions_radians: &JointPositions) -> Result<BTreeMap<u8, i32>> {
        if positions_radians.len() != self.calibrations.len() {
            return Err(Error::InvalidArgument(
                "A position command is required for every Orion joint.".into(),
            ));
        }
        self.calibrations
            .iter()
            .map(|joint| {
                let radians = positions_radians.get(&joint.name).ok_or_else(|| {
                    Error::InvalidArgument(format!("Missing position command for {}.", joint.name))
                })?;
                Ok((joint.servo_id, self.radians_to_raw(joint, *radians)?))
            })
            .collect()
    }

    fn convert_states(
        &self,
        raw_states: &BTreeMap<u8, Sts3215RawState>,
    ) -> Result<Vec<JointState>> {
        self.calibrations
            .iter()
            .map(|joint| {
                let raw = raw_states.get(&joint.servo_id).ok_or_else(|| {
                    Error::Runtime(format!("Missing STS3215 state for {}.", joint.name))
                })?;
                Ok(JointState {
                    name: joint.name.clone(),
                    position: self.raw_to_radians(joint, raw.position)?,
                    velocity: raw.velocity as f64
                        * VELOCITY_RAW_TO_RADIANS_PER_SECOND
                        * joint.encoder_direction as f64,
                    current_ma: raw.current as f64 * 6.5,
                    voltage_v: raw.voltage as f64 / 10.0,
                    temperature_c: raw.temperature as f64,
                    status: raw.status,
                })
            })
            .collect()
    }

    fn radians_to_raw(&self, joint: &JointCalibration, radians: f64) -> Result<i32> {
        if !radians.is_finite() {
            return Err(Error::InvalidArgument(format!(
                "{} position command must be finite.",
                joint.name
            )));
        }
        let steps_per_radian = ENCODER_RESOLUTION as f64 / (2.0 * PI);
        let delta = (radians * steps_per_radian).round() as i32 * joint.encoder_direction;
        if delta < joint.safe_min_delta_raw || delta > joint.safe_max_delta_raw {
            return Err(Error::OutOfRange(format!(
                "{} command is outside its calibrated safe range.",
                joint.name
            )));
        }
        Ok((joint.neutral_raw + delta).rem_euclid(ENCODER_RESOLUTION))
    }

    fn raw_to_radians(&self, joint: &JointCalibration, raw_position: i32) -> Result<f64> {
        if !(0..ENCODER_RESOLUTION).contains(&raw_position) {
            return Err(Error::Runtime(format!(
                "{} returned an invalid raw encoder position.",
                joint.name
            )));
        }
        let delta = (raw_position - joint.neutral_raw + ENCODER_RESOLUTION / 2)
            .rem_euclid(ENCODER_RESOLUTION)
            - ENCODER_RESOLUTION / 2;
        let steps_per_radian = ENCODER_RESOLUTION as f64 / (2.0 * PI);
        Ok(delta as f64 / (steps_per_radian * joint.encoder_direction as f64))
    }
}

impl<T: Sts3215Transport> Drop for Sts3215Driver<T> {
    fn drop(&mut self) {
        self.close();
    }
}

impl<T: Sts3215Transport> RuntimeDriver for Sts3215Driver<T> {
    fn apply_servo_profile(&mut self) -> Result<()> {
        Sts3215Driver::apply_servo_profile(self)
    }

    fn activate(&mut self) -> Result<Vec<JointState>> {
        Sts3215Driver::activate(self)
    }

    fn deactivate(&mut self) -> Result<()> {
        Sts3215Driver::deactivate(self)
    }

    fn read(&mut self) -> Result<Vec<JointState>> {
        Sts3215Driver::read(self)
    }

    fn write(&mut self, positions_radians: &JointPositions) -> Result<()> {
        Sts3215Driver::write(self, positions_radians)
    }

    fn joint_limits(&self) -> Result<Vec<JointLimit>> {
        Sts3215Driver::joint_limits(self)
    }

    fn validate_positions(&self, positions_radians: &JointPositions) -> Result<()> {
        Sts3215Driver::validate_positions(self, positions_radians)
    }

    fn clamp_positions_to_safe_range(
        &self,
        positions_radians: &JointPositions,
    ) -> Result<JointPositions> {
        Sts3215Driver::clamp_positions_to_safe_range(self, positions_radians)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeTransport {
        open: bool,
        calls: Vec<String>,
        registers: BTreeMap<(u8, Register), i32>,
        states: BTreeMap<u8, Sts3215RawState>,
        register_writes: Vec<(u8, Register, i32)>,
        position_writes: Vec<BTreeMap<u8, i32>>,
    }

    impl FakeTransport {
        fn add_servo(&mut self, id: u8, position: i32) {
            for (register, value) in [
                (Register::ModelNumber, 777),
                (Register::FirmwareMajorVersion, 3),
                (Register::FirmwareMinorVersion, 10),
                (Register::ReturnDelayTime, 0),
                (Register::Phase, 12),
                (Register::PCoefficient, 32),
                (Register::ICoefficient, 0),
                (Register::DCoefficient, 32),
                (Register::OperatingMode, 0),
                (Register::TorqueEnable, 0),
                (Register::Acceleration, 0),
                (Register::GoalPosition, position),
                (Register::GoalVelocity, 0),
                (Register::TorqueLimit, 1000),
                (Register::Status, 0),
                (Register::MaximumAcceleration, 50),
            ] {
                self.registers.insert((id, register), value);
            }
            self.states.insert(
                id,
                Sts3215RawState {
                    position,
                    voltage: 62,
                    temperature: 28,
                    ..Default::default()
                },
            );
        }
    }

    impl Sts3215Transport for FakeTransport {
        fn open(&mut self, port: &str, baud_rate: i32) -> Result<()> {
            self.calls.push(format!("open:{port}:{baud_rate}"));
            self.open = true;
            Ok(())
        }
        fn close(&mut self) {
            if self.open {
                self.calls.push("close".into());
            }
            self.open = false;
        }
        fn is_open(&self) -> bool {
            self.open
        }
        fn read_register(&mut self, id: u8, register: Register) -> Result<i32> {
            self.calls.push("read_register".into());
            self.registers
                .get(&(id, register))
                .copied()
                .ok_or_else(|| Error::Runtime(format!("missing fake register {id}:{register:?}")))
        }
        fn write_register(&mut self, id: u8, register: Register, value: i32) -> Result<()> {
            self.calls.push("write_register".into());
            self.register_writes.push((id, register, value));
            self.registers.insert((id, register), value);
            Ok(())
        }
        fn set_eeprom_lock(&mut self, _ids: &[u8], locked: bool) -> Result<()> {
            self.calls.push(
                if locked {
                    "eeprom_lock"
                } else {
                    "eeprom_unlock"
                }
                .into(),
            );
            Ok(())
        }
        fn read_states(&mut self, _ids: &[u8]) -> Result<BTreeMap<u8, Sts3215RawState>> {
            self.calls.push("read_states".into());
            Ok(self.states.clone())
        }
        fn write_positions(&mut self, values: &BTreeMap<u8, i32>) -> Result<()> {
            self.calls.push("write_positions".into());
            self.position_writes.push(values.clone());
            for (&id, &position) in values {
                self.registers
                    .insert((id, Register::GoalPosition), position);
            }
            Ok(())
        }
        fn set_torque(&mut self, _ids: &[u8], enabled: bool) -> Result<()> {
            self.calls
                .push(if enabled { "torque_on" } else { "torque_off" }.into());
            Ok(())
        }
    }

    fn calibration(name: &str, id: u8, neutral: i32, min: i32, max: i32) -> JointCalibration {
        JointCalibration {
            name: name.into(),
            servo_id: id,
            neutral_raw: neutral,
            encoder_direction: 1,
            safe_min_delta_raw: min,
            safe_max_delta_raw: max,
        }
    }

    fn calibrations() -> Vec<JointCalibration> {
        vec![
            calibration("base_yaw_joint", 1, 942, -1004, 1004),
            calibration("head_pitch_joint", 5, 3476, -385, 1145),
        ]
    }

    fn positions(values: &[(&str, f64)]) -> JointPositions {
        values
            .iter()
            .map(|(name, value)| ((*name).into(), *value))
            .collect()
    }

    #[test]
    fn applies_orion_profile_with_elbow_override() {
        let mut transport = FakeTransport::default();
        transport.add_servo(1, 904);
        transport.add_servo(3, 1259);
        transport.add_servo(5, 3547);
        let mut driver = Sts3215Driver::new(transport);
        driver
            .configure(
                "/dev/fake",
                1_000_000,
                vec![
                    calibration("base_yaw_joint", 1, 942, -1004, 1004),
                    calibration("elbow_pitch_joint", 3, 789, -739, 480),
                    calibration("head_pitch_joint", 5, 3476, -385, 1145),
                ],
            )
            .unwrap();

        assert_eq!(
            driver.transport().registers[&(1, Register::PCoefficient)],
            16
        );
        assert_eq!(
            driver.transport().registers[&(3, Register::PCoefficient)],
            32
        );
        assert_eq!(
            driver.transport().registers[&(5, Register::PCoefficient)],
            16
        );
        assert!(
            driver
                .transport()
                .register_writes
                .iter()
                .all(|(_, register, _)| !matches!(
                    register,
                    Register::GoalVelocity | Register::TorqueLimit
                ))
        );
        let unlock = driver
            .transport()
            .calls
            .iter()
            .position(|call| call == "eeprom_unlock")
            .unwrap();
        let lock = driver
            .transport()
            .calls
            .iter()
            .position(|call| call == "eeprom_lock")
            .unwrap();
        assert!(unlock < lock);
    }

    #[test]
    fn accepts_an_injected_per_joint_servo_profile() {
        let mut transport = FakeTransport::default();
        transport.add_servo(3, 1259);
        let mut profiles = make_orion_servo_profiles();
        profiles.get_mut("elbow_pitch_joint").unwrap().p_coefficient = 48;
        let mut driver = Sts3215Driver::with_profiles(transport, profiles);
        driver
            .configure(
                "/dev/fake",
                1_000_000,
                vec![calibration("elbow_pitch_joint", 3, 789, -739, 480)],
            )
            .unwrap();
        assert_eq!(
            driver.transport().registers[&(3, Register::PCoefficient)],
            48
        );
    }

    #[test]
    fn connects_and_reads_without_writing() {
        let mut transport = FakeTransport::default();
        transport.add_servo(1, 904);
        transport.add_servo(5, 3547);
        let mut driver = Sts3215Driver::new(transport);
        driver
            .connect("/dev/fake", 1_000_000, calibrations())
            .unwrap();
        assert_eq!(driver.read().unwrap().len(), 2);
        assert!(driver.transport().register_writes.is_empty());
        assert!(driver.transport().position_writes.is_empty());
        assert!(
            !driver
                .transport()
                .calls
                .iter()
                .any(|call| call == "torque_on")
        );

        let limits = driver.joint_limits().unwrap();
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].name, "base_yaw_joint");
        assert!((limits[0].lower_rad + 1.540_116_711).abs() < 1e-6);
        assert!((limits[0].upper_rad - 1.540_116_711).abs() < 1e-6);
    }

    #[test]
    fn refuses_activation_until_servo_profile_is_applied() {
        let mut transport = FakeTransport::default();
        transport.add_servo(1, 904);
        transport.add_servo(5, 3547);
        let mut driver = Sts3215Driver::new(transport);
        driver
            .connect("/dev/fake", 1_000_000, calibrations())
            .unwrap();
        assert!(driver.activate().is_err());
        assert!(driver.transport().position_writes.is_empty());
    }

    #[test]
    fn seeds_present_positions_before_torque_on() {
        let mut transport = FakeTransport::default();
        transport.add_servo(1, 904);
        transport.add_servo(5, 3547);
        let mut driver = Sts3215Driver::new(transport);
        driver
            .configure("/dev/fake", 1_000_000, calibrations())
            .unwrap();
        driver.transport_mut().calls.clear();
        driver.activate().unwrap();

        assert_eq!(driver.transport().position_writes[0][&1], 904);
        let calls = &driver.transport().calls;
        assert!(
            calls.iter().position(|call| call == "read_states").unwrap()
                < calls
                    .iter()
                    .position(|call| call == "write_positions")
                    .unwrap()
        );
        assert!(
            calls
                .iter()
                .position(|call| call == "write_positions")
                .unwrap()
                < calls.iter().position(|call| call == "torque_on").unwrap()
        );
    }

    #[test]
    fn deactivation_turns_torque_off() {
        let mut transport = FakeTransport::default();
        transport.add_servo(1, 904);
        transport.add_servo(5, 3547);
        let mut driver = Sts3215Driver::new(transport);
        driver
            .configure("/dev/fake", 1_000_000, calibrations())
            .unwrap();
        driver.activate().unwrap();
        driver.transport_mut().calls.clear();
        driver.deactivate().unwrap();
        assert_eq!(
            driver
                .transport()
                .calls
                .iter()
                .filter(|call| call.as_str() == "torque_off")
                .count(),
            1
        );
        assert!(!driver.is_active());
    }

    #[test]
    fn converts_commands_across_encoder_wrap_and_rejects_range() {
        let mut transport = FakeTransport::default();
        transport.add_servo(1, 942);
        transport.add_servo(5, 32);
        let mut driver = Sts3215Driver::new(transport);
        driver
            .configure("/dev/fake", 1_000_000, calibrations())
            .unwrap();
        let initial = driver.activate().unwrap();
        assert!((initial[1].position - 1.0).abs() < 0.002);
        driver
            .write(&positions(&[
                ("base_yaw_joint", 0.0),
                ("head_pitch_joint", 1.0),
            ]))
            .unwrap();
        assert_eq!(driver.transport().position_writes.last().unwrap()[&5], 32);
        assert!(
            driver
                .write(&positions(&[
                    ("base_yaw_joint", 0.0),
                    ("head_pitch_joint", 2.0)
                ]))
                .is_err()
        );
    }

    #[test]
    fn clamps_measured_start_inside_command_range() {
        let mut transport = FakeTransport::default();
        transport.add_servo(3, 1262);
        let mut driver = Sts3215Driver::new(transport);
        driver
            .connect(
                "/dev/fake",
                1_000_000,
                vec![calibration("elbow_pitch_joint", 3, 573, -661, 666)],
            )
            .unwrap();
        let clamped = driver
            .clamp_positions_to_safe_range(&positions(&[("elbow_pitch_joint", 1.057)]))
            .unwrap();
        let expected = 666.0 * 2.0 * PI / ENCODER_RESOLUTION as f64;
        assert!((clamped["elbow_pitch_joint"] - expected).abs() < 1e-12);
        driver.validate_positions(&clamped).unwrap();
    }

    #[test]
    fn validates_targets_without_writing() {
        let mut transport = FakeTransport::default();
        transport.add_servo(1, 942);
        transport.add_servo(5, 3476);
        let mut driver = Sts3215Driver::new(transport);
        driver
            .configure("/dev/fake", 1_000_000, calibrations())
            .unwrap();
        driver
            .validate_positions(&positions(&[
                ("base_yaw_joint", 0.5),
                ("head_pitch_joint", 0.5),
            ]))
            .unwrap();
        assert!(driver.transport().position_writes.is_empty());
        assert!(
            driver
                .validate_positions(&positions(&[
                    ("base_yaw_joint", 2.0),
                    ("head_pitch_joint", 0.0),
                ]))
                .is_err()
        );
    }

    #[test]
    fn refuses_torque_on_or_faulted_configuration() {
        for (register, value) in [(Register::TorqueEnable, 1), (Register::Status, 4)] {
            let mut transport = FakeTransport::default();
            transport.add_servo(1, 904);
            transport.add_servo(5, 3547);
            transport.registers.insert(
                (if register == Register::Status { 5 } else { 1 }, register),
                value,
            );
            let mut driver = Sts3215Driver::new(transport);
            assert!(
                driver
                    .configure("/dev/fake", 1_000_000, calibrations())
                    .is_err()
            );
            assert!(!driver.transport().is_open());
            assert!(driver.transport().register_writes.is_empty());
        }
    }
}
