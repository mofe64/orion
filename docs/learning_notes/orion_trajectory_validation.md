# Orion Trajectory Validation

This note explains Milestone 3 slice 3: the safety gate between a generated
motion and either execution backend.

## The Boundary

Generation and validation answer different questions:

```text
ResolvedTrajectory + measured stopped state
                    |
                    v
           GeneratedTrajectory
           exact desired motion
                    |
                    v
             ValidationReport
          all detected issues at once
                    |
             passing report only
                    v
           ValidatedTrajectory
          execution capability token
             /               \
            v                 v
    ROS controller         MuJoCo player
```

`GeneratedTrajectory` is intentionally inspectable even when it is unsafe.
That is necessary to explain every failure and calculate useful retiming
guidance. Neither backend accepts that raw type: both require a
`ValidatedTrajectory` wrapper produced by `require_valid_trajectory()`.

## What Is Checked

`trajectory_validator.py` collects rather than short-circuits on:

- Canonical joint names and ordering.
- Non-empty points and segments.
- Finite, positive, monotonically increasing timing.
- Complete finite position, velocity, and acceleration vectors.
- Mechanical and operational position bounds.
- Segment kind, duration, endpoint continuity, and stationary holds.
- Exact quintic peak velocity, acceleration, and jerk.
- Continuous intersection with configured forbidden joint-space regions.
- Agreement between first/last points, segments, and total duration.

Malformed configuration still raises `MotionValidationError`, because no
meaningful trajectory report can be produced against an invalid safety
contract. A well-formed but unsafe trajectory returns a `ValidationReport`.

## Complete Diagnostics

Each `ValidationIssue` includes a stable code and human-readable message. When
relevant, it also includes:

- Segment index and destination pose.
- Joint name.
- Measured peak and configured limit.
- Minimum duration for the violated dynamic constraint.
- Forbidden-region name.

For a quintic displacement `D` and configured limit `L`, the minimum duration
is calculated directly:

```text
velocity:      T >= 1.875 D / L
acceleration:  T >= sqrt((10 / sqrt(3)) D / L)
jerk:          T >= cube_root(60 D / L)
```

The report's per-segment duration requirement is the maximum of all applicable
joint and derivative requirements. It is guidance, not a mutation: authored
arrival and hold times remain unchanged so expressive timing changes stay
visible in motion source files and code review.

## Continuous Forbidden Regions

`config/forbidden_regions.yaml` describes an axis-aligned box in joint
configuration space. A region can constrain one or several joints. A
configuration is forbidden when every constrained joint lies inside its stated
interval at the same path coordinate.

The shared quintic applies the same monotonic scale factor to every joint, so a
transition traces a straight line in joint space even though its speed is
nonlinear in time. The validator intersects that line analytically with each
region. This checks the entire path, not a sample grid, and catches a crossing
even when both endpoints lie outside the region.

The project configuration currently contains:

```yaml
regions: []
```

That is deliberate. Orion has no evidence-backed self-collision region yet,
and inventing one would create a false safety claim. Synthetic unit tests use a
known region to prove continuous crossing behavior until simulator evidence
supports real project rules.

## Current Result

From the stopped `attentive` pose under the provisional simulation limits:

- All five functional motions pass and receive execution capabilities.
- All three expressive motions fail dynamic validation.
- `acknowledge_expressive` reports 9 issues rather than only the first.
- `look_at_left_expressive` reports 15 issues.
- `target_unreachable_expressive` reports 17 issues.

The expressive sources were not silently retimed. Their report requirements
are the inputs for a later explicit motion-authoring pass.

## Verification

```bash
cd /home/mofe/Desktop/dev/orion/ros2_ws
source /opt/ros/jazzy/setup.bash
colcon test --packages-select orion_motion --event-handlers console_direct+

cd /home/mofe/Desktop/dev/orion
.venv/bin/python -m unittest -q simulation/mujoco/test_mujoco_backend.py
.venv/bin/python simulation/mujoco/motion_player.py \
  look_at_left --start-pose attentive --lead-in 0 --headless
```

The unit suite includes a synthetic base-yaw region between `-0.70` and
`-0.60` radians. The `look_at_left` transition begins at `-0.30` and ends at
`-1.00`; both endpoints are outside, but validation rejects the continuous
crossing.
