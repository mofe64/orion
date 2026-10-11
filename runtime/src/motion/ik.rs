//! Pose-only lean planning. Geometry comes from Kinematics; execution still
//! requires MotionSequence, calibrated compilation and normal ownership.
//! Load the envelope and solve outside RuntimeCore's 50 Hz control tick.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::Kinematics;
use crate::{
    Error, JointLimit, JointPositions, MotionSequence, ORION_JOINT_NAMES, PoseLibrary, Result,
};

const SHOULDER: &str = "shoulder_pitch_joint";
const ELBOW: &str = "elbow_pitch_joint";
const WRIST: &str = "head_pitch_joint";
const NECK: &str = "head_roll_joint";
const YAW: &str = "base_yaw_joint";
const EPS: f64 = 1e-9;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComfortBoxConfig {
    /// Names are the maintained source of the joint spans and arm polygon.
    pub poses: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ComfortBox {
    joint_ranges: BTreeMap<String, [f64; 2]>,
    /// Counterclockwise convex hull in (shoulder, elbow) calibrated radians.
    arm_hull: Vec<[f64; 2]>,
}

impl ComfortBox {
    pub fn from_config(
        config: &ComfortBoxConfig,
        poses: &PoseLibrary,
        calibration: &[JointLimit],
    ) -> Result<Self> {
        let limits: BTreeMap<_, _> = calibration
            .iter()
            .map(|limit| (limit.name.as_str(), limit))
            .collect();
        if limits.len() != ORION_JOINT_NAMES.len()
            || limits.len() != calibration.len()
            || ORION_JOINT_NAMES.iter().any(|n| !limits.contains_key(n))
            || limits.values().any(|l| {
                !l.lower_rad.is_finite() || !l.upper_rad.is_finite() || l.lower_rad >= l.upper_rad
            })
        {
            return Err(invalid("Comfort box requires complete finite calibration"));
        }
        if config.poses.len() < 3
            || config.poses.iter().collect::<BTreeSet<_>>().len() != config.poses.len()
        {
            return Err(invalid(
                "Comfort box needs at least three distinct catalogue poses",
            ));
        }
        let mut joint_ranges: BTreeMap<String, [f64; 2]> = ORION_JOINT_NAMES
            .iter()
            .map(|name| ((*name).into(), [f64::INFINITY, f64::NEG_INFINITY]))
            .collect();
        let mut arms = Vec::new();
        for name in &config.poses {
            let pose = poses.pose(name)?;
            for (joint, range) in &mut joint_ranges {
                let value = pose[joint];
                let limit = limits[joint.as_str()];
                if !value.is_finite()
                    || value < limit.lower_rad - EPS
                    || value > limit.upper_rad + EPS
                {
                    return Err(invalid(format!(
                        "Comfort pose {name} exceeds {joint} calibration"
                    )));
                }
                range[0] = range[0].min(value).max(limit.lower_rad);
                range[1] = range[1].max(value).min(limit.upper_rad);
            }
            arms.push([pose[SHOULDER], pose[ELBOW]]);
        }
        let arm_hull = convex_hull(arms);
        if arm_hull.len() < 3 {
            return Err(invalid(
                "Comfort poses do not enclose a shoulder-elbow area",
            ));
        }
        Ok(Self {
            joint_ranges,
            arm_hull,
        })
    }

    pub fn contains(&self, pose: &JointPositions) -> bool {
        pose.len() == self.joint_ranges.len()
            && self.joint_ranges.iter().all(|(name, [lo, hi])| {
                pose.get(name)
                    .is_some_and(|v| v.is_finite() && *v >= lo - EPS && *v <= hi + EPS)
            })
            && self
                .arm_hull
                .iter()
                .zip(self.arm_hull.iter().cycle().skip(1))
                .all(|(a, b)| cross(*a, *b, [pose[SHOULDER], pose[ELBOW]]) >= -EPS)
    }

    /// Uniformly shorten the anchor-to-target joint-space segment. Clipping
    /// against the hull's half planes preserves coupled arm containment.
    pub fn project(
        &self,
        anchor: &JointPositions,
        target: &JointPositions,
    ) -> Result<(JointPositions, f64)> {
        if !self.contains(anchor) {
            return Err(invalid("Lean anchor is outside the comfort box"));
        }
        if target.len() != anchor.len()
            || anchor.keys().any(|n| !target.contains_key(n))
            || target.values().any(|v| !v.is_finite())
        {
            return Err(invalid("Lean target requires five finite joint angles"));
        }
        let mut scale = 1.0_f64;
        for (name, [lo, hi]) in &self.joint_ranges {
            let delta = target[name] - anchor[name];
            if delta > EPS {
                scale = scale.min((hi - anchor[name]) / delta);
            }
            if delta < -EPS {
                scale = scale.min((lo - anchor[name]) / delta);
            }
        }
        let start = [anchor[SHOULDER], anchor[ELBOW]];
        let end = [target[SHOULDER], target[ELBOW]];
        for (a, b) in self
            .arm_hull
            .iter()
            .zip(self.arm_hull.iter().cycle().skip(1))
        {
            let from = cross(*a, *b, start).max(0.0);
            let to = cross(*a, *b, end);
            if to < -EPS {
                scale = scale.min(from / (from - to));
            }
        }
        let scale = scale.clamp(0.0, 1.0);
        Ok((
            anchor
                .iter()
                .map(|(name, value)| (name.clone(), value + scale * (target[name] - value)))
                .collect(),
            scale,
        ))
    }

    /// Per-joint compiler limits alone cannot prove coupled containment.
    /// Call this before installation of any generated lean trajectory.
    pub fn validate_sequence(&self, sequence: &MotionSequence) -> Result<()> {
        for sample in 0..=(sequence.duration_seconds() / 0.02).ceil() as usize {
            if !self.contains(
                &sequence.sample((sample as f64 * 0.02).min(sequence.duration_seconds()))?,
            ) {
                return Err(invalid("Compiled motion leaves the comfort box"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LeanStatus {
    Aimed,
    Clamped,
    Unreachable,
}

#[derive(Clone, Debug, Serialize)]
pub struct LeanSolution {
    pub status: LeanStatus,
    /// Unreachable has no pose, so callers cannot accidentally play a fallback.
    pub positions: Option<JointPositions>,
    pub aim_error_rad: Option<f64>,
}

impl LeanSolution {
    fn unreachable() -> Self {
        Self {
            status: LeanStatus::Unreachable,
            positions: None,
            aim_error_rad: None,
        }
    }
}

/// Analytic two-link inverse kinematics, preserving home's elbow branch.
/// Zero lean preserves the arm position while aiming; one requests the wrist
/// near maximum extension on the shoulder-to-target ray. The comfort hull
/// clips that request toward the anchor, then pitch re-aims from that wrist.
pub fn solve_lean(
    model: &Kinematics,
    comfort: &ComfortBox,
    home: &JointPositions,
    anchor: &JointPositions,
    target: [f64; 3],
    lean: f64,
) -> Result<LeanSolution> {
    if !lean.is_finite() || !(0.0..=1.0).contains(&lean) || target.iter().any(|v| !v.is_finite()) {
        return Err(invalid(
            "Lean requires a finite base-frame point and fraction in [0, 1]",
        ));
    }
    model.fk(home)?;
    model.fk(anchor)?;
    if !comfort.contains(anchor) {
        return Err(invalid("Lean anchor is outside the comfort box"));
    }
    // The table/base occludes below-surface targets, and this planar mechanism
    // reaches ahead of its base, not through or behind the supporting body.
    if target[2] < 0.0 || target[1] >= 0.0 || target[0].hypot(target[1]) < 0.05 {
        return Ok(LeanSolution::unreachable());
    }
    let ([upper, lower], elbow_origin) = model.planar_arm()?;
    let branch = (home[ELBOW] + elbow_origin).sin().signum();
    if branch == 0.0 {
        return Err(invalid("Home must establish a nonsingular elbow branch"));
    }
    let mut posed = anchor.clone();
    posed.insert(YAW.into(), (-target[0]).atan2(-target[1]));
    posed.insert(NECK.into(), 0.0);
    let start_wrist = model.joint_frame(WRIST, &posed)?.position;
    let mut zero_arm = posed.clone();
    zero_arm.insert(SHOULDER.into(), 0.0);
    zero_arm.insert(ELBOW.into(), 0.0);
    let shoulder = model.joint_frame(SHOULDER, &zero_arm)?;
    let toward = sub(target, shoulder.position);
    let distance = norm(toward);
    if distance < EPS {
        return Ok(LeanSolution::unreachable());
    }
    // Retain a small bend at the mathematical end of the arm. No servo or
    // calibration limit is expanded to make a geometric solution fit.
    let extent = (upper + lower) * 0.995;
    let desired: [f64; 3] = std::array::from_fn(|i| {
        start_wrist[i]
            + lean * (shoulder.position[i] + extent * toward[i] / distance - start_wrist[i])
    });
    let local = shoulder.inverse_transform_point(desired);
    let cosine = (local[1] * local[1] + local[2] * local[2] - upper * upper - lower * lower)
        / (2.0 * upper * lower);
    if local[0].abs() > 1e-7 || !(-1.0 - EPS..=1.0 + EPS).contains(&cosine) {
        return Ok(LeanSolution::unreachable());
    }
    let bend = branch * cosine.clamp(-1.0, 1.0).acos();
    posed.insert(
        SHOULDER.into(),
        (-local[1]).atan2(local[2]) - (lower * bend.sin()).atan2(upper + lower * bend.cos()),
    );
    posed.insert(ELBOW.into(), bend - elbow_origin);
    // Clip the arm displacement before aiming from the shortened wrist. The
    // bearing is independent of the arm's extension fraction.
    let yaw = posed[YAW];
    posed.insert(YAW.into(), anchor[YAW]);
    posed.insert(NECK.into(), anchor[NECK]);
    let (mut posed, arm_scale) = comfort.project(anchor, &posed)?;
    let [yaw_min, yaw_max] = comfort.joint_ranges[YAW];
    posed.insert(YAW.into(), yaw.clamp(yaw_min, yaw_max));
    let [neck_min, neck_max] = comfort.joint_ranges[NECK];
    posed.insert(NECK.into(), 0.0_f64.clamp(neck_min, neck_max));
    let Some((posed, pitch_scale)) = aim_within_comfort(model, comfort, anchor, &posed, target)?
    else {
        return Ok(LeanSolution::unreachable());
    };
    let light = model.fk(&posed)?.light;
    let delta = sub(target, light.position);
    let axis = light.rotate_vector([0.0, 0.0, 1.0]);
    let dot: f64 = axis.iter().zip(delta).map(|(a, b)| a * b).sum();
    if dot <= 0.0 || norm(delta) < EPS {
        return Ok(LeanSolution::unreachable());
    }
    let aim_error_rad = (dot / norm(delta)).clamp(-1.0, 1.0).acos();
    let clamped = arm_scale < 1.0 - EPS || pitch_scale < 1.0 - EPS || aim_error_rad > 1e-5;
    Ok(LeanSolution {
        status: if clamped {
            LeanStatus::Clamped
        } else {
            LeanStatus::Aimed
        },
        positions: Some(posed),
        aim_error_rad: Some(aim_error_rad),
    })
}

fn aim_within_comfort(
    model: &Kinematics,
    comfort: &ComfortBox,
    anchor: &JointPositions,
    body: &JointPositions,
    target: [f64; 3],
) -> Result<Option<(JointPositions, f64)>> {
    let candidate = |scale: f64| -> Result<Option<JointPositions>> {
        let mut posed = body.clone();
        for name in [SHOULDER, ELBOW] {
            posed.insert(
                name.into(),
                anchor[name] + scale * (body[name] - anchor[name]),
            );
        }
        let Some(pitch) = aim_pitch(model, &posed, target)? else {
            return Ok(None);
        };
        posed.insert(WRIST.into(), pitch);
        Ok(comfort.contains(&posed).then_some(posed))
    };
    // Pitch depends on wrist position. Search the bounded arm segment for the
    // largest sampled feasible reach, then refine that interval. Each trial
    // uses the analytic aim, and every returned pose is checked against the
    // complete envelope. This never runs in the control tick.
    let mut upper = 1.0;
    for step in 0..=64 {
        let mut lower = 1.0 - step as f64 / 64.0;
        if let Some(mut best) = candidate(lower)? {
            if step == 0 {
                return Ok(Some((best, 1.0)));
            }
            for _ in 0..24 {
                let middle = (lower + upper) / 2.0;
                if let Some(posed) = candidate(middle)? {
                    lower = middle;
                    best = posed;
                } else {
                    upper = middle;
                }
            }
            return Ok(Some((best, lower)));
        }
        upper = lower;
    }
    Ok(None)
}

fn aim_pitch(model: &Kinematics, pose: &JointPositions, target: [f64; 3]) -> Result<Option<f64>> {
    let mut zero_pitch = pose.clone();
    zero_pitch.insert(WRIST.into(), 0.0);
    let wrist = model.joint_frame(WRIST, &zero_pitch)?;
    let light = model.fk(&zero_pitch)?.light;
    let offset = wrist.inverse_transform_point(light.position);
    let direction = wrist.inverse_rotate_vector(light.rotate_vector([0.0, 0.0, 1.0]));
    let local = wrist.inverse_transform_point(target);
    // In the wrist YZ plane, rotating the ray preserves its perpendicular
    // distance from the wrist. Account for the ring mount, not just the wrist
    // centre; otherwise nearby desk targets miss by several degrees.
    let angle_zero = (-direction[2]).atan2(-direction[1]);
    let perpendicular = -offset[1] * angle_zero.sin() + offset[2] * angle_zero.cos();
    let distance = local[1].hypot(local[2]);
    if distance <= perpendicular.abs() + EPS {
        return Ok(None);
    }
    Ok(Some(
        (-local[2]).atan2(-local[1]) + (perpendicular / distance).asin() - angle_zero,
    ))
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn norm(v: [f64; 3]) -> f64 {
    v[0].hypot(v[1]).hypot(v[2])
}
fn cross(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}
fn convex_hull(mut points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    let mut hull = Vec::new();
    for point in &points {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 0.0 {
            hull.pop();
        }
        hull.push(*point);
    }
    let lower_len = hull.len();
    for point in points.iter().rev().skip(1) {
        while hull.len() > lower_len
            && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 0.0
        {
            hull.pop();
        }
        hull.push(*point);
    }
    hull.pop();
    hull
}
fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidArgument(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        HardwareVersion, KeyframeArrival, MotionDefinition, MotionKeyframe, MotionLibrary,
        MotionSpace, MotionStyle,
    };
    use std::path::Path;

    fn setup() -> (Kinematics, PoseLibrary, Vec<JointLimit>, ComfortBox) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let poses = PoseLibrary::load(root.join("motion/config/v2/poses.yaml"), &ORION_JOINT_NAMES)
            .unwrap();
        let limits: Vec<_> = HardwareVersion::V2
            .load_calibration(root.join("simulation/mujoco/config/servo_calibration-v2.json"))
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
        let comfort = ComfortBox::from_config(
            HardwareVersion::V2.profile().comfort_box.as_ref().unwrap(),
            &poses,
            &limits,
        )
        .unwrap();
        (
            Kinematics::from_urdf(root.join("description/urdf/orion-v2.urdf")).unwrap(),
            poses,
            limits,
            comfort,
        )
    }

    fn compiled(
        start: &JointPositions,
        target: &JointPositions,
        limits: &[JointLimit],
    ) -> MotionSequence {
        let definition = MotionDefinition {
            name: "lean-test".into(),
            description: "".into(),
            space: MotionSpace::Absolute,
            style: MotionStyle::named("return_home").unwrap(),
            return_to_anchor: false,
            keyframes: vec![MotionKeyframe {
                pose_name: None,
                target: target.clone(),
                duration_seconds: 4.0,
                arrival: KeyframeArrival::Settle,
                hold_seconds: 0.0,
                marker: None,
            }],
        };
        MotionSequence::compile_scaled_calibrated(
            &definition,
            start.clone(),
            start.keys().map(|n| (n.clone(), 0.0)).collect(),
            None,
            start.clone(),
            1.0,
            limits,
        )
        .unwrap()
    }

    #[test]
    fn desk_grid_aims_within_one_degree_and_compiles_inside_coupled_comfort() {
        let (model, poses, limits, comfort) = setup();
        let home = poses.pose("home").unwrap();
        let mut solutions = 0;
        for x in [-0.20, -0.10, 0.0, 0.10, 0.20] {
            for y in [-0.30, -0.40, -0.60] {
                for lean in [0.7, 0.8, 0.9] {
                    let target = [x, y, 0.0];
                    let result = solve_lean(&model, &comfort, home, home, target, lean).unwrap();
                    let pose = result
                        .positions
                        .as_ref()
                        .expect("desk grid must be reachable");
                    assert!(comfort.contains(pose), "{pose:?}");
                    let light = model.fk(pose).unwrap().light;
                    let hit = light.table_point([0.0, 0.0, 1.0], 0.0).unwrap().unwrap();
                    assert!(
                        norm(sub(hit, target)) < 0.002,
                        "target {target:?} lean {lean} result {result:?} hit {hit:?}"
                    );
                    assert!(result.aim_error_rad.unwrap() < 1_f64.to_radians());
                    comfort
                        .validate_sequence(&compiled(home, pose, &limits))
                        .unwrap();
                    solutions += 1;
                }
            }
        }
        assert_eq!(solutions, 45);
    }

    #[test]
    fn convex_hull_rejects_unsafe_combinations_inside_individual_spans() {
        let (_, poses, limits, comfort) = setup();
        let home = poses.pose("home").unwrap();
        let mut mixed = home.clone();
        mixed.insert(SHOULDER.into(), comfort.joint_ranges[SHOULDER][1]);
        mixed.insert(ELBOW.into(), comfort.joint_ranges[ELBOW][1]);
        assert!(!comfort.contains(&mixed));
        assert!(
            comfort
                .validate_sequence(&compiled(home, &mixed, &limits))
                .is_err()
        );
        let (bounded, scale) = comfort.project(home, &mixed).unwrap();
        assert!((0.0..1.0).contains(&scale));
        assert!(comfort.contains(&bounded));
        for joint in [SHOULDER, ELBOW] {
            assert!(
                (bounded[joint] - home[joint] - scale * (mixed[joint] - home[joint])).abs() < EPS
            );
        }
    }

    #[test]
    fn lean_preserves_home_branch_and_is_continuous_across_the_desk() {
        let (model, poses, _, comfort) = setup();
        let home = poses.pose("home").unwrap();
        let mut previous: Option<JointPositions> = None;
        let (_, elbow_origin) = model.planar_arm().unwrap();
        for step in 0..101 {
            let x = -0.15 + step as f64 * 0.003;
            let pose = solve_lean(&model, &comfort, home, home, [x, -0.35, 0.0], 0.8)
                .unwrap()
                .positions
                .unwrap();
            assert!((pose[ELBOW] + elbow_origin).sin() > 0.0);
            if let Some(last) = previous {
                for name in ORION_JOINT_NAMES {
                    assert!((pose[name] - last[name]).abs() < 0.03, "{name}");
                }
            }
            previous = Some(pose);
        }
    }

    #[test]
    fn zero_lean_keeps_arm_and_strong_lean_has_visible_forward_travel() {
        let (model, poses, _, comfort) = setup();
        let home = poses.pose("home").unwrap();
        let stationary = solve_lean(&model, &comfort, home, home, [0.0, -0.4, 0.0], 0.0).unwrap();
        for name in [SHOULDER, ELBOW] {
            assert!((stationary.positions.as_ref().unwrap()[name] - home[name]).abs() < EPS);
        }
        let reach = solve_lean(&model, &comfort, home, home, [0.0, -0.28, 0.0], 0.8).unwrap();
        let reached = model
            .fk(reach.positions.as_ref().unwrap())
            .unwrap()
            .light
            .position;
        let original = model.fk(home).unwrap().light.position;
        assert!(
            original[1] - reached[1] > 0.12,
            "{original:?} to {reached:?}, {reach:?}"
        );
        assert_eq!(reach.status, LeanStatus::Clamped);
    }

    #[test]
    fn unreachable_and_invalid_targets_never_produce_a_pose() {
        let (model, poses, _, comfort) = setup();
        let home = poses.pose("home").unwrap();
        for target in [[0.0, 0.3, 0.0], [0.0, -0.3, -0.1], [0.0, -0.01, 0.0]] {
            let result = solve_lean(&model, &comfort, home, home, target, 0.8).unwrap();
            assert_eq!(result.status, LeanStatus::Unreachable);
            assert!(result.positions.is_none());
        }
        for lean in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            assert!(solve_lean(&model, &comfort, home, home, [0.0, -0.3, 0.0], lean).is_err());
        }
        assert!(solve_lean(&model, &comfort, home, home, [f64::NAN, -0.3, 0.0], 0.8).is_err());
        let mut outside = home.clone();
        outside.insert(SHOULDER.into(), 2.0);
        assert!(solve_lean(&model, &comfort, home, &outside, [0.0, -0.3, 0.0], 0.8).is_err());
    }

    #[test]
    fn table_rays_include_capture_pose_translation_and_reject_nonforward_hits() {
        let (model, poses, _, _) = setup();
        for name in ["home", "lean_reach_left", "lean_reach_right"] {
            let camera = model.fk(poses.pose(name).unwrap()).unwrap().camera;
            for target in [[0.12, -0.40, 0.0], [-0.10, -0.30, 0.0]] {
                let ray = camera.inverse_rotate_vector(sub(target, camera.position));
                assert!(norm(sub(camera.table_point(ray, 0.0).unwrap().unwrap(), target)) < 0.002);
            }
            for direction in [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0]] {
                let ray = camera.inverse_rotate_vector(direction);
                assert!(camera.table_point(ray, 0.0).unwrap().is_none());
            }
            assert!(camera.table_point([0.0; 3], 0.0).is_err());
            assert!(camera.table_point([f64::NAN, 0.0, 1.0], 0.0).is_err());
        }
    }

    #[test]
    fn reach_catalogue_poses_point_at_the_desk_with_margin_and_no_preview_tag() {
        let (model, poses, limits, comfort) = setup();
        let home = poses.pose("home").unwrap();
        for name in ["lean_reach_low", "lean_reach_left", "lean_reach_right"] {
            let definition = poses.definition(name).unwrap();
            assert!(!definition.tags.iter().any(|s| s == "simulation_preview"));
            assert_eq!(definition.idle_profile.as_deref(), Some("directional"));
            let light = model.fk(&definition.positions).unwrap().light;
            let hit = light.table_point([0.0, 0.0, 1.0], 0.0).unwrap().unwrap();
            assert!((0.25..=0.30).contains(&hit[0].hypot(hit[1])));
            assert!(light.position[0].hypot(light.position[1]) > 0.19);
            assert!(light.position[2] > 0.23);
            for limit in &limits {
                let value = definition.positions[&limit.name];
                assert!(value > limit.lower_rad + 0.05 && value < limit.upper_rad - 0.05);
            }
            comfort
                .validate_sequence(&compiled(home, &definition.positions, &limits))
                .unwrap();
        }
    }

    #[test]
    fn v2_catalogue_and_speech_clips_compile_with_reach_anchors() {
        let (_, poses, limits, _) = setup();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let motions = MotionLibrary::load(root.join("motion/motions/v2"), &poses).unwrap();
        let home = poses.pose("home").unwrap();
        for (_, pose) in poses.iter() {
            compiled(home, pose, &limits);
        }
        for anchor_name in [
            "home",
            "attentive",
            "look_left",
            "look_right",
            "lean_reach_low",
            "lean_reach_left",
            "lean_reach_right",
        ] {
            let anchor = poses.pose(anchor_name).unwrap();
            for (_, definition) in motions.iter() {
                let scale = definition.uniform_amplitude_scale(anchor, &limits).unwrap();
                MotionSequence::compile_scaled_calibrated(
                    definition,
                    anchor.clone(),
                    anchor.keys().map(|n| (n.clone(), 0.0)).collect(),
                    None,
                    anchor.clone(),
                    scale,
                    &limits,
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn comfort_configuration_rejects_missing_duplicate_and_out_of_calibration_poses() {
        let (_, poses, mut limits, _) = setup();
        let mut config = HardwareVersion::V2
            .profile()
            .comfort_box
            .as_ref()
            .unwrap()
            .clone();
        config.poses.push("missing".into());
        assert!(ComfortBox::from_config(&config, &poses, &limits).is_err());
        config.poses.pop();
        config.poses.push("home".into());
        assert!(ComfortBox::from_config(&config, &poses, &limits).is_err());
        config.poses.pop();
        limits
            .iter_mut()
            .find(|l| l.name == SHOULDER)
            .unwrap()
            .upper_rad = 0.5;
        assert!(ComfortBox::from_config(&config, &poses, &limits).is_err());
    }
}
