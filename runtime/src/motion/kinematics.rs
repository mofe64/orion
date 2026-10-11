//! URDF-derived camera and light poses; this module does not command motion.
//!
//! Distances are metres and joint angles are radians. The URDF root link is
//! the base frame. Both output frames use ROS optical axes: +X right, +Y down,
//! +Z forward. `rotation` maps local-frame vectors into the base frame.
//! Load and validate the tree once, outside the control tick; FK only does
//! bounded transform arithmetic and joint-map reads, with no I/O or allocation
//! on valid input. URDF limits are not servo calibration and are not applied.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use roxmltree::Node;
use serde::{Deserialize, Serialize};

use super::JointPositions;
use crate::error::{OrionRuntimeError, Result};

/// A local frame's rigid pose in the URDF root (base) frame.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub struct FramePose {
    pub position: [f64; 3],
    /// Row-major rotation; columns are the local X, Y and Z axes in the base.
    pub rotation: [[f64; 3]; 3],
}

impl FramePose {
    const IDENTITY: Self = Self {
        position: [0.0; 3],
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };

    /// Rotate a local direction into the base frame (no translation).
    pub fn rotate_vector(&self, vector: [f64; 3]) -> [f64; 3] {
        self.rotation.map(|row| dot(row, vector))
    }

    /// Transform a local point into the base frame, including translation.
    pub fn transform_point(&self, point: [f64; 3]) -> [f64; 3] {
        let rotated = self.rotate_vector(point);
        std::array::from_fn(|i| self.position[i] + rotated[i])
    }

