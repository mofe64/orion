# Servo tracking tools

Tracking records commanded and measured joint positions during movement and settling. It is disabled by default. The logger refuses to overwrite an existing file.

## Control boundary

The 50 Hz loop reads feedback before writing its next goal. Tracking includes that normal one-cycle ordering. It does not add servo writes.

A bounded queue sends records to a writer thread. Full queues drop records rather than delay movement. Serialization and file writes stay outside the movement loop. Graceful shutdown drains the queue.

Records include run ID, trajectory revision, local trajectory time and terminal outcome. Speech replacements retain the run ID while changing revision.

## Enable capture

The output directory must exist. Use a unique file name. Stop the installed daemon before starting another owner of its socket and servo port.

WARNING: Tracking playback moves powered joints. Keep support and a physical power disconnect available.

1. Start a V2 daemon with tracking enabled.

   ```bash
   runtime/target/release/oriond --serve --backend hardware --hardware v2 --tracking-log /tmp/orion-tracking.jsonl
   ```

2. Run the required motion through Studio or the runtime client.
3. Return Orion to rest through the normal rest coordinator.
4. Stop the daemon after completed rest and torque release.

## Analyze a capture

The analysis tool uses Python's standard library.

1. Produce JSON and Markdown summaries from a capture.

   ```bash
   /opt/orion/python/bin/python3.12 runtime/scripts/analyze_tracking.py /tmp/orion-tracking.jsonl --json /tmp/orion-tracking.json --markdown /tmp/orion-tracking.md
   ```

`--trials` restricts analysis to run IDs from a trial file. The report includes tracking error, excursion, lag, settling and queue loss.

RMS error is the time-weighted root mean square of commanded-minus-measured position. Lag estimates a correlation delay. Low-motion or nonlinear traces can make that delay ambiguous.

Present speed uses counts per second with the configured Phase register. Position derivatives also contain encoder and timing quantization. A compiled waypoint and a measured movement peak are different quantities.

## Gain development

`--experimental-servo-gains JOINT:P:I` permits a tracked shoulder or elbow configuration. It requires hardware serving with `--tracking-log`. It leaves motion assets and completion thresholds unchanged.

Gain tools use `runtime/scripts/experiment_safety.py`. A safety trip retains holding torque and creates `.experiment-safety-hold.json`. A paused daemon supplies no fresh feedback.

WARNING: Killing a paused daemon can release joint support. Support the arm before any recovery action.

1. Arrange supervised recovery after a safety hold.
2. Keep the existing hold until the recovery procedure controls the arm.

Servo gain changes write EEPROM. Repeated alternating configurations consume write endurance. The driver reads first, writes changed values, verifies them and relocks EEPROM.

## Speech timing analysis

The runtime emits `speech.motion_compiled` events with compiled stroke timing. The analyzer compares those events with recorded joint motion.

1. Analyze head-stroke timing with matching capture and daemon logs.

   ```bash
   /opt/orion/python/bin/python3.12 runtime/scripts/analyze_speech_apices.py hardware.jsonl daemon.log --json speech-apices.json
   ```

See [speech planning](speech-animation-runtime.md) and [runtime controls](../runtime/README.md).
