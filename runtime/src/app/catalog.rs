//! Offline catalog compilation uses the daemon's controller without hardware IO.
use crate::*;
use serde_json::{Value, json};
use std::path::Path;

struct SampleDriver {
    positions: JointPositions,
    limits: Vec<JointLimit>,
    clamps: std::cell::RefCell<Vec<String>>,
}
impl SampleDriver {
    fn states(&self) -> Vec<JointState> {
        ORION_JOINT_NAMES
            .iter()
            .map(|name| JointState {
                name: (*name).into(),
                position: self.positions[*name],
                velocity: 0.0,
                current_ma: 0.0,
                voltage_v: 7.4,
                temperature_c: 25.0,
                status: 0,
            })
            .collect()
    }
}
impl RuntimeDriver for SampleDriver {
    fn apply_servo_profile(&mut self) -> Result<()> {
        Ok(())
    }
    fn activate(&mut self) -> Result<Vec<JointState>> {
        Ok(self.states())
    }
    fn deactivate(&mut self) -> Result<()> {
        Ok(())
    }
    fn read(&mut self) -> Result<Vec<JointState>> {
        Ok(self.states())
    }
    fn write(&mut self, positions: &JointPositions) -> Result<()> {
        self.validate_positions(positions)?;
        self.positions = positions.clone();
        Ok(())
    }
    fn joint_limits(&self) -> Result<Vec<JointLimit>> {
        Ok(self.limits.clone())
    }
    fn validate_positions(&self, positions: &JointPositions) -> Result<()> {
        for limit in &self.limits {
            let value = positions
                .get(&limit.name)
                .ok_or_else(|| Error::InvalidArgument("Missing joint".into()))?;
            if !value.is_finite()
                || *value < limit.lower_rad - 1e-9
                || *value > limit.upper_rad + 1e-9
            {
                return Err(Error::OutOfRange(format!("{}: {value}", limit.name)));
            }
        }
        Ok(())
    }
    fn clamp_positions_to_safe_range(&self, positions: &JointPositions) -> Result<JointPositions> {
        Ok(self
            .limits
            .iter()
            .map(|l| {
                (l.name.clone(), {
                    let value = positions[&l.name];
                    let safe = value.clamp(l.lower_rad, l.upper_rad);
                    if safe != value {
                        self.clamps.borrow_mut().push(l.name.clone());
                    }
                    safe
                })
            })
            .collect())
    }
}

fn checked(response: String) -> Result<Value> {
    let value: Value = serde_json::from_str(&response)?;
    if value["ok"] != true {
        return Err(Error::Runtime(value.to_string()));
    }
    Ok(value)
}
fn command(core: &mut RuntimeCore<SampleDriver>, text: &str, time: f64) -> Result<()> {
    checked(core.handle_command(text, time))?;
    Ok(())
}
fn target(
    motion: &MotionDefinition,
    start: &JointPositions,
    limits: &[JointLimit],
) -> Result<JointPositions> {
    let scale = motion.uniform_amplitude_scale(start, limits)?;
    motion
        .resolved_targets_with_scale(start, scale)?
        .pop()
        .ok_or_else(|| Error::Runtime("Empty motion".into()))
}

