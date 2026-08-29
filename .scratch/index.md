# Handover Reports Index

> This index is the retrieval layer for Orion handovers. Reports are ordered newest first; use the topic and status sections to find the authoritative context for the work in front of you.

**Last Updated:** 2026-08-29
**Total Reports:** 2

---

## Quick Search

### By Topic

- [Testing](#testing)
- [Bug Fixes](#bug-fixes)
- [Features](#features)
- [Refactoring](#refactoring)
- [Documentation](#documentation)
- [Hardware](#hardware)
- [Native Runtime](#native-runtime)
- [Simulation](#simulation)
- [ROS 2 History](#ros-2-history)

### By Status

- [Complete](#complete)
- [In Progress](#in-progress)
- [Blocked](#blocked)

---

## All Reports (Reverse Chronological)

### 2026-08-29 - Orion native runtime, recalibration, and ROS 2 removal ✅

**File:** [handover-2026-08-29.md](./handover-2026-08-29.md)

**Primary Task:** Recalibrate the assembled robot, align shared poses and MuJoCo with the physical reference, implement and physically validate the native C++ runtime, and remove the active ROS 2 dependency.

**Tags:** `orion`, `native-runtime`, `c++`, `sts3215`, `calibration`, `rest-pose`, `motions`, `mujoco`, `ros2-removal`, `hardware`, `testing`

**Key Deliverables:**

- Accepted five-joint calibration and hardware-derived torque-free rest pose
- Native `oriond` lifecycle, status, named-pose, multi-keyframe motion, stop, and disable paths
- Shared ROS-independent `motion/` and `description/` assets consumed by hardware and MuJoCo
- Physical pose/motion/rest smoke test plus 174 passing portable/native tests

**Bugs Fixed:** 9

**Files Changed:** 180 files across the work since the previous handover baseline, plus this report/index update

**Next Steps:** Add status-aware sequencing, correct the busy-motion error, harden tracking/watchdog/obstacle handling, and physically tune the slowed right expressive motion.

### 2026-08-28 - Orion hardware bring-up and physical-control handoff ⏳

**File:** [handover-2026-08-28.md](./handover-2026-08-28.md)

**Primary Task:** Assemble and commission the five-servo Orion lamp, capture initial calibration/rest data, and prepare the first physical control backend.

**Tags:** `orion`, `sts3215`, `servo-setup`, `calibration`, `rest-pose`, `pose-execution`, `ros2-control`, `mujoco`, `hardware`, `safety`

**Key Deliverables:**

- Five-servo provisioning, verification, first-motion, calibration, rest-capture, and guarded pose-runner workflows
- Initial compiled/unit-tested `ros2_control` backend and the evidence that motivated the native runtime

**Bugs Fixed:** 6

**Files Changed:** 41 across the commissioning series, plus its report/index update

**Next Steps at the time:** Commission ROS control and characterize torque. These directions were superseded by the 2026-08-29 native-runtime handover; retain this report as bring-up history.

---

## Topic Index

### Testing

- [2026-08-29](./handover-2026-08-29.md) - Records 174 passing tests, a ROS-free native build, a valid MuJoCo load, and the successful post-cleanup physical smoke sequence.
- [2026-08-28](./handover-2026-08-28.md) - Records 46 commissioning tests, strict nudge results, and the original elbow/pose tracking failures.

### Bug Fixes

- [2026-08-29](./handover-2026-08-29.md) - Fixes calibration display/range handling, supported endpoints, margin starts, simulation reference alignment, stale pose families, command errors, Pi Git divergence, and backend paths.
- [2026-08-28](./handover-2026-08-28.md) - Fixes early yaw calibration rejection, rest shutdown behaviour, shared rest ranges, Ctrl+C parking, and temperature diagnostics.

### Features

- [2026-08-29](./handover-2026-08-29.md) - Native daemon, physical pose/motion playback, constrained MuJoCo pose editor, supported-rest capture, and rebuilt expressive motions.
- [2026-08-28](./handover-2026-08-28.md) - Servo provisioning, verification, guarded motion, initial calibration, rest capture, and physical pose commissioning.

### Refactoring

- [2026-08-29](./handover-2026-08-29.md) - Moves shared assets to `motion/` and `description/`, removes `ros2_ws/`, archives ROS documents, and deduplicates meshes.

### Documentation

- [2026-08-29](./handover-2026-08-29.md) - Records the accepted hardware contract, native architecture, current commands, migration evidence, risks, and next runtime milestone.
- [2026-08-28](./handover-2026-08-28.md) - Consolidates original assembly, wiring, calibration, safety, and planned control-backend context.

### Hardware

- [2026-08-29](./handover-2026-08-29.md) - Authoritative calibration, supported endpoints, 120 g head correction, captured rest, servo profile, and real motion evidence.
- [2026-08-28](./handover-2026-08-28.md) - Initial five-servo assembly, wiring, bus detection, and bring-up constraints.

### Native Runtime

- [2026-08-29](./handover-2026-08-29.md) - Current source of truth for `oriond`, C++ STS3215 ownership, daemon lifecycle, poses, motions, and known runtime gaps.

### Simulation

- [2026-08-29](./handover-2026-08-29.md) - Aligns MuJoCo with captured physical zero/rest, shared meshes, constrained pose editing, and torque analysis.
- [2026-08-28](./handover-2026-08-28.md) - Records the earlier torque-characterization scope and warns about unrealistic provisional actuator limits.

### ROS 2 History

- [2026-08-29](./handover-2026-08-29.md) - Explains why the active ROS workspace was removed and where useful historical design documents were archived.
- [2026-08-28](./handover-2026-08-28.md) - Historical record of the first `ros2_control` implementation before the architecture changed.

---

## Status Index

### Complete

- [2026-08-29](./handover-2026-08-29.md) - Recalibration, shared-motion rebuild, native runtime, MuJoCo alignment, physical proof, and ROS removal are complete for this phase.

### In Progress

- [2026-08-28](./handover-2026-08-28.md) - Historical snapshot of an in-progress ROS control plan; its open architecture work was superseded by the 2026-08-29 report.

### Blocked

- None. Further physical validation requires access to the assembled Pi/robot, but the current implementation is operational and the next software work is unblocked.

---

**Index Maintained By:** Orion project handover process
