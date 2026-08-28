# Handover Reports Index

> This index helps you quickly find relevant handover reports related to your current task.

**Last Updated:** 2026-08-28
**Total Reports:** 1

---

## Quick Search

### By Topic
- [Testing](#testing)
- [Bug Fixes](#bug-fixes)
- [Features](#features)
- [Refactoring](#refactoring)
- [Documentation](#documentation)
- [Hardware](#hardware)
- [ROS 2 Control](#ros-2-control)
- [Simulation](#simulation)

### By Status
- [Complete](#complete)
- [In Progress](#in-progress)
- [Blocked](#blocked)

---

## All Reports (Reverse Chronological)

### 2026-08-28 - Orion hardware bring-up and physical-control handoff ⏳
**File:** [handover-2026-08-28.md](./handover-2026-08-28.md)
**Primary Task:** Assemble and commission the five-servo Orion lamp, capture safe calibration/rest data, and prepare the physical `ros2_control` implementation.
**Tags:** `orion`, `sts3215`, `servo-setup`, `calibration`, `rest-pose`, `pose-execution`, `ros2-control`, `mujoco`, `hardware`, `safety`
**Key Deliverables:**
- Five-servo provisioning, verification, first-motion, calibration, rest-capture, and guarded pose-runner workflows
- Accepted torque-free rest pose and a concrete MuJoCo-torque/physical-ROS-control continuation plan
**Bugs Fixed:** 6
**Files Changed:** 41 across the commissioning series, plus this report and index
**Next Steps:** Retrieve the live Pi calibration JSON, characterize joint torque in MuJoCo, and implement `orion_hardware/STS3215System`.

---

## Topic Index

### Testing
Reports related to testing work:
- [2026-08-28](./handover-2026-08-28.md) - Documents 46 passing servo-setup tests, four strict nudge passes, and unresolved elbow/pose tracking failures.

### Bug Fixes
Reports with significant bug fixes:
- [2026-08-28](./handover-2026-08-28.md) - Fixed yaw calibration rejection, rest shutdown behaviour, shared rest ranges, Ctrl+C parking, and temperature diagnostics.

### Features
Reports about feature development:
- [2026-08-28](./handover-2026-08-28.md) - Servo provisioning, verification, guarded motion, calibration, rest capture, and physical pose commissioning.

### Refactoring
Reports about refactoring:
- None.

### Documentation
Reports about documentation:
- [2026-08-28](./handover-2026-08-28.md) - Consolidates assembly, wiring, calibration, safety, code state, and next architecture steps.

### Hardware
Reports about physical hardware:
- [2026-08-28](./handover-2026-08-28.md) - Five STS3215 servos assembled and detected; current physical test results and safety constraints recorded.

### ROS 2 Control
Reports about ROS 2 control:
- [2026-08-28](./handover-2026-08-28.md) - Scopes the future `orion_hardware/STS3215System` and graceful shutdown orchestration.

### Simulation
Reports about simulation:
- [2026-08-28](./handover-2026-08-28.md) - Scopes MuJoCo inverse-dynamics torque characterization and identifies unrealistic current actuator force caps.

---

## Status Index

### Complete
- None.

### In Progress
- [2026-08-28](./handover-2026-08-28.md) - Bring-up infrastructure is complete; torque characterization and physical ROS control remain.

### Blocked
- None. Physical progress requires the live Pi calibration JSON and cautious access to the assembled lamp, but implementation planning can continue.

---

**Index Maintained By:** Handover Skill