fn compile_item(
    kind: &str,
    name: &str,
    anchor: &str,
    poses: &PoseLibrary,
    motions: &MotionLibrary,
    scenes: &SceneLibrary,
    limits: &[JointLimit],
) -> Result<Value> {
    let start = poses
        .pose(if kind == "startup" { "rest" } else { anchor })?
        .clone();
    let driver = SampleDriver {
        positions: start.clone(),
        limits: limits.to_vec(),
        clamps: std::cell::RefCell::new(Vec::new()),
    };
    let mut core = RuntimeCore::new(driver, poses.clone(), motions.clone())?;
    let mut expected = start.clone();
    let mut coordinator = SceneCoordinator::new(scenes.clone(), Rgbw8::OFF);
    let mut lighting = RecordingLightingDevice::new(24)?;
    let mut audio = RecordingAudioDevice::default();
    if kind == "startup" {
        CharacterCoordinator::new(0).start(0.0, &mut core)?;
        expected = poses.pose("home")?.clone();
    } else {
        command(&mut core, "configure", 0.0)?;
        command(&mut core, "enable", 0.0)?;
        match kind {
            "pose" => command(&mut core, &format!("goto {name} 3"), 0.0)?,
            "motion" => {
                expected = target(motions.motion(name)?, &start, limits)?;
                command(&mut core, &format!("play {name}"), 0.0)?;
            }
            "scene" => {
                for event in &scenes.scene(name)?.events {
                    if let SceneAction::Motion(SceneMotion::Play { motion }) = &event.action {
                        expected = target(motions.motion(motion)?, &expected, limits)?;
                    }
                }
                coordinator.start(name, 0.0)?;
            }
            _ => return Err(Error::InvalidArgument("Unknown catalog item".into())),
        }
    }
    let mut samples = Vec::new();
    let mut returning = false;
    // Match the server's order: controller sample first, then scene events.
    for tick in 0..30_000 {
        let now = tick as f64 / OBSERVE_FREQUENCY_HZ;
        core.tick(now)?;
        coordinator.tick(now, &mut core, &mut lighting, &mut audio)?;
        samples.push(json!({"time":now, "positions":core.driver().positions}));
        if let Some(status) = coordinator.last_status() {
            if status.state != ScenePhase::Completed {
                return Err(Error::Runtime(format!("Scene failed: {status:?}")));
            }
        }
        let finished = core.mode() == RuntimeMode::Holding && !coordinator.is_active();
        if finished {
            if let Some(last) = core.snapshot().last_motion.as_ref() {
                if last.state != MovementPhase::Completed {
                    return Err(Error::Runtime(format!("Movement failed: {:?}", last.state)));
                }
            }
            if kind == "pose" && !returning {
                returning = true;
                command(&mut core, "goto home 3", now)?;
            } else {
                return Ok(
                    json!({"kind":kind,"name":name,"anchor":anchor,"samples":samples,
                    "expected_end":expected,"clamped_samples":*core.driver().clamps.borrow()}),
                );
            }
        }
    }
    Err(Error::Runtime("Catalog item exceeds 600 seconds".into()))
}

pub(super) fn run(arguments: &[String]) -> Result<i32> {
    let mut root = None;
    let mut output = None;
    let mut hardware = HardwareVersion::V2;
    let mut args = arguments.iter();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| Error::InvalidArgument(format!("{flag} requires a value")))?;
        match flag.as_str() {
            "--catalog" => root = Some(std::path::PathBuf::from(value)),
            "--hardware" => hardware = value.parse()?,
            "--output" => output = Some(std::path::PathBuf::from(value)),
            _ => {
                return Err(Error::InvalidArgument(format!(
                    "Unknown catalog option: {flag}"
                )));
            }
        }
    }
    let root = root.ok_or_else(|| Error::InvalidArgument("--catalog DIR is required".into()))?;
    let resources = hardware::resource_root()?;
    let profile = hardware.profile();
    let calibration = if hardware == HardwareVersion::V2 {
        &profile.joint_profile
    } else {
        "simulation/mujoco/config/servo_calibration.json"
    };
    let calibration = hardware.load_calibration(resources.join(calibration))?;
    let limits: Vec<_> = calibration
        .iter()
        .map(|c| {
            let (lower_rad, upper_rad) = c.safe_range_radians();
            JointLimit {
                name: c.name.clone(),
                lower_rad,
                upper_rad,
            }
        })
        .collect();
    let poses = PoseLibrary::load(root.join(&profile.poses), &ORION_JOINT_NAMES)?;
    let motions = MotionLibrary::load(root.join(&profile.motions), &poses)?;
    let scenes = SceneLibrary::load(root.join(&profile.scenes), &poses, &motions)?;
    scenes.validate_audio_cues(&CueLibrary::load(root.join("audio/cues"))?)?;
    let mut items = Vec::new();
    for (kind, names) in [
        ("startup", vec!["rest_to_home".into()]),
        ("pose", poses.names()),
        ("motion", motions.names()),
        ("scene", scenes.names()),
    ] {
        for name in names {
            let anchors = if kind == "motion" {
                authored_anchors(&root.join(&profile.motions), &name)?
            } else {
                vec![if kind == "startup" {
                    "rest".into()
                } else {
                    "home".into()
                }]
            };
            for anchor in anchors {
                match compile_item(kind, &name, &anchor, &poses, &motions, &scenes, &limits) {
                    Ok(item) => items.push(item),
                    Err(error) => items.push(
                        json!({"kind":kind,"name":name,"anchor":anchor,"error":error.to_string()}),
                    ),
                }
            }
        }
    }
    let failed = items.iter().any(|item| item.get("error").is_some());
    let document = json!({"format_version":1,"hardware":profile.hardware,"control_rate_hz":OBSERVE_FREQUENCY_HZ,
        "maximum_velocity_rad_s":STS3215_MAX_SPEED_RAD_S,"joint_limits":limits.iter().map(|l|
            json!({"name":l.name,"lower":l.lower_rad,"upper":l.upper_rad})).collect::<Vec<_>>(),"items":items});
    if let Some(path) = output {
        std::fs::write(path, serde_json::to_vec(&document)?)?;
    }
    for item in document["items"].as_array().unwrap() {
        if let Some(error) = item.get("error") {
            eprintln!("{} {}: {error}", item["kind"], item["name"]);
        }
    }
    println!(
        "Compiled {} catalog items",
        document["items"].as_array().unwrap().len()
    );
    Ok(if failed { 1 } else { 0 })
}

