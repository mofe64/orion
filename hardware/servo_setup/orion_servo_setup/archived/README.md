# Archived STS3215 motion commissioning tools

These modules powered Orion's one-off first-motion and named-pose experiments:

- `motion_test.py` and `motion_test_cli.py`
- `pose_execution.py` and `pose_cli.py`

They are not part of Orion's runtime control architecture. Their installed CLI
entry points were removed when the files were archived. Physical movement now
belongs behind the native Rust `oriond` runtime.

Calibration and rest-capture retain only the read-only preflight helper from
`motion_test.py`. The former pose executor and its legacy asset parser were removed;
all physical character motion now goes through the Rust v2 runtime.
