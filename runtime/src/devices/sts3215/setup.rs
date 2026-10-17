//! Explicit commissioning operations. Never enable torque or depend on a lamp's offsets.
use super::{
    driver::STS3215_MODEL_NUMBER,
    transport::{Register, Sts3215Transport, decode_sign_magnitude},
};
use crate::{Error, HardwareVersion, Result};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub const REST_RAW: i32 = 2047;

/// Ofs uses sign-magnitude, sign bit 11. A half-turn has two equivalent
/// representations; choose +2047 and accept one count of residual at that tie.
pub fn rest_offset(present: i32, old_offset: i32) -> i32 {
    let encoder = (present + old_offset).rem_euclid(4096);
    let offset = (encoder - REST_RAW + 2048).rem_euclid(4096) - 2048;
    if offset == -2048 { 2047 } else { offset }
}

pub fn encode_offset(offset: i32) -> i32 {
    offset.abs() | if offset < 0 { 1 << 11 } else { 0 }
}

pub fn detect_port() -> Result<String> {
    let ports: Vec<_> = serialport::available_ports()
        .map_err(|error| Error::Runtime(format!("Detecting servo adapters failed: {error}")))?
        .into_iter()
        .filter(|port| matches!(port.port_type, serialport::SerialPortType::UsbPort(_)))
        .map(|port| port.port_name)
        .collect();
    match ports.as_slice() {
        [port] => Ok(port.clone()),
        _ => Err(Error::InvalidArgument(format!(
            "Expected one USB servo adapter, found {} ({}); pass --port DEVICE.",
            ports.len(),
            ports.join(", ")
        ))),
    }
}

pub fn scan_one<T: Sts3215Transport>(transport: &mut T) -> Result<u8> {
    let mut ids = Vec::new();
    for id in 1..=253 {
        if transport.ping(id)? {
            ids.push(id);
        }
    }
    match ids.as_slice() {
        [id] => Ok(*id),
        _ => Err(Error::InvalidState(format!(
            "Expected exactly one servo, found {} ({ids:?}); connect only the servo being assigned.",
            ids.len()
        ))),
    }
}

fn write_verified<T: Sts3215Transport>(
    t: &mut T,
    id: u8,
    register: Register,
    value: i32,
) -> Result<()> {
    // An acknowledgement can be lost after an EEPROM write commits. Read it
    // once to establish the outcome; never retry the write blindly.
    let result = t.write_register(id, register, value);
    let observed = t.read_register(id, register)?;
    if observed != value {
        return Err(Error::Runtime(format!(
            "Servo {id} {register:?}: wrote {value}, read {observed}; acknowledgement: {result:?}"
        )));
    }
    Ok(())
}

pub fn set_id<T: Sts3215Transport>(t: &mut T, target: u8) -> Result<(u8, u8)> {
    let old = scan_one(t)?;
    if t.read_register(old, Register::ModelNumber)? != STS3215_MODEL_NUMBER {
        return Err(Error::InvalidState(
            "The connected servo is not an STS3215.".into(),
        ));
    }
    t.set_torque(&[old], false)?;
    if old == target {
        t.set_eeprom_lock(&[old], true)?;
        return Ok((old, target));
    }
    let operation = (|| {
        t.set_eeprom_lock(&[old], false)?;
        let acknowledgement = t.write_register(old, Register::Id, target as i32);
        if t.read_register(target, Register::Id)? != target as i32 || !t.ping(target)? {
            return Err(Error::Runtime(format!(
                "Servo ID write could not be verified: {acknowledgement:?}"
            )));
        }
        Ok((old, target))
    })();
    // If the ID write outcome is uncertain, lock at both possible addresses.
    let lock_new = t.set_eeprom_lock(&[target], true);
    if operation.is_err() {
        let _ = t.set_eeprom_lock(&[old], true);
    }
    operation.and_then(|value| lock_new.map(|()| value))
}

#[derive(Debug)]
pub struct ZeroReading {
    pub joint: String,
    pub before: i32,
    pub after: i32,
}