// Anchor lists are optional authoring metadata. The runtime otherwise starts at home.
fn authored_anchors(directory: &Path, name: &str) -> Result<Vec<String>> {
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.file_name().is_some_and(|n| n == "user") {
            continue;
        }
        if path.is_dir() {
            let found = authored_anchors(&path, name)?;
            if !found.is_empty() {
                return Ok(found);
            }
        } else if matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("yaml" | "yml")
        ) {
            let value: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(path)?)
                .map_err(|e| Error::Runtime(e.to_string()))?;
            if value["motion"]["name"].as_str() == Some(name) {
                let list = &value["motion"]["anchor_poses"];
                return if list.is_null() {
                    Ok(vec!["home".into()])
                } else {
                    serde_yaml::from_value(list.clone()).map_err(|e| Error::Runtime(e.to_string()))
                };
            }
        }
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_anchor_metadata_is_validated_and_each_anchor_compiles() {
        let root = hardware::resource_root().unwrap();
        let poses = PoseLibrary::load(root.join("motion/config/v2/poses.yaml"), &ORION_JOINT_NAMES)
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let source =
            std::fs::read_to_string(root.join("motion/motions/v2/idle_breathe.yaml")).unwrap();
        let path = directory.path().join("motion.yaml");
        std::fs::write(
            &path,
            source.replace(
                "  name: idle_breathe",
                "  name: idle_breathe\n  anchor_poses: [home, attentive]",
            ),
        )
        .unwrap();
        let motions = MotionLibrary::load(directory.path(), &poses).unwrap();
        assert_eq!(
            authored_anchors(directory.path(), "idle_breathe").unwrap(),
            vec!["home", "attentive"]
        );
        let motion = motions.motion("idle_breathe").unwrap();
        for name in ["home", "attentive"] {
            let anchor = poses.pose(name).unwrap().clone();
            let sequence = MotionSequence::new(motion, anchor.clone()).unwrap();
            assert_eq!(
                sequence.sample(sequence.duration_seconds()).unwrap(),
                anchor
            );
        }
        std::fs::write(
            path,
            source.replace(
                "  name: idle_breathe",
                "  name: idle_breathe\n  anchor_poses: [unknown_pose]",
            ),
        )
        .unwrap();
        assert!(MotionLibrary::load(directory.path(), &poses).is_err());
    }

    #[test]
    fn offline_startup_and_scene_use_runtime_targets_and_finish_at_their_anchors() {
        let root = hardware::resource_root().unwrap();
        let poses = PoseLibrary::load(root.join("motion/config/v2/poses.yaml"), &ORION_JOINT_NAMES)
            .unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions/v2"), &poses).unwrap();
        let scenes = SceneLibrary::load(root.join("scenes/v2"), &poses, &motions).unwrap();
        let limits: Vec<_> = HardwareVersion::V2
            .load_calibration(root.join("hardware/profiles/v2-joints.json"))
            .unwrap()
            .iter()
            .map(|c| {
                let (lower_rad, upper_rad) = c.safe_range_radians();
                JointLimit {
                    name: c.name.clone(),
                    lower_rad,
                    upper_rad,
                }
            })
            .collect();
        for (kind, name, final_pose) in [
            ("startup", "rest_to_home", "home"),
            ("pose", "look_left", "home"),
            ("scene", "acknowledge_left", "look_left"),
        ] {
            let item =
                compile_item(kind, name, "home", &poses, &motions, &scenes, &limits).unwrap();
            let samples = item["samples"].as_array().unwrap();
            let last: JointPositions =
                serde_json::from_value(samples.last().unwrap()["positions"].clone()).unwrap();
            for joint in ORION_JOINT_NAMES {
                assert!((last[joint] - poses.pose(final_pose).unwrap()[joint]).abs() < 1e-9);
            }
            for pair in samples.windows(2) {
                assert!(
                    (pair[1]["time"].as_f64().unwrap() - pair[0]["time"].as_f64().unwrap() - 0.02)
                        .abs()
                        < 1e-9
                );
            }
            if kind == "startup" {
                assert!(samples.last().unwrap()["time"].as_f64().unwrap() >= 4.0);
            }
        }
    }
}
