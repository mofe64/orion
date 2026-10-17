use crate::devices::sts3215::{
    setup,
    transport::{RustypotTransport, Sts3215Transport},
};
use crate::{Error, HardwareVersion, Result};

pub(super) fn run(arguments: &[String]) -> Result<i32> {
    let mut args = arguments.iter().map(String::as_str);
    let operation = args.next().unwrap_or("--help");
    if matches!(operation, "--help" | "-h") {
        println!(
            "Usage: oriond servo set-id --joint NAME [--port DEVICE] [--hardware v2]\n       oriond servo zero [--port DEVICE] [--hardware v2]\nConnect one servo for set-id. Fold the assembled lamp into mechanical rest for zero. Torque stays off."
        );
        return Ok(0);
    }
    if !matches!(operation, "set-id" | "zero") {
        return Err(Error::InvalidArgument(
            "servo requires set-id or zero".into(),
        ));
    }
    let mut hardware = HardwareVersion::V2;
    let mut port = None;
    let mut joint = None;
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| Error::InvalidArgument(format!("{flag} requires a value")))?;
        match flag {
            "--hardware" => hardware = value.parse()?,
            "--port" => port = Some(value.to_string()),
            "--joint" => joint = Some(value),
            _ => {
                return Err(Error::InvalidArgument(format!(
                    "Unknown servo option: {flag}"
                )));
            }
        }
    }
    if operation == "zero" && (hardware != HardwareVersion::V2 || joint.is_some()) {
        return Err(Error::InvalidArgument(
            "servo zero requires V2 and zeros all five joints together.".into(),
        ));
    }
    let target = if operation == "set-id" {
        Some(
            hardware
                .profile()
                .joints
                .get(
                    joint.ok_or_else(|| {
                        Error::InvalidArgument("set-id requires --joint NAME".into())
                    })?,
                )
                .ok_or_else(|| {
                    Error::InvalidArgument(
                        "Unknown joint name; use the joint names in the hardware profile.".into(),
                    )
                })?
                .servo_id,
        )
    } else {
        None
    };
    let port = port.map(Ok).unwrap_or_else(setup::detect_port)?;
    let mut transport = RustypotTransport::default();
    transport.open(&port, 1_000_000)?;
    let result = (|| {
        if let Some(target) = target {
            let (old, new) = setup::set_id(&mut transport, target)?;
            println!(
                "{}: servo ID {old} -> {new}; verified, EEPROM locked, torque off",
                joint.unwrap()
            );
        } else {
            for reading in setup::zero(&mut transport, hardware)? {
                println!(
                    "{}: {} -> {} (rest 2047); verified",
                    reading.joint, reading.before, reading.after
                );
            }
            println!("Mechanical rest is zero. EEPROM locked; torque off. Run oriond --check.");
        }
        Ok(0)
    })();
    transport.close();
    result
}
