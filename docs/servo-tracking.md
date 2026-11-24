# Servo tracking tools

Orion's trajectory compiler plans ideal joint paths, while physical servos carrying a real arm lag slightly behind and show small tracking errors. Tracking tools let you record commanded versus measured joint positions during a run and measure tracking error, lag, settling and high-frequency jitter without disturbing the 50 Hz control loop.

Tracking is off by default, and the logger refuses to overwrite an existing output file.

## Control boundary

Each 50 Hz cycle reads servo feedback before writing the next goal, and tracking logs that exact cycle snapshot with no extra bus traffic. Records pass over a bounded queue to a background writer thread (dropping samples if full rather than delaying the motors, and draining cleanly on shutdown), tagged with `run_id`, trajectory `revision`, local trajectory time and terminal outcome.

## Enable capture

Make sure the output directory exists, pick a fresh filename, and stop the installed daemon first so only one process owns the serial bus and socket.

WARNING: Tracking playback moves powered joints. Keep physical support and an accessible power disconnect ready.

1. Start a V2 daemon with tracking enabled.

   ```bash
   runtime/target/release/oriond --serve --backend hardware --hardware v2 --tracking-log /tmp/orion-tracking.jsonl
   ```

2. Run the motion or speech test through Studio or the runtime CLI.
3. Return Orion to `rest` through the normal rest coordinator.
4. Stop the daemon once rest completes and torque is released.

## Analyze a capture

1. Generate JSON and Markdown summaries using Python's standard library.

   ```bash
   /opt/orion/python/bin/python3.12 runtime/scripts/analyze_tracking.py /tmp/orion-tracking.jsonl --json /tmp/orion-tracking.json --markdown /tmp/orion-tracking.md
   ```

Pass `--trials` to restrict analysis to run IDs from a trial file. The summary reports:

- **RMS error:** Time-weighted root-mean-square difference between commanded and measured joint angles.
- **Lag:** Estimated correlation delay between plan and response (note that low-motion or nonlinear traces can make correlation lag ambiguous).
- **Directional error:** High-frequency error split by positive motion, negative motion and stationary holds in joint coordinates.
- **Jitter:** Tracking error minus a 5-sample (100 ms at 50 Hz) moving average, omitting cycles with dropped ticks, intervals over 30 ms, trajectory replacements or settling.
- **Present speed:** Converted from signed counts/second using the servo `Phase` register (subject to encoder and timing quantization; measured movement peaks differ from compiled waypoints).

## Gain development

`--experimental-servo-gains JOINT:P:I` tests PID gains on a tracked shoulder or elbow run (requiring hardware mode with `--tracking-log`) without altering motion assets or completion thresholds.

Gain runs use `runtime/scripts/experiment_safety.py`; if a safety trip triggers, the daemon retains holding torque and writes `.experiment-safety-hold.json`.

WARNING: Killing a paused daemon can drop joint support suddenly. Support the arm physically before any recovery action.

1. Arrange physical support and supervised recovery after a safety hold.
2. Keep the safety hold active until the recovery procedure has control of the arm.

Because servo gain changes write to EEPROM (which has finite write endurance), the driver reads existing values first, writes only changed registers, verifies them, and relocks EEPROM.

## Speech timing analysis

When `oriond` logs `speech.motion_compiled` events, you can compare compiled gesture strokes against actual recorded joint motion:

1. Run the apex analyzer against matching tracking and daemon logs.

   ```bash
   /opt/orion/python/bin/python3.12 runtime/scripts/analyze_speech_apices.py hardware.jsonl daemon.log --json speech-apices.json
   ```

See [speech animation runtime](speech-animation-runtime.md) and [runtime controls](../runtime/README.md).
