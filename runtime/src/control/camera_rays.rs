//! Capture-time geometry queries. Recording measured poses does no allocation.

use crate::motion::kinematics::Kinematics;
use crate::{Error, JointPositions, JointState, ORION_JOINT_NAMES, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

const CAPACITY: usize = 64;
const HISTORY_NS: u64 = 1_000_000_000;
const FUTURE_TOLERANCE_NS: u64 = 50_000_000;
pub const MAX_CAMERA_RAYS: usize = 16;

/// Nanoseconds since boot, including suspend. Inject this clock in tests;
/// never substitute `Instant`/CLOCK_MONOTONIC for the Linux sensor clock.
pub trait BoottimeClock: Send {
    fn now_ns(&self) -> Option<u64>;
}

pub struct SystemBoottimeClock {
    #[cfg(target_os = "macos")]
    numerator: u32,
    #[cfg(target_os = "macos")]
    denominator: u32,
}

impl SystemBoottimeClock {
    pub fn new() -> Result<Self> {
        #[cfg(target_os = "macos")]
        let clock = {
            #[repr(C)]
            struct TimebaseInfo {
                numerator: u32,
                denominator: u32,
            }
            unsafe extern "C" {
                fn mach_timebase_info(info: *mut TimebaseInfo) -> i32;
            }
            let mut info = TimebaseInfo {
                numerator: 0,
                denominator: 0,
            };
            // SAFETY: writable, correctly aligned Darwin timebase-info struct.
            if unsafe { mach_timebase_info(&mut info) } != 0 || info.denominator == 0 {
                return Err(invalid("Cannot read the continuous boot clock timebase"));
            }
            Self {
                numerator: info.numerator,
                denominator: info.denominator,
            }
        };
        #[cfg(not(target_os = "macos"))]
        let clock = Self {};
        clock
            .now_ns()
            .ok_or_else(|| invalid("Suspend-aware boot clock is unavailable"))?;
        Ok(clock)
    }
}

impl BoottimeClock for SystemBoottimeClock {
    fn now_ns(&self) -> Option<u64> {
        #[cfg(target_os = "linux")]
        {
            #[repr(C)]
            struct Timespec {
                seconds: std::ffi::c_long,
                nanoseconds: std::ffi::c_long,
            }
            unsafe extern "C" {
                fn clock_gettime(clock: i32, time: *mut Timespec) -> i32;
            }
            const CLOCK_BOOTTIME: i32 = 7;
            let mut time = Timespec {
                seconds: 0,
                nanoseconds: 0,
            };
            // SAFETY: Linux clock_gettime fills this valid C timespec pointer.
            if unsafe { clock_gettime(CLOCK_BOOTTIME, &mut time) } != 0 {
                return None;
            }
            let seconds = u64::try_from(time.seconds).ok()?;
            let nanoseconds = u64::try_from(time.nanoseconds).ok()?;
            seconds.checked_mul(1_000_000_000)?.checked_add(nanoseconds)
        }
        #[cfg(target_os = "macos")]
        {
            // Darwin simulation uses the continuous clock, which also advances
            // during sleep. Cache its scale at startup, not in the control tick.
            unsafe extern "C" {
                fn mach_continuous_time() -> u64;
            }
            // SAFETY: this OS clock function takes no pointers or mutable state.
            let ticks = unsafe { mach_continuous_time() };
            u64::try_from(
                u128::from(ticks) * u128::from(self.numerator) / u128::from(self.denominator),
            )
            .ok()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            None
        }
    }
}

#[derive(Clone, Copy, Default)]
struct PoseSample {
    boottime_ns: u64,
    positions: [f64; 5],
}

struct PoseHistory {
    samples: [PoseSample; CAPACITY],
    head: usize,
    len: usize,
}

impl Default for PoseHistory {
    fn default() -> Self {
        Self {
            samples: [PoseSample::default(); CAPACITY],
            head: 0,
            len: 0,
        }
    }
}

impl PoseHistory {
    fn sample(&self, index: usize) -> PoseSample {
        self.samples[(self.head + index) % CAPACITY]
    }

    fn record(&mut self, boottime_ns: u64, states: &[JointState]) {
        if self.len > 0 && boottime_ns <= self.sample(self.len - 1).boottime_ns {
            return;
        }
        let mut positions = [0.0; 5];
        for (index, name) in ORION_JOINT_NAMES.iter().enumerate() {
            let Some(state) = states.iter().find(|state| state.name == *name) else {
                return;
            };
            if !state.position.is_finite() {
                return;
            }
            positions[index] = state.position;
        }
        let cutoff = boottime_ns.saturating_sub(HISTORY_NS);
        while self.len > 0 && (self.len == CAPACITY || self.sample(0).boottime_ns < cutoff) {
            self.head = (self.head + 1) % CAPACITY;
            self.len -= 1;
        }
        self.samples[(self.head + self.len) % CAPACITY] = PoseSample {
            boottime_ns,
            positions,
        };
        self.len += 1;
    }

    fn interpolate(&self, capture: u64, now: u64) -> Result<PoseSample> {
        if self.len == 0 {
            return Err(invalid("No measured camera pose history yet"));
        }
        if capture > now.saturating_add(FUTURE_TOLERANCE_NS) {
            return Err(invalid("Camera timestamp is more than 50 ms in the future"));
        }
        let first = self.sample(0);
        let last = self.sample(self.len - 1);
        if capture < first.boottime_ns || now.saturating_sub(capture) > HISTORY_NS {
            return Err(invalid(
                "Camera timestamp is older than the measured pose buffer",
            ));
        }
        if (capture > last.boottime_ns
            && now.saturating_sub(last.boottime_ns) > FUTURE_TOLERANCE_NS)
            || now < last.boottime_ns
        {
            return Err(invalid(
                "Measured camera pose history is unavailable at this timestamp",
            ));
        }
        if capture == first.boottime_ns {
            return Ok(first);
        }
        if capture >= last.boottime_ns {
            return Ok(last);
        }
        for index in 1..self.len {
            let next = self.sample(index);
            if next.boottime_ns >= capture {
                if capture == next.boottime_ns {
                    return Ok(next);
                }
                let previous = self.sample(index - 1);
                if next.boottime_ns - previous.boottime_ns > FUTURE_TOLERANCE_NS {
                    return Err(invalid("Measured camera feedback gap exceeds 50 ms"));
                }
                let fraction = (capture - previous.boottime_ns) as f64
                    / (next.boottime_ns - previous.boottime_ns) as f64;
                return Ok(PoseSample {
                    boottime_ns: capture,
                    positions: std::array::from_fn(|i| {
                        previous.positions[i]
                            + fraction * (next.positions[i] - previous.positions[i])
                    }),
                });
            }
        }
        unreachable!("capture inside ordered history has neighbouring samples")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraRaysRequest {
    pub capture_boottime_ns: u64,
    pub rays: Vec<[f64; 3]>,
}

#[derive(Debug, Serialize)]
pub struct Bearing {
    pub azimuth: f64,
    pub elevation: f64,
}

impl Bearing {
    pub fn from_direction(direction: [f64; 3]) -> Self {
        Self {
            azimuth: (-direction[0]).atan2(-direction[1]),
            elevation: direction[2].atan2(direction[0].hypot(direction[1])),
        }
    }
}

#[derive(Serialize)]
pub struct CameraRaysResponse {
    pub ok: bool,
    pub bearings: Vec<Bearing>,
    pub pose_age_ms: f64,
    pub camera: crate::motion::kinematics::FramePose,
}

pub struct CameraGeometry {
    pub kinematics: Kinematics,
    history: PoseHistory,
    clock: Box<dyn BoottimeClock>,
}

impl CameraGeometry {
    pub fn now_ns(&self) -> Option<u64> {
        self.clock.now_ns()
    }
    /// Load once at startup. Neither recording nor querying reads the URDF.
    pub fn load(path: impl AsRef<Path>, clock: Box<dyn BoottimeClock>) -> Result<Self> {
        Ok(Self {
            kinematics: Kinematics::from_urdf(path)?,
            history: PoseHistory::default(),
            clock,
        })
    }

    pub fn record_measured(&mut self, states: &[JointState]) {
        if let Some(now) = self.clock.now_ns() {
            self.history.record(now, states);
        }
    }

    pub fn query(&self, request: CameraRaysRequest) -> Result<CameraRaysResponse> {
        if request.rays.is_empty() || request.rays.len() > MAX_CAMERA_RAYS {
            return Err(invalid("camera-rays requires 1 to 16 unit rays"));
        }
        if request.rays.iter().any(|ray| {
            ray.iter().any(|v| !v.is_finite())
                || (ray.iter().map(|v| v * v).sum::<f64>() - 1.0).abs() > 1e-6
        }) {
            return Err(invalid("Camera rays must be finite unit vectors"));
        }
        let now = self
            .clock
            .now_ns()
            .ok_or_else(|| invalid("Camera boot clock read failed"))?;
        let pose = self.history.interpolate(request.capture_boottime_ns, now)?;
        let positions: JointPositions = ORION_JOINT_NAMES
            .iter()
            .zip(pose.positions)
            .map(|(name, angle)| ((*name).to_owned(), angle))
            .collect();
        let camera = self.kinematics.fk(&positions)?.camera;
        Ok(CameraRaysResponse {
            ok: true,
            camera: camera.clone(),
            bearings: request
                .rays
                .into_iter()
                .map(|ray| Bearing::from_direction(camera.rotate_vector(ray)))
                .collect(),
            pose_age_ms: now.saturating_sub(pose.boottime_ns) as f64 / 1e6,
        })
    }
}

fn invalid(message: &str) -> Error {
    Error::InvalidArgument(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    thread_local! {
        static COUNTING: Cell<bool> = const { Cell::new(false) };
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }
    struct CountingAllocator;
    fn count_allocation() {
        let _ = COUNTING.try_with(|enabled| {
            if enabled.get() {
                let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
            }
        });
    }
    // SAFETY: all memory operations delegate unchanged to the system allocator;
    // counters are thread-local so concurrent unit tests cannot affect the count.
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            count_allocation();
            unsafe { System.alloc(layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            count_allocation();
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            count_allocation();
            unsafe { System.realloc(ptr, layout, size) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
    }
    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    struct TestClock(Arc<AtomicU64>);
    impl BoottimeClock for TestClock {
        fn now_ns(&self) -> Option<u64> {
            Some(self.0.load(Ordering::Relaxed))
        }
    }

    fn home() -> JointPositions {
        let poses: serde_yaml::Value =
            serde_yaml::from_str(include_str!("../../../motion/config/v2/poses.yaml")).unwrap();
        serde_yaml::from_value(poses["poses"]["home"]["positions"].clone()).unwrap()
    }

    fn states(positions: &JointPositions) -> Vec<JointState> {
        positions
            .iter()
            .rev()
            .map(|(name, position)| JointState {
                name: name.clone(),
                position: *position,
                velocity: 0.0,
                current_ma: 0.0,
                voltage_v: 12.0,
                temperature_c: 25.0,
                status: 0,
            })
            .collect()
    }

    fn geometry() -> (CameraGeometry, Arc<AtomicU64>) {
        let now = Arc::new(AtomicU64::new(1_000_000_000));
        let camera = CameraGeometry::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../description/urdf/orion-v2.urdf"),
            Box::new(TestClock(now.clone())),
        )
        .unwrap();
        (camera, now)
    }

    fn request(capture: u64, rays: Vec<[f64; 3]>) -> CameraRaysRequest {
        CameraRaysRequest {
            capture_boottime_ns: capture,
            rays,
        }
    }

    #[test]
    fn recording_pose_history_never_allocates_even_when_the_ring_wraps() {
        let (mut camera, clock) = geometry();
        let states = states(&home());
        ALLOCATIONS.with(|count| count.set(0));
        COUNTING.with(|count| count.set(true));
        for index in 0..10_000 {
            clock.store(1_000_000_000 + index * 20_000_000, Ordering::Relaxed);
            camera.record_measured(&states);
        }
        COUNTING.with(|count| count.set(false));
        assert_eq!(ALLOCATIONS.with(Cell::get), 0);
        assert_eq!(camera.history.len, 51);
    }

    #[test]
    fn feedback_gaps_reject_invented_poses_but_keep_exact_samples() {
        let (mut camera, clock) = geometry();
        camera.record_measured(&states(&home()));
        clock.store(1_300_000_000, Ordering::Relaxed);
        camera.record_measured(&states(&home()));
        assert!(
            camera
                .query(request(1_150_000_000, vec![[0., 0., 1.]]))
                .is_err()
        );
        for stamp in [1_000_000_000, 1_300_000_000] {
            assert!(camera.query(request(stamp, vec![[0., 0., 1.]])).is_ok());
        }
        clock.store(1_350_000_000, Ordering::Relaxed);
        camera.record_measured(&states(&home()));
        assert!(
            camera
                .query(request(1_325_000_000, vec![[0., 0., 1.]]))
                .is_ok()
        );
    }

    #[test]
    fn history_interpolates_measured_angles_and_rejects_old_or_future_captures() {
        let (mut camera, clock) = geometry();
        let mut positions = home();
        camera.record_measured(&states(&positions));
        positions.insert(
            "base_yaw_joint".into(),
            (home()["base_yaw_joint"] + 0.5) / 2.0,
        );
        clock.store(1_050_000_000, Ordering::Relaxed);
        camera.record_measured(&states(&positions));
        positions.insert("base_yaw_joint".into(), 0.5);
        clock.store(1_100_000_000, Ordering::Relaxed);
        camera.record_measured(&states(&positions));
        let pose = camera
            .history
            .interpolate(1_050_000_000, 1_100_000_000)
            .unwrap();
        assert!((pose.positions[0] - (home()["base_yaw_joint"] + 0.5) / 2.0).abs() < 1e-12);
        assert!(
            camera
                .query(request(999_999_999, vec![[0.0, 0.0, 1.0]]))
                .is_err()
        );
        assert!(
            camera
                .query(request(1_150_000_001, vec![[0.0, 0.0, 1.0]]))
                .is_err()
        );
        let future = camera
            .query(request(1_150_000_000, vec![[0.0, 0.0, 1.0]]))
            .unwrap();
        assert_eq!(future.pose_age_ms, 0.0);
        clock.store(1_120_000_000, Ordering::Relaxed);
        let future = camera
            .query(request(1_170_000_000, vec![[0.0, 0.0, 1.0]]))
            .unwrap();
        assert_eq!(future.pose_age_ms, 20.0);
        clock.store(1_100_000_000, Ordering::Relaxed);
        let response = camera
            .query(request(1_050_000_000, vec![[0.0, 0.0, 1.0]]))
            .unwrap();
        assert_eq!(response.pose_age_ms, 50.0);
        // A camera or feedback outage must not allow a fresh timestamp to reuse
        // a pose more than 50 ms away, even though the clock itself keeps moving.
        clock.store(2_300_000_000, Ordering::Relaxed);
        assert!(
            camera
                .query(request(2_300_000_000, vec![[0.0, 0.0, 1.0]]))
                .is_err()
        );
        assert!(
            camera
                .query(request(1_100_000_000, vec![[0.0, 0.0, 1.0]]))
                .is_err()
        );
        camera.record_measured(&states(&positions));
        assert_eq!(camera.history.len, 1);
        assert!(
            camera
                .query(request(1_100_000_000, vec![[0.0, 0.0, 1.0]]))
                .is_err()
        );
    }

    #[test]
    fn duplicate_or_invalid_feedback_does_not_pollute_history() {
        let (mut camera, clock) = geometry();
        let mut feedback = states(&home());
        camera.record_measured(&feedback);
        camera.record_measured(&feedback);
        clock.store(999_999_999, Ordering::Relaxed);
        camera.record_measured(&feedback);
        clock.store(1_020_000_000, Ordering::Relaxed);
        feedback[0].position = f64::NAN;
        camera.record_measured(&feedback);
        camera.record_measured(&feedback[1..]);
        assert_eq!(camera.history.len, 1);
    }

    #[test]
    fn ray_count_and_unit_vector_contract_is_strict() {
        let (mut camera, _) = geometry();
        camera.record_measured(&states(&home()));
        assert!(
            camera
                .query(request(1_000_000_000, vec![[0.0, 0.0, 1.0]; 16]))
                .is_ok()
        );
        for rays in [
            vec![],
            vec![[0.0, 0.0, 1.0]; 17],
            vec![[0.0; 3]],
            vec![[0.0, 0.0, 2.0]],
            vec![[f64::NAN, 0.0, 1.0]],
        ] {
            assert!(camera.query(request(1_000_000_000, rays)).is_err());
        }
        assert!(
            serde_json::from_str::<CameraRaysRequest>(
                r#"{"capture_boottime_ns":1,"rays":[[0,0,1]],"joints":{}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn capture_pose_keeps_a_stationary_target_stable_during_yaw_and_swivel_sweeps() {
        let point = [-7.0, -30.0, 3.0];
        let mut first_bearing: Option<Bearing> = None;
        let positions_at = |t: f64| {
            let mut pose = home();
            pose.insert("base_yaw_joint".into(), -0.6 + t);
            pose.insert("head_roll_joint".into(), -0.4 + 0.8 * t);
            pose
        };
        let mut maximum_current_pose_error: f64 = 0.0;
        for index in 0..20 {
            let capture_seconds = 0.51 + f64::from(index) * 0.02;
            let (mut camera, clock) = geometry();
            for tick in 0..=36 + index {
                let seconds = f64::from(tick) * 0.02;
                clock.store(
                    1_000_000_000 + u64::from(tick as u32) * 20_000_000,
                    Ordering::Relaxed,
                );
                camera.record_measured(&states(&positions_at(seconds)));
            }
            let capture = 1_000_000_000 + (capture_seconds * 1e9).round() as u64;
            let true_camera = camera
                .kinematics
                .fk(&positions_at(capture_seconds))
                .unwrap()
                .camera;
            let direction: [f64; 3] = std::array::from_fn(|i| point[i] - true_camera.position[i]);
            let length = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
            let expected = Bearing::from_direction(direction);
            let ray: [f64; 3] = std::array::from_fn(|column| {
                (0..3)
                    .map(|row| true_camera.rotation[row][column] * direction[row] / length)
                    .sum()
            });
            let result = camera.query(request(capture, vec![ray])).unwrap();
            assert!((result.bearings[0].azimuth - expected.azimuth).abs() < 0.5_f64.to_radians());
            assert!(
                (result.bearings[0].elevation - expected.elevation).abs() < 0.5_f64.to_radians()
            );
            // Finite-distance parallax is real: stability compares captures;
            // accuracy uses the direction from each true camera origin.
            let baseline = first_bearing.get_or_insert(expected);
            assert!((result.bearings[0].azimuth - baseline.azimuth).abs() < 0.5_f64.to_radians());
            assert!(
                (result.bearings[0].elevation - baseline.elevation).abs() < 0.5_f64.to_radians()
            );
            let latest = clock.load(Ordering::Relaxed);
            let wrong = camera.query(request(latest, vec![ray])).unwrap();
            maximum_current_pose_error = maximum_current_pose_error
                .max((wrong.bearings[0].azimuth - result.bearings[0].azimuth).abs());
        }
        assert!(
            maximum_current_pose_error > 5.0_f64.to_radians(),
            "current-pose substitution must visibly fail this same sweep"
        );
    }

    #[test]
    fn base_bearing_signs_match_the_lamps_axes() {
        assert_eq!(Bearing::from_direction([0.0, -1.0, 0.0]).azimuth, 0.0);
        assert!(Bearing::from_direction([-1.0, -1.0, 0.0]).azimuth > 0.0);
        assert!(Bearing::from_direction([1.0, -1.0, 0.0]).azimuth < 0.0);
        assert!(Bearing::from_direction([0.0, -1.0, 1.0]).elevation > 0.0);
    }

    #[test]
    fn system_boot_clock_is_available_and_monotonic() {
        let clock = SystemBoottimeClock::new().unwrap();
        let before = clock.now_ns().unwrap();
        let after = clock.now_ns().unwrap();
        assert!(after >= before);
    }
}
