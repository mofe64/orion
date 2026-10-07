# Orion hardware versions and V2 calibration

Start V2 setup with Orion's existing servo calibration and rest capture. V1
keeps its calibration and pose files. V2 selects its own files using
`--hardware v2`.

## Calibrate V2 first

Run from the Orion checkout on the Pi, with the installed runtime stopped:

```bash
uv sync --project hardware/servo_setup --locked
uv run --project hardware/servo_setup orion-verify-servos --hardware v2 \
  --port /dev/ttyACM0
uv run --project hardware/servo_setup orion-calibrate-servos --hardware v2 \
  --port /dev/ttyACM0
uv run --project hardware/servo_setup orion-capture-rest --hardware v2 \
  --port /dev/ttyACM0
```

Use the actual serial port. Calibration uses the same torque-off zero and range
capture as V1, including its existing endpoint margins and backups. Rest capture
uses the same unsupported stability check and writes the measured rest pose.

V2 calibration writes `~/.config/orion/servo_calibration-v2.json`. V2 rest capture
writes `~/.config/orion/poses-v2.yaml`, copying the V2 pose template on first use.
That template's other poses remain simulation candidates until adapted to the
assembled lamp. The captured calibration supplies servo IDs, encoder zeros,
directions and numerical limits to the V2 runtime.

Encoder direction defaults to `+1`, as in the existing V1 capture. Check the
fitted mechanism against the model's positive axes and set any reversed joint's
`encoder_direction` to `-1`. The optional `--directions FILE` argument supplies
those numerical values during capture. Home and other motion poses must use
these measured zeros and ranges; CAD geometry cannot supply encoder zeros.

## Select the hardware profile

Profiles live in `hardware/profiles/v1.json` and `hardware/profiles/v2.json`.
They select LED/audio interfaces, calibration filenames and asset paths. The
runtime reads the selected calibration file; it does not manufacture calibration
values from CAD.

| Interface | V1 | V2 |
| --- | --- | --- |
| LED | 40-pixel RGBW matrix | 24-pixel RGBW ring |
| LED driver | GPIO12, RP1 PWM, GRBW | Same GPIO12 driver |
| Audio | ReSpeaker 2-Mics V2 HAT, `seeed2micvoicec` | XVF3800 USB, `Array` |
| Calibration | `servo_calibration.json` | `servo_calibration-v2.json` |
| Simulation | `simulation/mujoco/scene.xml` | `simulation/mujoco/v2/scene.xml` |

The V2 mechanical tree is base → shoulder → elbow → wrist → neck. Existing API
keys remain stable:

| Expected servo ID | API key | V2 mechanism |
| --- | --- | --- |
| 1 | `base_yaw_joint` | Base yaw |
| 2 | `shoulder_pitch_joint` | Shoulder pitch |
| 3 | `elbow_pitch_joint` | Elbow pitch |
| 4 | `head_roll_joint` | Neck swivel |
| 5 | `head_pitch_joint` | Wrist pitch |

Verify physical ID assignments before capture. The zero-pose model rotates yaw
and swivel about +Z and pitch about +X, using the right-hand rule. Angles use
radians and model geometry uses metres.

Read calibrated hardware state without enabling torque:

```bash
runtime/target/release/oriond --hardware v2 --check --port /dev/ttyACM0
```

For motion development, use `--serve --hardware v2 --character-on-start off`
and the existing [hardware lifecycle](../runtime/README.md#physical-hardware).
V2 does not inherit V1's measured gravity-gain overrides.

## LED and USB audio setup

Reuse the [persistent GPIO12 installation](../hardware/lighting/README.md#persistent-raspberry-pi-5-setup)
for the ring. Verify it after reboot with `hardware/lighting/verify-persistent.sh`.
The module must match the running kernel. Add `--hardware v2` to Orion's direct
light/audio checks for the 24-pixel ring and USB playback.

### Identify USB capture

The XVF3800 capture profile has not been checked on the Pi. Inspect `arecord -l`,
`aplay -l` and ALSA hardware parameters, then select the installed two- or
six-channel stream and its processed voice channel. The listener accepts
`--hardware v2 --capture-channels 2|6 --processed-channel N`; these are ordinary
capture settings. V2 skips HAT mixer commands and raw-microphone direction
estimation. See the [Seeed USB guide](https://wiki.seeedstudio.com/respeaker_xvf3800_introduction/)
for firmware-specific channel layouts.

## CAD, URDF and MuJoCo

The V2.1 R4/R2 model is packaged with 33 portable visual meshes and a matching
`description/urdf/orion-v2.urdf`. A second CAD-to-URDF conversion is unnecessary
for geometry preview. Run the profile-aware daemon with
`--serve --backend mujoco --hardware v2`.

The model uses fixed-base kinematic tracking. Its metrics report
`validation_scope: "kinematic_preview"` and do not certify balance, collisions
or motor tracking. Existing standalone calibrated editors retain V1 assumptions.

`simulation/mujoco/v2/mass-properties.json` records CAD-derived centres and
inertias at a PLA density of 1.24 g/cm³. Printed defaults are solid PLA equivalents;
actual print weights depend on walls, infill and skins. Supplier masses use
explicit assumptions, and missing electronics, fasteners and cables are listed.
Use slicer part weights excluding supports, or measured weights, to improve them.

```bash
.venv/bin/python scripts/import_v2_model.py
.venv/bin/python scripts/estimate_v2_inertias.py --inputs /path/to/mass-inputs.json
```

The optional input follows `simulation/mujoco/v2/mass-inputs.example.json`:
`part_masses_g` maps mesh filenames to masses in grams; `extra_components` supplies
`name`, `body`, `mass_g`, `centre_m` and `size_m` for omitted hardware. Centres are
body-local and sizes are box dimensions in metres. Both URDF and MJCF receive the
same link mass, centre and inertia. Contacts and complete assembly dynamics
remain additional modelling work.

## Deploy the selected version

After calibration and motion validation, commit and push the intended revision.
Prepare the release from the workstation:

```bash
scripts/deploy_pi.sh --hardware v2 --host mofe@orion.local \
  --root /home/mofe/dev/orion --branch main --prepare-only
```

Use `--hardware v1` for V1. Follow the existing
[release installation](quickstart.md#deploy-to-the-pi) using the printed release
path. The installer validates calibration and compiles motions against its limits
before switching services. V2 startup defaults to maintenance mode; automatic
character startup is an explicit setting. Physical microphone, wake, transcription,
spoken-reply and Studio checks follow installation. The camera pipeline remains
separate work.