    /// Express a base-frame vector in this local frame (rotation transpose).
    pub fn inverse_rotate_vector(&self, vector: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| (0..3).map(|j| self.rotation[j][i] * vector[j]).sum())
    }

    pub fn inverse_transform_point(&self, point: [f64; 3]) -> [f64; 3] {
        self.inverse_rotate_vector(std::array::from_fn(|i| point[i] - self.position[i]))
    }

    /// Intersect a forward local ray with a horizontal plane in base metres.
    /// Parallel, upward, and behind-camera intersections have no desk point.
    pub fn table_point(&self, ray: [f64; 3], table_z: f64) -> Result<Option<[f64; 3]>> {
        let norm = ray[0].hypot(ray[1]).hypot(ray[2]);
        if !norm.is_finite() || norm < 1e-12 || !table_z.is_finite() {
            return Err(invalid(
                "Table intersection requires a finite nonzero ray and plane",
            ));
        }
        if ray[2] <= 0.0 {
            return Ok(None);
        }
        let direction = self.rotate_vector(ray.map(|v| v / norm));
        if direction[2] >= -1e-6 {
            return Ok(None);
        }
        let distance = (table_z - self.position[2]) / direction[2];
        if distance <= 0.0 || !distance.is_finite() {
            return Ok(None);
        }
        Ok(Some(std::array::from_fn(|i| {
            self.position[i] + distance * direction[i]
        })))
    }

    fn compose(self, local: Self) -> Self {
        Self {
            position: self.transform_point(local.position),
            rotation: std::array::from_fn(|i| {
                std::array::from_fn(|j| {
                    (0..3)
                        .map(|k| self.rotation[i][k] * local.rotation[k][j])
                        .sum()
                })
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frames {
    pub camera: FramePose,
    pub light: FramePose,
}

#[derive(Clone, Debug)]
struct Joint {
    name: String,
    parent: String,
    origin: FramePose,
    axis: Option<[f64; 3]>,
}

/// Validated root-to-frame chains, derived from joint origins and axes in a URDF.
#[derive(Clone, Debug)]
pub struct Kinematics {
    camera: Vec<Joint>,
    light: Vec<Joint>,
    joint_names: Vec<String>,
}

impl Kinematics {
    /// Load the camera and light chains from a connected fixed/revolute URDF.
    pub fn from_urdf(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let xml = fs::read_to_string(path).map_err(|error| {
            invalid(format!("Could not read URDF '{}': {error}", path.display()))
        })?;
        Self::parse(&xml)
            .map_err(|error| invalid(format!("Invalid URDF '{}': {error}", path.display())))
    }

    /// Compute both poses. Require exactly the URDF's movable joints, all finite.
    ///
    /// This is a pure geometric query, not a safety or reachability check.
    pub fn fk(&self, positions: &JointPositions) -> Result<Frames> {
        if positions.len() != self.joint_names.len()
            || self
                .joint_names
                .iter()
                .any(|name| !positions.contains_key(name))
        {
            return Err(invalid(
                "FK requires exactly the URDF's movable joint names",
            ));
        }
        if positions.values().any(|value| !value.is_finite()) {
            return Err(invalid("FK joint angles must be finite"));
        }
        Ok(Frames {
            camera: evaluate(&self.camera, positions),
            light: evaluate(&self.light, positions),
        })
    }

    /// Pose immediately after a movable joint, with the same calibrated inputs
    /// as FK. Used only by planners, outside RuntimeCore's control tick.
    pub fn joint_frame(&self, name: &str, positions: &JointPositions) -> Result<FramePose> {
        self.fk(positions)?;
        let index = self
            .light
            .iter()
            .position(|joint| joint.name == name)
            .ok_or_else(|| invalid(format!("No light-chain joint {name}")))?;
        Ok(evaluate(&self.light[..=index], positions))
    }

    /// Extract the two-link planar mechanism, including its baked joint-frame
    /// rotations. Reject a different mechanism instead of silently fitting it.
    pub(crate) fn planar_arm(&self) -> Result<([f64; 2], f64)> {
        let names = [
            "base_yaw_joint",
            "shoulder_pitch_joint",
            "elbow_pitch_joint",
            "head_pitch_joint",
            "head_roll_joint",
            "light_joint",
        ];
        if self.light.iter().map(|j| j.name.as_str()).ne(names) {
            return Err(invalid("Lean IK requires the V2 light chain"));
        }
        for (index, joint) in self.light.iter().enumerate().take(5) {
            let expected = if index == 0 || index == 4 {
                [0.0, 0.0, -1.0]
            } else {
                [1.0, 0.0, 0.0]
            };
            if joint.axis != Some(expected) || joint.origin.position[0].abs() > 1e-9 {
                return Err(invalid("Lean IK requires V2 joint axes in one arm plane"));
            }
            if index < 4 && joint.origin.position[1].abs() > 1e-9 {
                return Err(invalid("Lean IK requires aligned pitch joint centres"));
            }
            let r = joint.origin.rotation;
            if (r[0][0] - 1.0).abs() > 1e-9 || r[0][1].abs() > 1e-9 || r[0][2].abs() > 1e-9 {
                return Err(invalid("Lean IK requires pitch-only joint origins"));
            }
        }
        let lengths = [
            self.light[2].origin.position[2],
            self.light[3].origin.position[2],
        ];
        if lengths.iter().any(|v| *v <= 0.0) {
            return Err(invalid("Lean IK requires positive link lengths"));
        }
        let r = self.light[2].origin.rotation;
        Ok((lengths, r[2][1].atan2(r[1][1])))
    }

    fn parse(xml: &str) -> Result<Self> {
        let document = roxmltree::Document::parse(xml)
            .map_err(|error| invalid(format!("Could not parse XML: {error}")))?;
        let robot = document.root_element();
        if !robot.has_tag_name("robot") {
            return Err(invalid("URDF root must be robot"));
        }
        let mut links = BTreeSet::new();
        for link in robot.children().filter(|node| node.has_tag_name("link")) {
            let name = attribute(link, "name")?;
            if !links.insert(name.to_owned()) {
                return Err(invalid(format!("Duplicate link: {name}")));
            }
        }
        let mut joints = Vec::new();
        let mut names = BTreeSet::new();
        let mut parents = BTreeMap::new();
        for node in robot.children().filter(|node| node.has_tag_name("joint")) {
            let name = attribute(node, "name")?.to_owned();
            if !names.insert(name.clone()) {
                return Err(invalid(format!("Duplicate joint: {name}")));
            }
            let parent = attribute(required_child(node, "parent")?, "link")?.to_owned();
            let child = attribute(required_child(node, "child")?, "link")?.to_owned();
            if !links.contains(&parent) || !links.contains(&child) {
                return Err(invalid(format!("Joint {name} references an unknown link")));
            }
            if parents.insert(child.clone(), joints.len()).is_some() {
                return Err(invalid(format!("Link {child} has multiple parent joints")));
            }
            let origin = unique_child(node, "origin")?;
            let xyz = vector(origin.and_then(|n| n.attribute("xyz")), [0.0; 3])?;
            let rpy = vector(origin.and_then(|n| n.attribute("rpy")), [0.0; 3])?;
            let axis = match attribute(node, "type")? {
                "fixed" => None,
                "revolute" => {
                    let axis_node = unique_child(node, "axis")?;
                    let axis = vector(axis_node.and_then(|n| n.attribute("xyz")), [1.0, 0.0, 0.0])?;
                    let norm = axis[0].hypot(axis[1]).hypot(axis[2]);
                    if !norm.is_finite() || norm <= 1e-12 {
                        return Err(invalid(format!(
                            "Joint {name} has an invalid rotation axis"
                        )));
                    }
                    Some(axis.map(|value| value / norm))
                }
                kind => return Err(invalid(format!("Unsupported joint type: {kind}"))),
            };
            joints.push(Joint {
                name,
                parent,
                origin: FramePose {
                    position: xyz,
                    rotation: rpy_rotation(rpy),
                },
                axis,
            });
        }
        let roots: Vec<_> = links
            .iter()
            .filter(|link| !parents.contains_key(*link))
            .collect();
        if roots.len() != 1 {
            return Err(invalid("URDF must have exactly one root link"));
        }
        let root = roots[0];
        // Validate every branch, including branches not used by the output frames.
        for link in &links {
            chain_to(link, root, &joints, &parents)?;
        }
        for frame in ["camera_optical_frame", "light_frame"] {
            if !links.contains(frame) {
                return Err(invalid(format!("URDF is missing {frame}")));
            }
            let joint = parents.get(frame).map(|index| &joints[*index]);
            if joint.is_none_or(|joint| joint.axis.is_some()) {
                return Err(invalid(format!(
                    "Frame {frame} must have a fixed mount joint"
                )));
            }
        }
        Ok(Self {
            camera: chain_to("camera_optical_frame", root, &joints, &parents)?,
            light: chain_to("light_frame", root, &joints, &parents)?,
            joint_names: joints
                .iter()
                .filter(|joint| joint.axis.is_some())
                .map(|joint| joint.name.clone())
                .collect(),
        })
    }
}

fn invalid(message: impl Into<String>) -> OrionRuntimeError {
    OrionRuntimeError::InvalidArgument(message.into())
}

fn attribute<'a>(node: Node<'a, '_>, name: &str) -> Result<&'a str> {
    node.attribute(name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            invalid(format!(
                "{} requires attribute {name}",
                node.tag_name().name()
            ))
        })
}

fn unique_child<'a, 'input>(
    node: Node<'a, 'input>,
    name: &str,
) -> Result<Option<Node<'a, 'input>>> {
    let mut children = node.children().filter(|child| child.has_tag_name(name));
    let first = children.next();
    if children.next().is_some() {
        return Err(invalid(format!(
            "{} has duplicate {name} elements",
            node.tag_name().name()
        )));
    }
    Ok(first)
}

fn required_child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Result<Node<'a, 'input>> {
    unique_child(node, name)?
        .ok_or_else(|| invalid(format!("{} requires {name}", node.tag_name().name())))
}

fn vector(text: Option<&str>, default: [f64; 3]) -> Result<[f64; 3]> {
    let Some(text) = text else {
        return Ok(default);
    };
    let values: Vec<f64> = text
        .split_whitespace()
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| invalid(format!("Invalid URDF vector: {text}")))?;
    let values: [f64; 3] = values
        .try_into()
        .map_err(|_| invalid("URDF vectors require three components"))?;
    if values.iter().any(|value| !value.is_finite()) {
        return Err(invalid("URDF vectors must be finite"));
    }
    Ok(values)
}

fn chain_to(
    target: &str,
    root: &str,
    joints: &[Joint],
    parents: &BTreeMap<String, usize>,
) -> Result<Vec<Joint>> {
    let mut chain = Vec::new();
    let mut seen = BTreeSet::new();
    let mut current = target;
    while current != root {
        if !seen.insert(current) {
            return Err(invalid(format!("Cycle in URDF at {current}")));
        }
        let joint = parents
            .get(current)
            .map(|index| &joints[*index])
            .ok_or_else(|| invalid(format!("Link {current} is not connected to the root")))?;
        chain.push(joint.clone());
        current = &joint.parent;
    }
    chain.reverse();
    Ok(chain)
}

fn evaluate(chain: &[Joint], positions: &JointPositions) -> FramePose {
    let mut pose = FramePose::IDENTITY;
    for joint in chain {
        // URDF convention: translate/rotate by the origin, then rotate around
        // the axis expressed in that joint frame, not in its parent's frame.
        pose = pose.compose(joint.origin);
        if let Some(axis) = joint.axis {
            pose = pose.compose(FramePose {
                rotation: axis_rotation(axis, positions[&joint.name]),
                ..FramePose::IDENTITY
            });
        }
    }
    pose
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

fn axis_rotation(axis: [f64; 3], angle: f64) -> [[f64; 3]; 3] {
    let (s, c) = angle.sin_cos();
    let [x, y, z] = axis;
    let cross = [[0.0, -z, y], [z, 0.0, -x], [-y, x, 0.0]];
    std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            let identity = if i == j { 1.0 } else { 0.0 };
            c * identity + (1.0 - c) * axis[i] * axis[j] + s * cross[i][j]
        })
    })
}

