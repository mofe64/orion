# Orion Rust Runtime Port and Physical Parity Handover

## Header

- **Date:** 2026-08-29
- **Status:** Rust migration and physical feature parity complete; lighting, audio, scene orchestration, and Studio are next
- **Repository (workstation):** `/home/mofe/Desktop/dev/orion`
- **Repository (Raspberry Pi):** `/home/mofe/dev/orion`
- **Branch/HEAD before this handover:** `main` at `9371a6a` (`origin/main` is identical)
- **Working tree before this handover:** Clean
- **Primary objective:** Port the complete active Orion runtime from C++ to Rust, validate it in MuJoCo and on the assembled Raspberry Pi robot, remove the superseded implementation, make `runtime/` the sole runtime package, and align current documentation.
- **Tags:** `orion`, `rust`, `rustypot`, `native-runtime`, `sts3215`, `mujoco`, `hardware`, `feature-parity`, `runtime-removal`, `documentation`, `raspberry-pi`

## Continuation Update: Rust Is the Sole Active Runtime

This report supersedes the active-runtime conclusions in
`handover-2026-08-29.md`. That report remains an accurate historical record of
what existed at the time, but it must not be used as the current runtime file
map or architecture.

```text
motion/config + motion/motions
             |
             v
       runtime/oriond (Rust)
          |          |
          v          v
      rustypot    MuJoCo bridge
          |          |
          v          v
   physical Orion  simulated Orion
```

- `runtime/` is a Rust 2024 crate named `orion-runtime`.
- `rustypot = 1.6.0` provides packet parsing, serial communication, and
  synchronized STS3215 operations.
- Orion keeps an explicit raw register map and project-specific encoder,
  signed-telemetry, firmware-byte, and acceleration-register conversions.
- `runtime/mujoco_bridge.py` is used only by the MuJoCo backend. Physical
  hardware operation is Rust-only and does not require Python.
- No tracked C++ runtime, CMake build, C++ vendor SDK, or C++ source remains.
- The directory is `runtime/`, not `runtime_rust/`.
- The Pi builds and runs `oriond` from the source checkout. It is not installed
  as a systemd service and is not enabled at boot.

Key migration commits:

| Commit | Purpose |
|---|---|
| `d223166` | Add the Rust runtime, `rustypot` hardware transport, and Rust/MuJoCo backend |
| `29e518a` | Remove the superseded C++ runtime and vendor SDK |
| `302614d` | Rename `runtime_rust/` to the canonical `runtime/` directory |
| `9371a6a` | Make current documentation describe Rust as the confirmed sole runtime |

The tag `cpp-runtime-final-2026-08-29` preserves the final comparison baseline
in Git history. It is not an active implementation.

## What Was Worked On

### Complete native Rust runtime

The Rust implementation preserves the active command contract:

- `--check`
- `--serve`
- `--status`
- `--configure`
- `--enable`
- `--goto POSE --duration SECONDS`
- `--play MOTION`
- `--stop`
- `--disable`

The daemon owns the serial bus, samples at 50 Hz, exposes versioned JSON state
through `/tmp/oriond.sock`, reads the shared pose/motion YAML, and uses:

```text
observe -> configured -> holding -> moving -> holding
```

The port includes calibration loading, raw/radian conversion across the
circular 12-bit encoder domain, synchronized five-servo reads/writes, Orion's
STS3215 profile and elbow gain override, measured-position seeding before
torque enable, torque/configuration/fault refusal paths, calibration-bound
target validation, quintic named-pose trajectories, authored keyframe motion,
status snapshots, stop, and disable.

### Native MuJoCo execution

The Rust daemon launches `runtime/mujoco_bridge.py` and drives the same Orion
model, poses, motions, lifecycle, and commands used by hardware. The bridge
reports measured position/velocity and stability metrics.

This is intentionally a hybrid simulator boundary:

```text
Rust daemon -> Python bridge -> Python MuJoCo bindings
```

Python is not in the STS3215 hardware path. Full Rust tests invoke the MuJoCo
integration and expect `.venv/bin/python`; hardware-only Pi use does not.

### Physical Raspberry Pi parity trial

The Rust runtime was built and exercised on the assembled aarch64 Pi at
`/home/mofe/dev/orion` against `/dev/ttyACM0` and the authoritative live file:

`/home/mofe/.config/orion/servo_calibration.json`

A direct C++/Rust torque-off state comparison matched joint position, velocity,
current, voltage, and status. The only observed difference was one shoulder
temperature sample changing from `29 C` to `28 C`, normal sampling variation.

The supplied physical Rust transcript completed:

1. torque-off observation and status
2. profile configuration
3. torque enable and holding
4. `zero_reference` in 5 seconds
5. `attentive` in 3 seconds
6. `home` in 3 seconds
7. functional `look_at_left`
8. expressive `look_at_left_expressive`
9. start `look_right`, then stop while moving
10. return to captured `rest`
11. confirm holding, disable, and read final torque-off state

All servos reported about `6.2 V`, `27–29 C`, and status `0`. Mid-motion stop
returned to holding correctly. Final torque-off state was within approximately
`0.031 rad` of the captured rest target on every joint.

### Repository and documentation cleanup

The former C++ tree, CMake files, headers, tests, and vendored FEETECH SDK were
deleted. The Rust crate was renamed from `runtime_rust/` to `runtime/`; its
MuJoCo path, ignore rule, commands, architecture docs, and operator docs were
updated.

Every active and archived README/document now names Rust as Orion's sole
runtime and does not present hardware validation as pending. Dated `.scratch`
reports retain C++ references as project history; this report and the index are
the authoritative current handover.

`runtime/README.md` now documents the complete two-terminal Pi workflow and:

```text
goto rest -> poll status until holding -> disable -> stop foreground daemon
```

## What Got Done

- Implemented all active `oriond` logic in Rust.
- Added a pure-Rust `rustypot` STS3215 transport and Orion register boundary.
- Preserved calibration, pose, motion, quintic, state, and socket semantics.
- Added a Rust-owned MuJoCo backend through the existing Python bindings.
- Passed the full workstation Rust and repository test matrix.
- Compared C++ and Rust hardware telemetry.
- Executed poses, functional/expressive motion, cancellation, rest, and disable
  on the physical robot through Rust.
- Deleted all tracked C++ runtime code and vendor sources.
- Renamed the sole runtime package to `runtime/`.
- Documented source-tree Pi operation without systemd.
- Documented confirmed-rest-before-disable normal shutdown.

## Bugs Fixed

1. **Two runtime implementations remained.** Removed the C++ implementation
   and vendor SDK after Rust parity was proven.
2. **The Rust package name implied a temporary parallel port.** Renamed
   `runtime_rust/` to `runtime/`.
3. **MuJoCo resolved the old package path.** Updated the bridge lookup to
   `runtime/mujoco_bridge.py`.
4. **Documents described C++ ownership or a pending hardware gate.** Current
   documentation now identifies confirmed Rust ownership.
5. **The Pi guide could disable immediately after requesting rest.** It now
   requires status to return to `holding` first.
6. **A missing Pi `.venv` looked like a missing bridge.** The missing optional
   Python executable was identified; hardware operation does not need it.

## Key Decisions & Why

### Rust owns physical control

Rust is not a provisional port. It is Orion's sole runtime for serial
ownership, lifecycle, motion execution, telemetry, and the hardware boundary.

### Use rustypot without hiding Orion's register contract

`rustypot` removes the C++ SDK wrapper, while explicit Orion register widths
and conversions keep the transport reviewable against physically tested
hardware.

### Run from source until the implementation matures

Do not create a systemd unit, copy a binary into a system path, or enable a boot
service yet. Build `runtime/Cargo.toml` and run
`runtime/target/release/oriond` from `/home/mofe/dev/orion`.

### Keep Python out of the hardware dependency chain

Python remains useful for MuJoCo, validation, calibration, and authoring. It is
not required for the physical daemon. Install a Pi venv only if simulator tests
are intentionally being run there.

### Rest completion precedes normal torque disable

`--goto` is asynchronous. Success means accepted, not completed. Poll status
until `"mode":"holding"` after `--goto rest`, then issue `--disable`.

### Define the local scene runtime before Studio

Lighting, audio, and motion need one portable scene schema and one local clock.
Studio should author, preview, validate, and transfer complete scenes; it must
not become a second hardware controller.

## Lessons Learned & Gotchas

- Run source commands from the repository root; default paths are relative.
- The Pi path is `/home/mofe/dev/orion/runtime/`, not `runtime_rust/`.
- Full `cargo test --all-targets` needs `.venv/bin/python` for MuJoCo.
- Hardware-only Pi tests can skip it:

  ```bash
  cargo test --manifest-path runtime/Cargo.toml --all-targets -- \
    --skip mujoco::tests::rust_runtime_executes_and_settles_in_native_mujoco
  ```

- The simulator backend still uses Python MuJoCo bindings.
- The daemon is a foreground process, not a service; only one process may own
  `/dev/ttyACM0`.
- The live calibration file on the Pi is authoritative and outside Git.
- `--check` reads torque-off state and does not enable torque or write profile
  registers.
- `--goto` and `--play` return on acceptance. Poll status before the next move.
- Busy motion still reports the misleading `enable holding torque before
  moving` error.
