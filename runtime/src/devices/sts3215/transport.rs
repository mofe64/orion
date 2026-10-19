use std::collections::BTreeMap;
use std::time::Duration;

use rustypot::servo::feetech::sts3215::Sts3215Controller;

use crate::error::{OrionRuntimeError as Error, Result};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Register {
    Id,
    ModelNumber,
    FirmwareMajorVersion,
    FirmwareMinorVersion,
    ReturnDelayTime,
    MaxTorqueLimit,
    Phase,
    PCoefficient,
    DCoefficient,
    ICoefficient,
    ProtectionCurrent,
    OperatingMode,
    TorqueEnable,
    Acceleration,
    GoalPosition,
    GoalVelocity,
    TorqueLimit,
    PresentPosition,
    PresentVelocity,
    PresentCurrent,
    PresentVoltage,
    PresentTemperature,
    Status,
    MaximumVelocityLimit,
    MaximumAcceleration,
    /// Persistent position correction (Ofs). Present and goal positions are
    /// reported relative to it: present = encoder - offset (mod 4096).
    HomingOffset,
}

#[derive(Clone, Copy, Debug)]
struct RegisterInfo {
    address: u8,
    width: u8,
    writable: bool,
}

impl Register {
    fn info(self) -> RegisterInfo {
        match self {
            Self::Id => RegisterInfo {
                address: 5,
                width: 1,
                writable: true,
            },
            Self::FirmwareMajorVersion => RegisterInfo {
                address: 0,
                width: 1,
                writable: false,
            },
            Self::FirmwareMinorVersion => RegisterInfo {
                address: 1,
                width: 1,
                writable: false,
            },
            Self::ModelNumber => RegisterInfo {
                address: 3,
                width: 2,
                writable: false,
            },
            Self::ReturnDelayTime => RegisterInfo {
                address: 7,
                width: 1,
                writable: true,
            },
            Self::MaxTorqueLimit => RegisterInfo {
                address: 16,
                width: 2,
                writable: true,
            },
            Self::Phase => RegisterInfo {
                address: 18,
                width: 1,
                writable: true,
            },
            Self::PCoefficient => RegisterInfo {
                address: 21,
                width: 1,
                writable: true,
            },
            Self::DCoefficient => RegisterInfo {
                address: 22,
                width: 1,
                writable: true,
            },
            Self::ICoefficient => RegisterInfo {
                address: 23,
                width: 1,
                writable: true,
            },
            Self::ProtectionCurrent => RegisterInfo {
                address: 28,
                width: 2,
                writable: true,
            },
            Self::HomingOffset => RegisterInfo {
                address: 31,
                width: 2,
                // Only the explicit torque-free `servo zero` operation writes Ofs.
                writable: true,
            },
            Self::OperatingMode => RegisterInfo {
                address: 33,
                width: 1,
                writable: true,
            },
            Self::TorqueEnable => RegisterInfo {
                address: 40,
                width: 1,
                writable: true,
            },
            Self::Acceleration => RegisterInfo {
                address: 41,
                width: 1,
                writable: true,
            },
            Self::GoalPosition => RegisterInfo {
                address: 42,
                width: 2,
                writable: true,
            },
            Self::GoalVelocity => RegisterInfo {
                address: 46,
                width: 2,
                writable: true,
            },
            Self::TorqueLimit => RegisterInfo {
                address: 48,
                width: 2,
                writable: true,
            },
            Self::PresentPosition => RegisterInfo {
                address: 56,
                width: 2,
                writable: false,
            },
            Self::PresentVelocity => RegisterInfo {
                address: 58,
                width: 2,
                writable: false,
            },
            Self::PresentVoltage => RegisterInfo {
                address: 62,
                width: 1,
                writable: false,
            },
            Self::PresentTemperature => RegisterInfo {
                address: 63,
                width: 1,
                writable: false,
            },
            Self::Status => RegisterInfo {
                address: 65,
                width: 1,
                writable: false,
            },
            Self::PresentCurrent => RegisterInfo {
                address: 69,
                width: 2,
                writable: false,
            },
            Self::MaximumVelocityLimit => RegisterInfo {
                address: 84,
                width: 1,
                writable: false,
            },
            Self::MaximumAcceleration => RegisterInfo {
                address: 85,
                width: 1,
                writable: true,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Sts3215RawState {
    pub position: i32,
    pub velocity: i32,
    pub current: i32,
    pub voltage: i32,
    pub temperature: i32,
    pub status: i32,
}

pub trait Sts3215Transport {
    fn resolve_port(&mut self, requested: &str, _baud_rate: i32, _ids: &[u8]) -> Result<String> {
        Ok(requested.to_owned())
    }
    fn open(&mut self, port: &str, baud_rate: i32) -> Result<()>;
    fn close(&mut self);
    fn is_open(&self) -> bool;
    fn ping(&mut self, servo_id: u8) -> Result<bool> {
        Ok(self.read_register(servo_id, Register::ModelNumber).is_ok())
    }
    fn read_register(&mut self, servo_id: u8, register: Register) -> Result<i32>;
    fn write_register(&mut self, servo_id: u8, register: Register, value: i32) -> Result<()>;
    fn set_eeprom_lock(&mut self, servo_ids: &[u8], locked: bool) -> Result<()>;
    fn read_states(&mut self, servo_ids: &[u8]) -> Result<BTreeMap<u8, Sts3215RawState>>;
    fn write_positions(&mut self, positions: &BTreeMap<u8, i32>) -> Result<()>;
    fn set_torque(&mut self, servo_ids: &[u8], enabled: bool) -> Result<()>;
}

pub struct RustypotTransport {
    controller: Option<Sts3215Controller>,
    timeout: Duration,
}

impl Default for RustypotTransport {
    fn default() -> Self {
        Self {
            controller: None,
            timeout: Duration::from_millis(20),
        }
    }
}

impl RustypotTransport {
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            controller: None,
            timeout,
        }
    }

    fn controller(&mut self) -> Result<&mut Sts3215Controller> {
        self.controller
            .as_mut()
            .ok_or_else(|| Error::InvalidState("STS3215 serial port is not open.".into()))
    }
}

#[derive(Clone, Debug)]
struct PortCandidate {
    path: String,
    known: bool,
}
impl PortCandidate {
    fn new(path: String, info: Option<&serialport::SerialPortInfo>) -> Self {
        // Waveshare Bus Servo Adapter (A) uses the WCH CH343, VID:PID 1a86:55d3.
        // Other WCH converters are not equivalent board identities.
        let usb = info.and_then(|port| match &port.port_type {
            serialport::SerialPortType::UsbPort(usb) => Some(usb),
            _ => None,
        });
        let label = path.to_ascii_lowercase();
        let known = usb.is_some_and(|usb| usb.vid == 0x1a86 && usb.pid == 0x55d3)
            || label.contains("ch343")
            || label.contains("bus_servo_adapter");
        Self { path, known }
    }
}

fn find_adapter<T: Sts3215Transport>(
    transport: &mut T,
    candidates: Vec<PortCandidate>,
    baud: i32,
    ids: &[u8],
) -> Result<String> {
    if ids.is_empty() {
        return Err(Error::InvalidArgument(
            "Servo IDs are required for discovery".into(),
        ));
    }
    let known_present = candidates.iter().any(|port| port.known);
    let mut matches = Vec::new();
    for port in candidates
        .into_iter()
        .filter(|port| !known_present || port.known)
    {
        if transport.open(&port.path, baud).is_ok()
            && ids.iter().all(|id| {
                (0..2)
                    .any(|_| transport.read_register(*id, Register::ModelNumber).ok() == Some(777))
            })
        {
            matches.push(port.path);
        }
        transport.close();
    }
    match matches.as_slice() {
        [port] => Ok(port.clone()),
        _ => Err(Error::Runtime(format!(
            "Expected one matching STS adapter, found {}",
            matches.len()
        ))),
    }
}

impl Sts3215Transport for RustypotTransport {
    fn resolve_port(&mut self, requested: &str, baud_rate: i32, ids: &[u8]) -> Result<String> {
        if !requested.is_empty() && !requested.starts_with("/dev/ttyACM") {
            return Ok(requested.to_owned());
        }
        let ports = serialport::available_ports()
            .map_err(|e| Error::Runtime(format!("Listing serial adapters failed: {e}")))?;
        let mut candidates = Vec::new();
        // Preserve by-id names, but use the enumerated USB metadata to select
        // the shipping CH343 adapter before any legacy fallback probing.
        for entry in std::fs::read_dir("/dev/serial/by-id")
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            let target = std::fs::canonicalize(&path).ok();
            let info = ports.iter().find(|port| {
                target.as_ref().is_some_and(|target| {
                    std::fs::canonicalize(&port.port_name).ok().as_ref() == Some(target)
                })
            });
            candidates.push(PortCandidate::new(
                path.to_string_lossy().into_owned(),
                info,
            ));
        }
        for port in &ports {
            if matches!(port.port_type, serialport::SerialPortType::UsbPort(_))
                || port.port_name.starts_with("/dev/ttyACM")
                || port.port_name.starts_with("/dev/ttyUSB")
            {
                let canonical = std::fs::canonicalize(&port.port_name).ok();
                if !candidates.iter().any(|candidate| {
                    canonical.is_some() && std::fs::canonicalize(&candidate.path).ok() == canonical
                }) {
                    candidates.push(PortCandidate::new(port.port_name.clone(), Some(port)));
                }
            }
        }
        find_adapter(self, candidates, baud_rate, ids)
    }

    fn open(&mut self, port: &str, baud_rate: i32) -> Result<()> {
        if port.is_empty() || baud_rate <= 0 {
            return Err(Error::InvalidArgument(
                "A serial port and positive baud rate are required.".into(),
            ));
        }
        self.close();
        let serial = serialport::new(port, baud_rate as u32)
            .timeout(self.timeout)
            .exclusive(true)
            .open()
            .map_err(|error| {
                Error::Runtime(format!(
                    "Could not open STS3215 serial port {port}: {error}"
                ))
            })?;
        self.controller = Some(
            Sts3215Controller::new()
                .with_protocol_v1()
                .with_serial_port(serial),
        );
        Ok(())
    }

    fn close(&mut self) {
        self.controller = None;
    }

    fn is_open(&self) -> bool {
        self.controller.is_some()
    }

    fn ping(&mut self, servo_id: u8) -> Result<bool> {
        self.controller()?
            .ping(servo_id)
            .map_err(|error| Error::Runtime(format!("Pinging servo {servo_id} failed: {error}")))
    }

    fn read_register(&mut self, servo_id: u8, register: Register) -> Result<i32> {
        let info = register.info();
        let bytes = self
            .controller()?
            .read_raw_data(servo_id, info.address, info.width)
            .map_err(|error| Error::Runtime(format!("Reading servo {servo_id} failed: {error}")))?;
        decode_unsigned(&bytes)
    }

    fn write_register(&mut self, servo_id: u8, register: Register, value: i32) -> Result<()> {
        let info = register.info();
        if !info.writable {
            return Err(Error::InvalidArgument(
                "Attempted to write a read-only STS3215 register.".into(),
            ));
        }
        let bytes = encode_unsigned(value, info.width)?;
        self.controller()?
            .write_raw_data(servo_id, info.address, bytes)
            .map_err(|error| {
                let message = format!(
                    "Writing servo {servo_id} {register:?} (address {}, value {value}) failed: {error}",
                    info.address
                );
                if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                    error.kind() == std::io::ErrorKind::TimedOut
                }) {
                    Error::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, message))
                } else {
                    Error::Runtime(message)
                }
            })
    }

    fn set_eeprom_lock(&mut self, servo_ids: &[u8], locked: bool) -> Result<()> {
        let mut first_error = None;
        for &id in servo_ids {
            let result = self
                .controller()?
                .write_raw_data(id, 55, vec![u8::from(locked)])
                .map_err(|error| {
                    Error::Runtime(format!(
                        "{} servo {id} EEPROM failed: {error}",
                        if locked { "Locking" } else { "Unlocking" }
                    ))
                });
            if let Err(error) = result {
                if !locked {
                    return Err(error);
                }
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn read_states(&mut self, servo_ids: &[u8]) -> Result<BTreeMap<u8, Sts3215RawState>> {
        if servo_ids.is_empty() {
            return Err(Error::InvalidArgument(
                "At least one STS3215 ID is required for state feedback.".into(),
            ));
        }
        let packets = self
            .controller()?
            .sync_read_raw_data(servo_ids, 56, 15)
            .map_err(|error| {
                Error::Runtime(format!(
                    "Reading synchronized STS3215 states failed: {error}"
                ))
            })?;
        if packets.len() != servo_ids.len() {
            return Err(Error::Runtime(
                "Synchronized STS3215 state count does not match requested IDs.".into(),
            ));
        }
        let mut states = BTreeMap::new();
        for (&id, bytes) in servo_ids.iter().zip(packets) {
            if bytes.len() != 15 {
                return Err(Error::Runtime(format!(
                    "Decoding synchronized state for servo {id} returned {} bytes instead of 15.",
                    bytes.len()
                )));
            }
            states.insert(
                id,
                Sts3215RawState {
                    position: decode_word(&bytes[0..2]),
                    velocity: decode_sign_magnitude(decode_word(&bytes[2..4]), 15),
                    voltage: bytes[6] as i32,
                    temperature: bytes[7] as i32,
                    status: bytes[9] as i32,
                    current: decode_sign_magnitude(decode_word(&bytes[13..15]), 15),
                },
            );
        }
        Ok(states)
    }

    fn write_positions(&mut self, positions: &BTreeMap<u8, i32>) -> Result<()> {
        if positions.is_empty() {
            return Err(Error::InvalidArgument(
                "At least one STS3215 position is required.".into(),
            ));
        }
        let mut ids = Vec::with_capacity(positions.len());
        let mut data = Vec::with_capacity(positions.len());
        for (&id, &position) in positions {
            if !(0..4096).contains(&position) {
                return Err(Error::OutOfRange(
                    "STS3215 goal position must be in [0, 4095].".into(),
                ));
            }
            ids.push(id);
            data.push((position as u16).to_le_bytes().to_vec());
        }
        self.controller()?
            .sync_write_raw_data(&ids, 42, &data)
            .map_err(|error| {
                Error::Runtime(format!(
                    "Writing synchronized STS3215 positions failed: {error}"
                ))
            })
    }

    fn set_torque(&mut self, servo_ids: &[u8], enabled: bool) -> Result<()> {
        for &id in servo_ids {
            self.controller()?
                .write_raw_data(id, 40, vec![u8::from(enabled)])
                .map_err(|error| {
                    Error::Runtime(format!(
                        "{} torque on servo {id} failed: {error}",
                        if enabled { "Enabling" } else { "Disabling" }
                    ))
                })?;
        }
        Ok(())
    }
}

fn decode_unsigned(bytes: &[u8]) -> Result<i32> {
    match bytes {
        [value] => Ok(*value as i32),
        [low, high] => Ok(u16::from_le_bytes([*low, *high]) as i32),
        _ => Err(Error::Runtime(format!(
            "STS3215 register returned unsupported width {}.",
            bytes.len()
        ))),
    }
}

fn encode_unsigned(value: i32, width: u8) -> Result<Vec<u8>> {
    let maximum = if width == 1 {
        u8::MAX as i32
    } else {
        u16::MAX as i32
    };
    if value < 0 || value > maximum {
        return Err(Error::OutOfRange(
            "STS3215 register value does not fit its wire width.".into(),
        ));
    }
    Ok(if width == 1 {
        vec![value as u8]
    } else {
        (value as u16).to_le_bytes().to_vec()
    })
}

fn decode_word(bytes: &[u8]) -> i32 {
    u16::from_le_bytes([bytes[0], bytes[1]]) as i32
}

pub(super) fn decode_sign_magnitude(value: i32, sign_bit: u32) -> i32 {
    let sign_mask = 1_i32 << sign_bit;
    if value & sign_mask != 0 {
        -(value & !sign_mask)
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_adapters_exclude_other_serial_devices_even_when_unavailable() {
        use crate::devices::sts3215::driver::tests::FakeTransport;
        for replies in [true, false] {
            let mut transport = FakeTransport::default();
            if replies {
                transport.add_servo(1, 2047);
            }
            let result = find_adapter(
                &mut transport,
                vec![
                    PortCandidate {
                        path: "/zigbee".into(),
                        known: false,
                    },
                    PortCandidate {
                        path: "/servo".into(),
                        known: true,
                    },
                ],
                1_000_000,
                &[1],
            );
            assert_eq!(result.is_ok(), replies);
            assert!(transport.calls.contains(&"open:/servo:1000000".into()));
            assert!(!transport.calls.iter().any(|call| call.contains("zigbee")));
            assert!(!transport.is_open());
        }
    }

    #[test]
    fn discovery_retries_each_model_read_and_supports_legacy_single_ports() {
        use crate::devices::sts3215::driver::tests::FakeTransport;
        let mut transport = FakeTransport::default();
        transport.add_servo(1, 2047);
        transport.add_servo(2, 2047);
        transport.probe_fail_once.extend([1, 2]);
        let found = find_adapter(
            &mut transport,
            vec![PortCandidate {
                path: "/dev/ttyACM1".into(),
                known: false,
            }],
            1_000_000,
            &[1, 2],
        )
        .unwrap();
        assert_eq!(found, "/dev/ttyACM1");
        assert_eq!(
            transport
                .calls
                .iter()
                .filter(|call| call.as_str() == "read_register")
                .count(),
            4
        );
        assert!(!transport.is_open());
    }

    #[test]
    fn shipping_usb_identity_does_not_include_other_wch_or_zigbee_chips() {
        for (pid, known) in [(0x55d3, true), (0x55d4, false), (0x7523, false)] {
            let info = serialport::SerialPortInfo {
                port_name: "/dev/ttyACM0".into(),
                port_type: serialport::SerialPortType::UsbPort(serialport::UsbPortInfo {
                    vid: 0x1a86,
                    pid,
                    serial_number: None,
                    manufacturer: None,
                    product: None,
                }),
            };
            assert_eq!(
                PortCandidate::new(info.port_name.clone(), Some(&info)).known,
                known
            );
        }
    }

    #[test]
    fn serial_open_is_exclusive() {
        use serialport::SerialPort;
        let (_master, slave) = serialport::TTYPort::pair().unwrap();
        let path = slave.name().unwrap();
        let mut transport = RustypotTransport::default();
        assert!(transport.open(&path, 1_000_000).is_err());
        assert!(!transport.is_open());
    }

    #[test]
    fn preserves_orion_register_widths_and_signed_telemetry() {
        assert_eq!(Register::MaximumAcceleration.info().width, 1);
        assert_eq!(Register::FirmwareMajorVersion.info().address, 0);
        assert_eq!(decode_sign_magnitude(0x8005, 15), -5);
        assert_eq!(decode_sign_magnitude(5, 15), 5);
        assert_eq!(encode_unsigned(254, 1).unwrap(), vec![254]);
        assert!(encode_unsigned(256, 1).is_err());
    }
}