fn rpy_rotation([roll, pitch, yaw]: [f64; 3]) -> [[f64; 3]; 3] {
    // URDF fixed-axis roll/pitch/yaw means Rz(yaw) * Ry(pitch) * Rx(roll).
    let (sr, cr) = roll.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    [
        [cy * cp, cy * sp * sr - sy * cr, cy * sp * cr + sy * sr],
        [sy * cp, sy * sp * sr + cy * cr, sy * sp * cr - cy * sr],
        [-sp, cp * sr, cp * cr],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    const URDF: &str = include_str!("../../../description/urdf/orion-v2.urdf");

    #[derive(Deserialize)]
    struct Fixture {
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        name: String,
        positions: JointPositions,
        camera: FramePose,
        light: FramePose,
    }

    fn fixture() -> Fixture {
        serde_json::from_str(include_str!("../../tests/fixtures/camera_fk_v2.json")).unwrap()
    }

    fn model() -> Kinematics {
        Kinematics::from_urdf(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../description/urdf/orion-v2.urdf"),
        )
        .unwrap()
    }

    fn zero() -> JointPositions {
        crate::ORION_JOINT_NAMES
            .iter()
            .map(|name| ((*name).to_owned(), 0.0))
            .collect()
    }

    fn assert_pose(actual: FramePose, expected: FramePose, label: &str) {
        let distance = actual
            .position
            .iter()
            .zip(expected.position)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt();
        // trace(R_expected^T * R_actual) gives the relative rotation angle.
        let trace: f64 = (0..3)
            .flat_map(|i| (0..3).map(move |j| (i, j)))
            .map(|(i, j)| actual.rotation[i][j] * expected.rotation[i][j])
            .sum();
        let angle = ((trace - 1.0) / 2.0).clamp(-1.0, 1.0).acos();
        assert!(distance <= 0.001, "{label}: position error {distance} m");
        assert!(
            angle <= 0.5_f64.to_radians(),
            "{label}: rotation error {} deg",
            angle.to_degrees()
        );
    }

    #[test]
    fn fk_matches_committed_mujoco_camera_and_light_fixtures() {
        let model = model();
        let fixture = fixture();
        assert_eq!(fixture.cases.len(), 24);
        for case in fixture.cases {
            let frames = model.fk(&case.positions).unwrap();
            assert_pose(frames.camera, case.camera, &format!("{} camera", case.name));
            assert_pose(frames.light, case.light, &format!("{} light", case.name));
        }
    }

    #[test]
    fn home_matches_the_measured_bent_arm_and_downward_view() {
        let home = fixture()
            .cases
            .into_iter()
            .find(|case| case.name == "home")
            .unwrap();
        let model = model();
        let camera = model.fk(&home.positions).unwrap().camera;
        let mut centres = Vec::new();
        for name in [
            "shoulder_pitch_joint",
            "elbow_pitch_joint",
            "head_pitch_joint",
        ] {
            let index = model
                .camera
                .iter()
                .position(|joint| joint.name == name)
                .unwrap();
            centres.push(evaluate(&model.camera[..=index], &home.positions).position);
        }
        for (pair, expected) in centres.windows(2).zip([-46.0, 25.0]) {
            let y = pair[1][1] - pair[0][1];
            let z = pair[1][2] - pair[0][2];
            let lean = (-y).atan2(z).to_degrees();
            assert!((lean - expected).abs() < 1.0, "home link lean {lean}");
        }
        let forward = camera.rotate_vector([0.0, 0.0, 1.0]);
        let elevation = forward[2].atan2(forward[0].hypot(forward[1])).to_degrees();
        assert!((elevation + 13.0).abs() < 1.0, "home elevation {elevation}");
        assert!(forward[1] < -0.9, "home camera must look ahead");
    }

    #[test]
    fn baked_reference_matches_old_fk_of_q_minus_reference_to_one_picometre() {
        let model = model();
        // Before P4 every movable origin had identity rotation. Retain the
        // original translations, axes, meshes and optical mounts; restore only
        // those three origin rotations to reconstruct that CAD model.
        let reference = [
            ("shoulder_pitch_joint", 1.118),
            ("elbow_pitch_joint", -1.500),
            ("head_pitch_joint", 0.070),
        ];
        let mut old = model.clone();
        for chain in [&mut old.camera, &mut old.light] {
            for joint in chain {
                if reference.iter().any(|(name, _)| *name == joint.name) {
                    joint.origin.rotation = FramePose::IDENTITY.rotation;
                }
            }
        }
        let mut cases = fixture().cases;
        let poses: serde_yaml::Value =
            serde_yaml::from_str(include_str!("../../../motion/config/v2/poses.yaml")).unwrap();
        cases.push(Case {
            name: "rest".into(),
            positions: serde_yaml::from_value(poses["poses"]["rest"]["positions"].clone()).unwrap(),
            camera: FramePose::IDENTITY,
            light: FramePose::IDENTITY,
        });
        for case in cases {
            let mut shifted = case.positions.clone();
            for (name, offset) in reference {
                *shifted.get_mut(name).unwrap() -= offset;
            }
            let actual = model.fk(&case.positions).unwrap();
            let expected = old.fk(&shifted).unwrap();
            for (actual, expected) in [
                (actual.camera, expected.camera),
                (actual.light, expected.light),
            ] {
                for (a, e) in actual
                    .position
                    .into_iter()
                    .chain(actual.rotation.into_iter().flatten())
                    .zip(
                        expected
                            .position
                            .into_iter()
                            .chain(expected.rotation.into_iter().flatten()),
                    )
                {
                    assert!((a - e).abs() <= 1e-12, "{}: {a} != {e}", case.name);
                }
            }
        }
    }

    #[test]
    fn positive_yaw_moves_right_and_positive_pitch_tilts_down() {
        let model = model();
        let mut positions = zero();
        // Preserve the original CAD-reference displacement assertions. The
        // calibrated zero is bent and places the camera behind the yaw axis.
        positions.insert("shoulder_pitch_joint".into(), 1.118);
        positions.insert("elbow_pitch_joint".into(), -1.500);
        positions.insert("head_pitch_joint".into(), 0.070);
        let upright = positions.clone();
        let reference = model.fk(&positions).unwrap().camera;
        positions.insert("base_yaw_joint".into(), 0.2);
        let right = model.fk(&positions).unwrap().camera;
        // Facing -Y, the lamp's own right is base -X.
        assert!(right.position[0] < reference.position[0] - 0.01);
        assert!(right.rotate_vector([0.0, 0.0, 1.0])[0] < -0.1);
        positions = upright;
        positions.insert("head_pitch_joint".into(), 0.070 + 0.2);
        assert!(
            model
                .fk(&positions)
                .unwrap()
                .camera
                .rotate_vector([0.0, 0.0, 1.0])[2]
                < -0.1
        );
    }

    #[test]
    fn upright_cad_reference_optical_axes_are_right_down_and_forward() {
        let mut upright = zero();
        upright.insert("shoulder_pitch_joint".into(), 1.118);
        upright.insert("elbow_pitch_joint".into(), -1.500);
        upright.insert("head_pitch_joint".into(), 0.070);
        let frames = model().fk(&upright).unwrap();
        for frame in [frames.camera, frames.light] {
            for (local, expected) in [
                ([1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]),
                ([0.0, 1.0, 0.0], [0.0, 0.0, -1.0]),
                ([0.0, 0.0, 1.0], [0.0, -1.0, 0.0]),
            ] {
                for (actual, expected) in frame.rotate_vector(local).into_iter().zip(expected) {
                    assert!((actual - expected).abs() < 1e-12);
                }
            }
        }
    }

    #[test]
    fn fk_rejects_incomplete_extra_and_nonfinite_joint_inputs() {
        let model = model();
        let mut missing = zero();
        missing.remove("base_yaw_joint");
        assert!(model.fk(&missing).is_err());
        let mut extra = zero();
        extra.insert("unknown".into(), 0.0);
        assert!(model.fk(&extra).is_err());
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut positions = zero();
            positions.insert("head_pitch_joint".into(), value);
            assert!(model.fk(&positions).is_err());
        }
    }

    #[test]
    fn fk_does_not_use_cad_ranges_as_servo_calibration() {
        let mut positions = zero();
        positions.insert("head_roll_joint".into(), 1.7);
        assert!(
            model()
                .fk(&positions)
                .unwrap()
                .camera
                .position
                .iter()
                .all(|v| v.is_finite())
        );
    }

    #[test]
    fn urdf_order_does_not_change_the_loaded_chain() {
        let document = roxmltree::Document::parse(URDF).unwrap();
        let mut xml = String::from("<robot name='reordered'>");
        for node in document
            .root_element()
            .children()
            .filter(|n| n.is_element())
            .rev()
        {
            xml.push_str(&URDF[node.range()]);
        }
        xml.push_str("</robot>");
        let home = fixture()
            .cases
            .into_iter()
            .find(|case| case.name == "home")
            .unwrap();
        assert_eq!(
            Kinematics::parse(&xml)
                .unwrap()
                .fk(&home.positions)
                .unwrap(),
            model().fk(&home.positions).unwrap()
        );
    }

    #[test]
    fn origin_rotation_precedes_the_joint_axis_rotation() {
        let xml = format!(
            r#"<robot name="test">
          <link name="base"/><link name="neck"/>
          <link name="camera_optical_frame"/><link name="light_frame"/>
          <joint name="turn" type="revolute"><parent link="base"/><child link="neck"/>
            <origin xyz="1 2 3" rpy="0 {FRAC_PI_2} 0"/><axis xyz="0 0 2"/></joint>
          <joint name="camera" type="fixed"><parent link="neck"/><child link="camera_optical_frame"/>
            <origin xyz="1 0 0" rpy="0 0 0"/></joint>
          <joint name="light" type="fixed"><parent link="neck"/><child link="light_frame"/></joint>
        </robot>"#
        );
        let positions = [("turn".into(), FRAC_PI_2)].into_iter().collect();
        let frames = Kinematics::parse(&xml).unwrap().fk(&positions).unwrap();
        // Ry(90) * Rz(90) maps local X to +Y, not to -Z or +X.
        assert_pose(
            frames.camera,
            FramePose {
                position: [1.0, 3.0, 3.0],
                rotation: [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            },
            "nonzero joint origin",
        );
        assert_eq!(frames.light.position, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn rpy_uses_fixed_roll_then_pitch_then_yaw() {
        let rotation = rpy_rotation([FRAC_PI_2, FRAC_PI_2, FRAC_PI_2]);
        assert_pose(
            FramePose {
                rotation,
                position: [0.0; 3],
            },
            FramePose {
                rotation: [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]],
                position: [0.0; 3],
            },
            "rpy",
        );
        let frame = FramePose {
            position: [1.0, 2.0, 3.0],
            rotation: axis_rotation([0.0, 0.0, 1.0], PI),
        };
        let point = frame.transform_point([1.0, 0.0, 0.0]);
        assert!((point[0]).abs() < 1e-12);
        assert!((point[1] - 2.0).abs() < 1e-12);
    }

    #[test]
    fn invalid_urdf_geometry_and_trees_are_rejected() {
        let invalid_models = [
            "<robot>",
            "<not_a_robot/>",
            &URDF.replace("name=\"light_frame\"", "name=\"camera_optical_frame\""),
            &URDF.replace("name=\"light_joint\"", "name=\"camera_optical_joint\""),
            &URDF.replace("type=\"revolute\"", "type=\"prismatic\""),
            &URDF.replace("xyz=\"0 0 -1\"", "xyz=\"0 0 0\""),
            &URDF.replace("xyz=\"0 -0.082 0.0157\"", "xyz=\"NaN 0 0\""),
            &URDF.replace("xyz=\"0 -0.082 0.0157\"", "xyz=\"0 0\""),
            &URDF.replace(
                "<parent link=\"v21_base\" />",
                "<parent link=\"missing\" />",
            ),
            &URDF.replace(
                "<parent link=\"v21_base\" />",
                "<parent link=\"v21_neck_swivel_link\" />",
            ),
            &URDF.replace(
                "<child link=\"light_frame\" />",
                "<child link=\"camera_optical_frame\" />",
            ),
            &URDF.replace(
                "<link name=\"light_frame\" />",
                "<link name=\"light_frame\" /><link name=\"disconnected\" />",
            ),
            &URDF.replace(
                "<axis xyz=\"1 0 0\" />",
                "<axis xyz=\"1 0 0\" /><axis xyz=\"1 0 0\" />",
            ),
        ];
        for xml in invalid_models {
            assert!(Kinematics::parse(xml).is_err(), "accepted invalid URDF");
        }
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("missing-kinematics.urdf");
        assert!(Kinematics::from_urdf(missing).is_err());
    }
}
