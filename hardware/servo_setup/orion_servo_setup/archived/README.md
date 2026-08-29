# Archived STS3215 motion commissioning tools

These modules powered Orion's one-off first-motion and named-pose experiments:

- `motion_test.py` and `motion_test_cli.py`
- `pose_execution.py` and `pose_cli.py`

They are not part of Orion's runtime control architecture. Their installed CLI
entry points were removed when the files were archived. Physical movement now
belongs behind the native C++ `oriond` runtime.

Calibration and rest-capture code temporarily reuse read-only preflight and
calibration-loading helpers from these modules. Before deleting this directory,
move those shared helpers into active setup modules and update their imports.
The four matching unit-test files can then be deleted with the archived code.