pub fn zero<T: Sts3215Transport>(t: &mut T, hardware: HardwareVersion) -> Result<Vec<ZeroReading>> {
    zero_with_wait(
        t,
        hardware,
        || std::thread::sleep(Duration::from_millis(20)),
        Duration::from_secs(2),
    )
}

pub(super) fn zero_with_wait<T: Sts3215Transport>(
    t: &mut T,
    hardware: HardwareVersion,
    mut wait: impl FnMut(),
    duration: Duration,
) -> Result<Vec<ZeroReading>> {
    let joints = &hardware.profile().joints;
    let ids: Vec<_> = joints.values().map(|joint| joint.servo_id).collect();
    // Disable torque before taking any position sample.
    t.set_torque(&ids, false)?;
    let mut old = BTreeMap::new();
    let mut first = BTreeMap::new();
    let mut bounds = BTreeMap::new();
    for &id in &ids {
        if t.read_register(id, Register::ModelNumber)? != STS3215_MODEL_NUMBER {
            return Err(Error::InvalidState(format!(
                "Servo {id} is not an STS3215."
            )));
        }
        old.insert(id, t.read_register(id, Register::HomingOffset)?);
        let position = t.read_register(id, Register::PresentPosition)?;
        if !(0..4096).contains(&position) {
            return Err(Error::Runtime(format!(
                "Servo {id} returned invalid position {position}"
            )));
        }
        first.insert(id, position);
        bounds.insert(id, (0, 0));
    }
    let started = Instant::now();
    loop {
        wait();
        for &id in &ids {
            let position = t.read_register(id, Register::PresentPosition)?;
            if !(0..4096).contains(&position) {
                return Err(Error::Runtime(format!(
                    "Servo {id} returned invalid position {position}"
                )));
            }
            let delta = (position - first[&id] + 2048).rem_euclid(4096) - 2048;
            let (low, high) = bounds.get_mut(&id).unwrap();
            *low = (*low).min(delta);
            *high = (*high).max(delta);
            if *high - *low > 10 {
                return Err(Error::InvalidState("lamp is moving; let it settle".into()));
            }
        }
        if started.elapsed() >= duration {
            break;
        }
    }
    let operation = (|| {
        t.set_eeprom_lock(&ids, false)?;
        for &id in &ids {
            let present = t.read_register(id, Register::PresentPosition)?;
            let offset = rest_offset(present, decode_sign_magnitude(old[&id], 11));
            write_verified(t, id, Register::HomingOffset, encode_offset(offset))?;
        }
        t.set_eeprom_lock(&ids, true)?;
        let mut readings = Vec::new();
        for (name, joint) in joints {
            let id = joint.servo_id;
            let after = t.read_register(id, Register::PresentPosition)?;
            if (after - REST_RAW).abs() > 8 {
                return Err(Error::Runtime(format!(
                    "{name} reads {after}; expected 2047 +/-8"
                )));
            }
            readings.push(ZeroReading {
                joint: name.clone(),
                before: first[&id],
                after,
            });
        }
        Ok(readings)
    })();
    if let Err(error) = operation {
        let mut recovery_errors = Vec::new();
        // Restore every offset, including the write whose acknowledgement failed.
        for &id in &ids {
            if let Err(e) = (|| {
                t.set_eeprom_lock(&[id], false)?;
                write_verified(t, id, Register::HomingOffset, old[&id])
            })() {
                recovery_errors.push(format!("servo {id}: {e}"));
            }
        }
        if let Err(e) = t.set_eeprom_lock(&ids, true) {
            recovery_errors.push(e.to_string());
        }
        return Err(Error::Runtime(format!(
            "Zero failed: {error}. {}",
            if recovery_errors.is_empty() {
                "Old offsets restored; EEPROM locked; torque remains off.".into()
            } else {
                format!(
                    "Offset recovery incomplete: {}; torque remains off. Inspect the bus before retrying.",
                    recovery_errors.join("; ")
                )
            }
        )));
    }
    operation
}