- `--stop` freezes the latest command and holds; it is not controlled braking.
- `--disable` during movement removes torque immediately. Reach rest first for
  normal shutdown.
- Neither `--disable` nor killing the daemon is a physical emergency stop.
- Tracking persistence, communication watchdog policy, inspectable fault
  states, and measured-settling completion are not implemented.
- Do not restore C++ or ROS for lighting, audio, Studio, or orchestration.
- Two test/source compatibility labels still mention C++ history; they are not
  active dependencies and can be renamed opportunistically.

## Clear Next Steps

The user has not selected the next slice after the Rust migration. Do not
assume command-lifecycle hardening is automatically first; the user previously
redirected that proposal to complete the Rust port. Confirm scope, then follow
this product order.

### Priority 1: Establish lighting and audio device boundaries

1. Audit the installed LED chipset/protocol, RGB/RGBW order, pixel count, level
   shifting, power source, and reported BCM GPIO12 data path.
2. Add a minimal Rust lighting interface with a fake backend before the Pi
   driver. Keep lighting failures isolated from motor control.
3. Enumerate ReSpeaker HAT ALSA devices on the Pi and record a stable playback
   identifier for the intended differential approximately 4-ohm speaker.
4. Add a minimal audio interface and one local sound playback command before
   voice, microphones, or cloud-agent work.

The 2026-08-28 hardware handover contains reported wiring, but verify the
assembled robot before treating those GPIO/pixel details as a contract.

### Priority 2: Define and play a multimodal scene

1. Define a versioned schema for motion references, lighting/audio events,
   transitions, holds, loops, and metadata.
2. Validate complete scenes before playback; keep motor validation in Rust.
3. Execute locally with one monotonic clock so playback survives editor loss.
4. Add one fake-device/MuJoCo scene test, then a Pi trial.

### Priority 3: Build the first Orion Studio slice

1. Build Studio as an external authoring client, not a safety controller.
2. Start with 3D model display, named-pose editing, a basic timeline,
   light-state editing, preview, and scene export.
3. Transfer complete scenes rather than streaming frames.
4. Reuse shared pose, motion, description, and scene schemas.

### Priority 4: Address runtime debt when selected

1. Correct busy-motion errors and add status-aware waiting.
2. Add command IDs/results if the user selects lifecycle v2.
3. Add measured settling, tracking persistence, watchdog and telemetry policy,
   explicit fault reasons, and controlled stopping.
4. Distinguish normal rest-confirm-disable shutdown from a real hardware
   emergency-stop mechanism.

### Priority 5: Close Pi and repository loose ends

1. Confirm the Pi is at `9371a6a` or later and uses the source-tree binary.
2. Check whether `/home/mofe/.local/bin/oriond-rust` remains as an earlier test
   artifact; do not treat it as authoritative.
3. Confirm whether stash `pre-origin-main-alignment-2026-08-29` is still needed
   before removing it.
4. Back up `/home/mofe/.config/orion/servo_calibration.json` outside Git.

## Map of Important Files

| Path | Purpose |
|---|---|
| `runtime/Cargo.toml` | Rust crate and pinned runtime dependencies |
| `runtime/src/main.rs` | CLI, backend selection, and 50 Hz foreground loop |
| `runtime/src/transport.rs` | `rustypot` serial and Orion register semantics |
| `runtime/src/driver.rs` | Calibration, servo profile, synchronized I/O, torque lifecycle |
| `runtime/src/daemon.rs` | Backend-independent command/motion state machine |
| `runtime/src/socket.rs` | Unix socket server/client |
| `runtime/src/state.rs` | Versioned telemetry/lifecycle snapshot |
| `runtime/src/pose.rs` | Shared named-pose loading |
| `runtime/src/motion.rs` | Authored-motion loading and sampling |
| `runtime/src/trajectory.rs` | Quintic pose interpolation |
| `runtime/src/mujoco.rs` | Rust MuJoCo boundary and integration test |
| `runtime/mujoco_bridge.py` | JSON bridge to Python MuJoCo bindings |
| `runtime/README.md` | Build, simulation, Pi operation, and shutdown guide |
| `motion/config/poses.yaml` | Shared poses including physical zero/rest |
| `motion/motions/` | Functional and expressive motions |
| `simulation/mujoco/scene.xml` | Default Rust-bridge simulation scene |
| `simulation/mujoco/requirements.txt` | Optional Python MuJoCo dependencies |
| `simulation/mujoco/config/servo_calibration.json` | Simulator calibration mirror |
| `hardware/servo_setup/README.md` | Commissioning and calibration guide |
| `docs/orion_control_architecture.md` | Current Rust control architecture |
| `docs/Orion Guidebook.md` | Roadmap, scene milestone, and Studio boundary |
| `/home/mofe/.config/orion/servo_calibration.json` (Pi only) | Live physical calibration |

