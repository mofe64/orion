# Run the Orion runtime in MuJoCo

The MuJoCo backend runs the same `oriond` state machine used on the robot
without accessing physical hardware.

## Prerequisites

Install:

- A stable Rust toolchain with Rust 2024 edition support (Rust 1.85 or newer).
- Python 3.12.
- `uv`, or another way to create a Python virtual environment.

Run every command below from the Orion repository root.

## 1. Create the simulator environment

```bash
uv venv .venv --python 3.12
uv pip install --python .venv/bin/python \
  -r simulation/mujoco/requirements.txt \
  pytest
```

The runtime defaults to `.venv/bin/python` when it launches the MuJoCo bridge.
The motion library is imported from the checkout with `PYTHONPATH`; it does not
need an editable package installation.

## 2. Build and test the runtime

```bash
cargo build --manifest-path runtime/Cargo.toml
cargo test --manifest-path runtime/Cargo.toml --all-targets
PYTHONPATH=motion .venv/bin/python -m pytest -q motion/test
```

The test suite exercises the runtime contract and launches the MuJoCo model
through the normal daemon state machine.

## 3. Start `oriond`

In the first terminal:

```bash
runtime/target/debug/oriond --serve --backend mujoco \
  --start-pose attentive
```

Leave it running. It owns the default private command socket at
`/tmp/oriond.sock`.

## 4. Submit a movement

In a second terminal:

```bash
runtime/target/debug/oriond --status
runtime/target/debug/oriond --configure
runtime/target/debug/oriond --enable
runtime/target/debug/oriond --goto home --duration 3.0 --wait
runtime/target/debug/oriond --play look_at_left_expressive --wait
```

`--wait` follows the accepted run ID until the movement completes, times out,
or is cancelled.

## 5. Shut down cleanly

```bash
runtime/target/debug/oriond --goto rest --duration 3.0 --wait
runtime/target/debug/oriond --disable
```

Stop the first terminal with `Ctrl+C`.

You have used Orion's normal lifecycle, named asset library, interpolation,
and completion logic without opening a serial or GPIO device. Continue with
the [runtime reference](../../runtime/README.md) or
[run Orion Studio](first-studio-run.md).