#[cfg(test)]
mod tests {
    use super::super::driver::tests::FakeTransport;
    use super::*;
    #[test]
    fn offset_wrap_sign_and_encoder_minus_offset() {
        for encoder in 0_i32..4096 {
            for old in [-2047, -1220, 0, 1368, 2047] {
                let present = (encoder - old).rem_euclid(4096);
                let offset = rest_offset(present, old);
                assert!((-2047..=2047).contains(&offset));
                assert_eq!(decode_sign_magnitude(encode_offset(offset), 11), offset);
                assert!(((encoder - offset).rem_euclid(4096) - REST_RAW).abs() <= 1);
            }
        }
        assert_eq!(rest_offset(1000, 0), -1047);
        assert_eq!(rest_offset(3000, 0), 953);
    }
    #[test]
    fn refuses_zero_or_multiple_servos_without_writes_and_scans_253() {
        let mut t = FakeTransport::default();
        assert!(set_id(&mut t, 1).is_err());
        t.add_servo(253, 1200);
        assert_eq!(scan_one(&mut t).unwrap(), 253);
        t.add_servo(2, 1300);
        assert!(set_id(&mut t, 1).is_err());
        assert!(t.register_writes.is_empty());
    }
    #[test]
    fn assigns_id_and_relocks_at_new_address() {
        let mut t = FakeTransport::default();
        t.add_servo(253, 1000);
        assert_eq!(set_id(&mut t, 4).unwrap(), (253, 4));
        assert_eq!(t.registers[&(4, Register::Id)], 4);
        assert_eq!(t.calls.last().unwrap(), "eeprom_lock");
    }
    fn five() -> FakeTransport {
        let mut t = FakeTransport::default();
        for id in 1..=5 {
            t.add_servo(id, 500 + i32::from(id) * 500);
        }
        t
    }
    #[test]
    fn zeros_all_five_and_keeps_torque_off() {
        let mut t = five();
        let readings = zero_with_wait(&mut t, HardwareVersion::V2, || {}, Duration::ZERO).unwrap();
        assert_eq!(readings.len(), 5);
        assert!(readings.iter().all(|r| r.after == 2047));
        assert_eq!(t.calls.first().unwrap(), "torque_off");
        assert!(!t.calls.iter().any(|c| c == "torque_on"));
        assert_eq!(t.calls.last().unwrap(), "read_register");
    }
    #[test]
    fn restores_every_old_offset_when_final_position_check_fails() {
        let mut t = five();
        let offsets = [-230, 110, 407, 1440, -19];
        for (index, offset) in offsets.iter().enumerate() {
            t.registers.insert(
                (index as u8 + 1, Register::HomingOffset),
                encode_offset(*offset),
            );
        }
        t.fail_zero_verify = true;
        let error = zero_with_wait(&mut t, HardwareVersion::V2, || {}, Duration::ZERO).unwrap_err();
        assert!(
            error.to_string().contains("Old offsets restored"),
            "{error}"
        );
        for id in 1..=5 {
            assert_eq!(
                t.registers[&(id, Register::HomingOffset)],
                encode_offset(offsets[id as usize - 1])
            );
            assert_eq!(
                t.registers[&(id, Register::PresentPosition)],
                500 + i32::from(id) * 500
            );
        }
        assert_eq!(t.calls.last().unwrap(), "eeprom_lock");
    }
    #[test]
    fn a_partial_eeprom_write_failure_restores_all_five_offsets() {
        let mut t = five();
        t.fail_write_once = Some((3, Register::HomingOffset));
        let error = zero_with_wait(&mut t, HardwareVersion::V2, || {}, Duration::ZERO).unwrap_err();
        assert!(error.to_string().contains("Old offsets restored"));
        for id in 1..=5 {
            assert_eq!(t.registers[&(id, Register::HomingOffset)], 0);
        }
        assert_eq!(t.calls.last().unwrap(), "eeprom_lock");
    }

    #[test]
    fn motion_rejection_precedes_eeprom_writes() {
        let mut t = five();
        t.drift = 11;
        let error = zero_with_wait(&mut t, HardwareVersion::V2, || {}, Duration::ZERO).unwrap_err();
        assert!(error.to_string().contains("lamp is moving; let it settle"));
        assert!(t.register_writes.is_empty());
        assert!(!t.calls.contains(&"eeprom_unlock".into()));
    }
}
