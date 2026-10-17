"""Build a disposable adapter around unchanged Rust runtime sources for preview IO."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

# This is compiled outside the repository. It can call the real command-time
# planners without exposing development commands in the released application.
RUST = r'''
mod movement_review {
    use crate::*;
    use serde_json::{Value, json};
    use std::path::Path;
    struct Driver {
        positions: JointPositions,
        limits: Vec<JointLimit>,
    }
    impl Driver {
        fn states(&self) -> Vec<JointState> {
            ORION_JOINT_NAMES
                .iter()
                .map(|n| JointState {
                    name: (*n).into(),
                    position: self.positions[*n],
                    velocity: 0.,
                    current_ma: 0.,
                    voltage_v: 7.4,
                    temperature_c: 25.,
                    status: 0,
                })
                .collect()
        }
    }
    impl RuntimeDriver for Driver {
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
        fn write(&mut self, q: &JointPositions) -> Result<()> {
            self.validate_positions(q)?;
            self.positions = q.clone();
            Ok(())
        }
        fn joint_limits(&self) -> Result<Vec<JointLimit>> {
            Ok(self.limits.clone())
        }
        fn validate_positions(&self, q: &JointPositions) -> Result<()> {
            for l in &self.limits {
                if !q[&l.name].is_finite()
                    || q[&l.name] < l.lower_rad - 1e-9
                    || q[&l.name] > l.upper_rad + 1e-9
                {
                    return Err(Error::OutOfRange(l.name.clone()));
                }
            }
            Ok(())
        }
        fn clamp_positions_to_safe_range(&self, q: &JointPositions) -> Result<JointPositions> {
            self.validate_positions(q)?;
            Ok(q.clone())
        }
    }
    fn item(root: &Path, assets: &Path, request: &Value) -> Result<Value> {
        let poses = PoseLibrary::load(
            assets.join("motion/config/v2/poses.yaml"),
            &ORION_JOINT_NAMES,
        )?;
        let motions = MotionLibrary::load(assets.join("motion/motions/v2"), &poses)?;
        let hardware = HardwareVersion::V2;
        let limits: Vec<_> = hardware
            .load_calibration(root.join("hardware/profiles/v2-joints.json"))?
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
        let name = request["name"].as_str().unwrap();
        let start_name = request["start"].as_str().unwrap();
        let anchor_name = request["anchor"].as_str().unwrap();
        let start = poses.pose(start_name)?.clone();
        let anchor = poses.pose(anchor_name)?.clone();
        let mut core = RuntimeCore::new(
            Driver {
                positions: start.clone(),
                limits: limits.clone(),
            },
            poses.clone(),
            motions.clone(),
        )?;
        core.configure_comfort_box(hardware.profile().comfort_box.clone().unwrap())?;
        core.configure_camera_geometry(
            root.join("description/urdf/orion-v2.urdf"),
            Box::new(crate::control::camera_rays::SystemBoottimeClock::new()?),
        )?;
        let geometry = crate::motion::kinematics::Kinematics::from_urdf(
            root.join("description/urdf/orion-v2.urdf"),
        )?;
        let bearing = |pose: &str| -> Result<(f64, f64)> {
            let axis = geometry
                .fk(poses.pose(pose)?)?
                .light
                .rotate_vector([0., 0., 1.]);
            Ok((
                (-axis[0]).atan2(-axis[1]),
                axis[2].atan2(axis[0].hypot(axis[1])),
            ))
        };
        let mut definition = match request["planner"].as_str().unwrap_or("authored") {
            "arrival" => {
                let (a, e) = bearing("attentive")?;
                core.prepare_arrival_motion(&anchor, a, e)?
            }
            "pet" => {
                let (a, e) = bearing("delight")?;
                core.prepare_pet_motion(&anchor, None, a, e, None, 3.)?.0
            }
            "peek" => {
                let light = geometry.fk(poses.pose("lean_reach_low")?)?.light;
                let point = light
                    .table_point([0., 0., 1.], 0.)?
                    .ok_or_else(|| Error::Runtime("Desk ray misses table".into()))?;
                core.prepare_peek_motion(&anchor, point)?
            }
            _ => motions.motion(name)?.clone(),
        };
        // Synthetic markers expose exact compiler arrival times without changing curves.
        for (i, frame) in definition.keyframes.iter_mut().enumerate() {
            frame.marker = Some(format!("review_{i}"));
        }
        let scale = definition.uniform_amplitude_scale(&anchor, &limits)?;
        let targets = definition.resolved_targets_with_scale(&anchor, scale)?;
        let sequence = MotionSequence::compile_scaled_calibrated(
            &definition,
            start.clone(),
            start.keys().map(|n| (n.clone(), 0.)).collect(),
            None,
            anchor.clone(),
            scale,
            &limits,
        )?;
        let duration = sequence.duration_seconds();
        let ticks = (duration * 50.).ceil() as usize;
        let mut samples = Vec::new();
        for tick in 0..=ticks {
            let time = tick as f64 * 0.02;
            let state = sequence.sample_state(time)?;
            samples.push(
                json!({"time":time,"positions":state.positions,"velocities":state.velocities}),
            );
        }
        let keyframes:Vec<_>=targets.iter().enumerate().map(|(i,q)|json!({"positions":q,"time":sequence.marker_time(&format!("review_{i}")).unwrap()})).collect();
        Ok(
            json!({"kind":"motion","name":name,"start_pose":start_name,"anchor":anchor_name,"planner":request["planner"],"anchor_source":request["source"],"samples":samples,"keyframes":keyframes,"expected_end":targets.last().unwrap(),"clamped_samples":[],"amplitude_scale":scale}),
        )
    }
    fn hands(root: &Path, assets: &Path) -> Result<Value> {
        let poses = PoseLibrary::load(assets.join("motion/config/v2/poses.yaml"), &ORION_JOINT_NAMES)?;
        let motions = MotionLibrary::load(assets.join("motion/motions/v2"), &poses)?;
        let anchor = poses.pose("home")?.clone();
        let limits = HardwareVersion::V2.load_calibration(root.join("hardware/profiles/v2-joints.json"))?
            .iter().map(|c| { let (lower_rad, upper_rad)=c.safe_range_radians(); JointLimit{name:c.name.clone(),lower_rad,upper_rad} }).collect();
        let mut core=RuntimeCore::new(Driver{positions:anchor.clone(),limits},poses,motions)?;
        core.configure_camera_geometry(root.join("description/urdf/orion-v2.urdf"), Box::new(crate::control::camera_rays::SystemBoottimeClock::new()?))?;
        let mut character=crate::expression::character::CharacterCoordinator::new(1);
        character.set_gaze_enabled(true);character.start(0.,&mut core)?;
        // Finish the regular startup before recording the held home pose.
        for tick in 1..=200 {let now=tick as f64*0.02;core.tick(now)?;character.tick(now,&mut core,false,None,false,None,None)?;}
        character.set_curiosity_available(true);
        let mut samples=Vec::new();
        let mut observations=Vec::new();
        for tick in 0..=1000 {
            let time=tick as f64*0.02;let now=4.+time;
            core.tick(now)?;
            if tick%5==0 && time<16. {
                let point = if time<2. || time>=12. {None}
                    else if time<5. {Some([-0.10+0.20*(time-2.)/3.,-0.35,0.])}
                    else if time<9. {Some([0.35,-0.001,0.])}
                    else {Some([0.10-0.20*(time-9.)/3.,-0.35,0.])};
                let value=serde_json::from_value(json!({"capture_boottime_ns":core.camera_boottime_ns().unwrap(),
                    "camera_on":true,"tracking_mode":"hands","settings":{},"objects":[],"faces":[],
                    "looking_away":null,"hand_count":u8::from(point.is_some()),"hand_point":point}))
                    .map_err(|e|Error::Runtime(e.to_string()))?;
                character.observe_surroundings(value,now,&mut core)?;
                observations.push(json!({"time":time,"point":point}));
            }
            if tick==800 {character.leave_hands(now,&mut core)?;}
            character.tick(now,&mut core,false,None,false,None,None)?;
            if let Some(failure)=&character.status().hands_failure {return Err(Error::Runtime(failure.message.clone()));}
            let velocities=core.snapshot().motion.as_ref().and_then(|m|core.character_commanded_state(m.run_id,now))
                .map(|s|s.velocities).unwrap_or_else(||anchor.keys().map(|n|(n.clone(),0.)).collect());
            samples.push(json!({"time":time,"positions":core.driver().positions,"velocities":velocities}));
        }
        Ok(json!({"kind":"tracking","name":"hands_desk","start_pose":"home","anchor":"home",
            "anchor_source":"runtime/src/expression/hands.rs: current pose with head-first tracking",
            "samples":samples,"keyframes":[],"expected_end":anchor,"clamped_samples":[],"amplitude_scale":1.,
            "hand_points":observations,"maximum_arm_degrees":5.}))
    }
    pub fn run() -> Result<()> {
        let args: Vec<_> = std::env::args().collect();
        let root = Path::new(&args[1]);
        let assets = Path::new(&args[2]);
        let requests: Value = serde_json::from_slice(&std::fs::read(&args[3])?)?;
        let mut items:Vec<_>=requests.as_array().unwrap().iter().map(|r|match item(root,assets,r){Ok(v)=>v,Err(e)=>json!({"kind":"motion","name":r["name"],"start_pose":r["start"],"anchor":r["anchor"],"error":e.to_string()})}).collect();
        items.push(match hands(root, assets) {Ok(v)=>v,Err(e)=>json!({"kind":"tracking","name":"hands_desk","start_pose":"home","anchor":"home","error":e.to_string()})});
        println!("{}", json!(items));
        Ok(())
    }
}
pub fn movement_review_main() -> Result<()> {
    movement_review::run()
}
'''


def build_adapter(root):
    manifest = (root / 'runtime/Cargo.toml').read_text()
    # Keep package identity and dependencies locked. Only this disposable crate's
    # entry points differ; the shipped Cargo.toml and source files stay intact.
    manifest = manifest.replace('path = "src/lib.rs"', 'path = "review_lib.rs"')
    begin = manifest.index('[[bin]]')
    end = manifest.index('[dependencies]')
    manifest = manifest[:begin] + '[[bin]]\nname = "movement-review"\npath = "review_main.rs"\n\n' + manifest[end:]
    manifest = manifest.replace('path = "../history"', 'path = ' + json.dumps(str(root / 'history')))
    source = 'include!(' + json.dumps(str(root / 'runtime/src/lib.rs')) + ');\n' + RUST
    digest = hashlib.sha256((source + manifest).encode()).hexdigest()[:16]
    directory = Path(tempfile.gettempdir()) / 'orion-movement-review' / digest / 'runtime'
    directory.mkdir(parents=True, exist_ok=True)
    files = {'Cargo.toml': manifest, 'Cargo.lock': (root / 'runtime/Cargo.lock').read_text(),
             'review_lib.rs': source, 'review_main.rs': 'fn main(){if let Err(e)=orion_runtime::movement_review_main(){eprintln!("{e}");std::process::exit(1);}}'}
    for name, value in files.items():
        path = directory / name
        if not path.exists() or path.read_text() != value: path.write_text(value)
    # Runtime source discovery in a development executable uses its manifest parent.
    for name in ('hardware', 'audio'):
        link = directory.parent / name
        if not link.exists(): link.symlink_to(root / name, target_is_directory=True)
    target = root / 'runtime/target'
    environment = {**os.environ, 'CARGO_TARGET_DIR': str(target), 'ORION_BUILD_REVISION': 'movement-review'}
    subprocess.run(['cargo', 'build', '--locked', '--manifest-path', str(directory / 'Cargo.toml'), '--bin', 'movement-review'], check=True, env=environment, cwd=root)
    return target / 'debug/movement-review'