There is no active LED driver, speaker/audio integration, multimodal scene
player, or `orion_studio/` implementation yet.

## Test Execution Notes

Complete workstation matrix before final documentation updates:

- **29 Rust tests:** 26 library and 3 CLI.
- **72 motion tests.**
- **56 servo-setup tests.**
- **21 standalone MuJoCo tests.**
- **4 robot-description tests.**
- **182 total**, no failures.

After renaming the directory, all 29 Rust and 21 standalone MuJoCo tests passed
again. Path audits found no tracked `runtime_rust/`, C++ source, or diff error.

On the Pi, 25 of 26 library tests passed before Cargo stopped at MuJoCo. The
bridge existed; `.venv/bin/python` did not. This is an optional simulator
environment issue, not a physical runtime failure.

## Current Raspberry Pi Workflow

```bash
cd /home/mofe/dev/orion
cargo build --manifest-path runtime/Cargo.toml --release --locked

runtime/target/release/oriond --check \
  --port /dev/ttyACM0 \
  --calibration /home/mofe/.config/orion/servo_calibration.json
```

Terminal 1 owns hardware:

```bash
runtime/target/release/oriond --serve \
  --backend hardware \
  --port /dev/ttyACM0 \
  --baud-rate 1000000 \
  --calibration /home/mofe/.config/orion/servo_calibration.json
```

Terminal 2 sends commands. Normal shutdown:

```bash
runtime/target/release/oriond --goto rest --duration 3.0
runtime/target/release/oriond --status
# Repeat status until mode is holding.
runtime/target/release/oriond --disable
```

Then stop Terminal 1 with `Ctrl+C`.

## Gotchas for Next Agent

- Read this report, `runtime/README.md`, the control architecture, and the
  relevant Guidebook milestone before changing architecture.
- Treat prior C++ reports as history, not current guidance.
- Do not recreate `runtime_rust/`, reinstall C++, or restore ROS.
- Do not install `oriond` as a service unless the user changes that decision.
- Do not tell the user Python is required for hardware operation.
- Do not disable immediately after requesting rest; confirm holding first.
- Do not build Studio before the scene contract it will author.
- Do not let Studio, lighting, or audio write servo registers directly.
- Preserve joint names, calibration conventions, schemas, and simulator
  assumptions.
- Preserve unrelated worktree changes and stage explicit paths when committing.
- Keep Pi guidance practical and source-based while the runtime is developing.

## Session Statistics

- Active native runtimes: 1 (Rust)
- Tracked C++ implementation/source files: 0
- Rust runtime source modules: 13
- Runtime command families: 9
- Physical servos controlled through Rust: 5/5
- Physical Rust coverage: configure, enable, 3 poses, 2 authored motions,
  mid-motion stop, rest, and disable
- Workstation tests passing: 182
- Commits since `e817682`: 5
- Branch before handover: `main` at `9371a6a`, matching `origin/main`
- Uncommitted changes before handover: 0
- Session elapsed time and token count: unknown

## Handoff Checklist

- [x] Rust hardware transport and driver implemented
- [x] Rust daemon lifecycle/socket implemented
- [x] Shared poses and motions load in Rust
- [x] Rust MuJoCo backend implemented and tested
- [x] Full workstation matrix passed
- [x] C++/Rust hardware telemetry compared
- [x] Physical poses, motions, stop, rest, and disable executed through Rust
- [x] C++ runtime and vendor SDK removed
- [x] Rust package renamed to `runtime/`
- [x] Current docs name Rust as sole confirmed runtime
- [x] Pi source-tree workflow and normal shutdown documented
- [ ] Select the next product slice with the user
- [ ] Implement LED control
- [ ] Integrate ReSpeaker speaker playback
- [ ] Define and implement the local scene contract/player
- [ ] Build the first Orion Studio slice
- [ ] Revisit runtime hardening when selected
- [ ] Confirm and clean stale Pi artifacts when authorized

## End Matter

Orion has completed its runtime migration. The robot, MuJoCo, shared motions,
calibration contract, and operator workflow now converge on one Rust `oriond`.
The next agent should not reopen C++/ROS/runtime selection or treat Rust
hardware validation as pending.

The recommended product path is verified LED and speaker device boundaries,
then a local multimodal scene format/player, then Orion Studio as an authoring
client. Runtime hardening remains technical debt, but the next slice should be
confirmed with the user rather than substituted for the requested
lighting/audio/Studio direction.
