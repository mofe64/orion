# 0001: Use a native Rust runtime

- **Status:** Accepted
- **Scope:** Raspberry Pi runtime and MuJoCo control

## Context

Orion needs deterministic device ownership, explicit lifecycle state, bounded
motion execution, and a simulator path that exercises the same control logic
as physical hardware. Earlier framework experiments introduced boundaries that
were not required by the single-robot product.

## Decision

Use the ROS-independent Rust `oriond` daemon as Orion's runtime. Physical
STS3215 hardware and MuJoCo implement the same driver-facing state machine.
Keep poses, motions, calibration conversion, trajectory generation, scene
coordination, and run lifecycle inside this runtime boundary.

## Consequences

- Simulation can validate the same semantic operations used on hardware.
- The Pi has one long-running owner for motion and device state.
- Other processes integrate through a small command protocol rather than
  importing or reimplementing control logic.
- Any distributed robotics framework must adapt to this boundary instead
  of replacing it implicitly.
