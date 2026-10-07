# Servo tracking capture

`oriond --serve --tracking-log FILE` records commanded and measured joint
positions while a movement executes and settles. Capture is off by default.
The file must not exist; the logger refuses to overwrite an earlier experiment.
Its parent directory must exist and be writable by the daemon user.

## Timing and control boundary

`RuntimeCore` reads feedback before sampling and writing the new trajectory
goal on each 50 Hz cycle. A sample contains the exact floating-point position
passed to the driver, before encoder rounding, together with that cycle's
measured position and velocity. During settling, the logged command remains
the last position written; capture does not introduce extra servo writes.
Errors and lag therefore include the normal feedback-before-write delay.

A bounded 512-record queue connects the controller to a dedicated writer.
The controller uses `try_send`; full or disconnected queues drop records
instead of waiting. Serialization, file writes and 250 ms flushes occur on the
writer thread. A writer failure reports to stderr and leaves control running.
Graceful daemon teardown drains the queue and writes a summary with the dropped
record count. Hard termination can lose buffered records and the summary.

Samples retain an increasing `elapsed_seconds` from the original movement
start. Speech replacements keep their existing run ID, so capture also records
`trajectory_revision`, the replacement's actual `motion_name`, and its local
`trajectory_elapsed_seconds`. The run's original status label remains
`run_name`. Terminal records distinguish completion, cancellation and timeout.

The header records backend and build revision. Archive the active calibration,
motion assets, executable hash and source diff with hardware experiments;
the JSONL alone does not identify those inputs.

## Feedback velocity units

Hardware `measured_velocity_rad_s` uses `2π / 4096` rad/s per signed Present
Speed count/s, followed by the calibrated encoder direction. This requires
Phase register 18 bit 2 set, as verified on all five Orion servos. See the
[control reference](trajectory-and-joint-control.md#calibration-and-radians-conversion)
for the register source and conversion contract.

Captures from builds that predate the count/s conversion report hardware speed
49.9712 times too high. Do not silently reinterpret those files or apply this
correction to MuJoCo logs. Position-based excursion,
tracking error and lag calculations do not use the velocity field;
`analyze_tracking.py`'s maximum measured velocity does.

The servos report speed in steps of 50 counts/s (0.0767 rad/s). Count/s is the
unit, not a guarantee of one-count/s measurement precision. Adjacent-position
derivatives also have encoder and timing quantization.

## Enable and analyze

On the Pi, stop the existing daemon with its normal safe-stop mechanism before
starting another process on the same socket or serial device. From the project
root, using a telemetry-enabled binary:

```bash
runtime/target/release/oriond --serve --backend hardware \
  --port /dev/ttyACM0 --baud-rate 1000000 \
  --calibration /home/mofe/.config/orion/servo_calibration.json \
  --tracking-log /tmp/orion-tracking-hardware.jsonl
```

For an installed service, add `--tracking-log FILE` to its effective `ExecStart`
using a temporary systemd override and restore the previous command afterward.
Keep the existing executable, calibration, asset paths and other arguments.
Terminate cleanly after returning to mechanical rest so the summary is flushed.

The analysis uses only Python's standard library:

```bash
python3 runtime/scripts/analyze_tracking.py /tmp/orion-tracking-hardware.jsonl \
  --trials /tmp/orion-tracking-trials/trials.jsonl \
  --json /tmp/orion-tracking-analysis.json \
  --markdown /tmp/orion-tracking-analysis.md
```

## Repeatable hardware trials

This command physically moves Orion and plays three spoken utterances. Run it
with the hardware daemon already awake at home. Pause microphone capture if
accidental wake-ups would interfere, and restore it after the trial.

```bash
python3 runtime/scripts/run_tracking_experiment.py \
  --motions motion/motions/v1/idle --repeats 3 \
  --output /tmp/orion-tracking-trials \
  --speech-python /path/to/active-release/speech/.venv/bin/python \
  --speech-root /path/to/active-release/speech \
  --piper-model /home/mofe/.local/share/orion/voice-stack/models/piper-alba-medium
```

The runner stops automatic character motion for explicit idle trials, requests
`goto home 3.0` before every clip, then sends `play CLIP` over the runtime socket.
It runs each YAML clip directly under the idle directory three times by default.
`--idles-only` skips speech and its model arguments; `--speech-only` skips idles.

Speech uses the installed Piper worker protocol, PCM16 mono at 24 kHz, and the
normal authenticated gateway upload routes. Two utterances use complete WAV
uploads. The longer reply uses ordered stream chunks and an explicit end marker.
The runtime supplies waveform analysis and generated `speaking_performance`
motion. This exercises the normal playback and animation path; it does not
exercise acoustic wake detection, ASR or agent text generation. The scripts do
not change gains, clips, compiler settings or animation timing.

`trials.jsonl` records clip repetitions, runtime run IDs, starting feedback,
speech duration and terminal results. TTS audio and its worker log stay in the
trial output directory. The runner leaves character mode running after speech;
use the normal rest command and confirm completion before releasing torque.

## Long speech trials

Pass `--speech-only --long-speech` to select a short baseline, complete WAVs
of approximately 60 and 120 seconds, and an ordered stream of approximately
60 seconds. The installed Piper voice determines the exact durations; the
runner rejects audio above the 120-second gateway limit before playback.
This opt-in profile also records speech-clock/status observations in
`trials.jsonl`. The default three-utterance profile remains unchanged.

## Measurement definitions

- Commanded peak: maximum absolute excursion from the trajectory's starting
  command. Measured peak: maximum absolute excursion from the starting measured
  position. Separate starts prevent an initial gravity bias from being counted
  as motion. An idle-to-speech takeover uses the first sample of the new portion
  as its start and is labelled in the JSON report.
- Amplitude ratio: measured peak divided by commanded peak. Category tables
  average peaks across moving runs, then divide those averages. Excursions below
  two encoder counts (about 0.00307 rad) have no meaningful ratio. A ratio below
  2/3 meets P10's greater-than-one-third-loss threshold.
- RMS error: time-weighted RMS of commanded minus measured position, without
  bias removal or lag compensation. Maximum error is the largest absolute error.
- Lag: best correlation shift in a ±500 ms search at 10 ms resolution, using a
  common interior window and executing samples. Positive means feedback follows
  the goal. Correlation removes offset and gain, but lag is approximate for
  nonlinear, quantized or nearly stationary responses. Check correlation and
  search-limit flags in JSON; an unavailable lag is `null`.
- Final error: signed commanded-minus-measured error averaged over the last
  250 ms of terminal settling, only for a completed run. This is a short final
  settling estimate, not a long post-completion equilibrium measurement.
- Command-envelope excess and terminal position standard deviation help
  distinguish bias, overshoot and settling jitter. Peak ratios above one alone
  do not prove overshoot: changing gravity bias can increase measured excursion.

Pass `--trials trials.jsonl` to restrict the report to the requested trials and
exclude setup moves and incidental automatic idles. Omit it to inspect all runs.

The report includes per-run measurements, category aggregates, amplitude-loss
flags, cycle spacing, queue loss, missing samples and settle timeouts. A capture
without a clean summary is explicitly marked incomplete. MuJoCo includes gravity,
joint friction and finite-gain control; it is a useful baseline rather than an
assumption of perfect tracking or a measurement of the real STS3215 electronics.

## Gain experiments

`--experimental-servo-gains JOINT:P:I` selects one shoulder or elbow profile for
a tracked hardware daemon. It leaves the default profile factory, D coefficient,
clips, trajectories and completion thresholds unchanged. The
[runtime reference](../runtime/README.md#experimental-servo-gains) defines its
accepted values and CLI restrictions.

The Python guards reject fast temperature jumps and stop with holding torque;
removing torque while the arm moves can let it fall. Hardware gain trials
require explicit authorization and a person beside the stable robot, ready to
support the arm. Servo-reported rest alone does not establish chassis stability.

P, D and I at addresses 21, 22 and 23 are EEPROM registers in the
[Feetech memory table](https://www.feetechrc.com/Data/feetechrc/upload/file/20240702/舵机协议内存表-磁编码版本.xlsx).
Lock address 55 is SRAM. `apply_servo_profile` reads first, unlocks only for
changed persistent values, writes and verifies those differences, then relocks. EEPROM write
endurance is finite; do not repeatedly alternate gains or reconfigure per trial.
The table does not supply an endurance rating. A P candidate needs one changed
P-byte write and one restoration write if starting and ending at defaults.

The shared `runtime/scripts/experiment_safety.py` guard requires three fresh,
consecutive readings above 55°C. A change greater than 5°C within one second
is logged as a glitch. Rejected values do not replace the plausible reference;
a persistent plateau is reconsidered after one second and still needs three
plausible observations. This heuristic rejects rapid spikes without hiding a
persistent hot sensor indefinitely; it does not independently measure heat.

On a guard trip or runner failure, scripts latch `.experiment-safety-hold.json`
and send `stop`, retaining the driver's last commanded position and torque.
They cancel speech without using `character stop`, which would schedule a
return home. The cleanup wrapper sends SIGSTOP to pause the daemon without
running driver teardown, preventing autonomous motion. The listener stays
stopped; automatic rest, gain restoration and service shutdown are blocked.
A paused daemon supplies no fresh feedback. Keep physical support available
and arrange a separately supervised recovery; do not kill the paused daemon
or resume it merely to clear the latch.

After a successful candidate, request normal `character rest`. Confirm its
completed run, no active motion, torque off and stationary feedback before
stopping the daemon or reading registers. The gain service-stop helper waits
for that state and never sends `disable`; its temporary `TimeoutStopSec=infinity`
prevents systemd from terminating the driver while waiting. Transfer the shared
helper with every experiment harness. Never access the serial bus with a
separate reader while the daemon owns it.

Restore the original daemon only after candidate rest completes. Apply its
default profile while inactive, confirm three fresh stationary rest observations,
then stop safely for a read-only P/D/I and lock check. Restart the original
services and confirm completed rest with torque off again. Preserve both
candidate and restored register captures.

A normal powered home hold uses the daemon's holding state, without repeated
goal writes from a synthetic hold clip. Movement tracking ends at completion,
so archive timestamped status observations during that hold to measure
position reversals, current and temperature. Record observation frequency and
any limit on detecting rapid hunting.

## Speech apex timing

Generated plans emit `speech.motion_compiled` JSON events to daemon stderr.
Each event supplies the exact compiled stroke arrival, its local audio peak,
apex-minus-peak offset, movement run ID and trajectory start on the runtime
clock. Save stderr or the service journal alongside the tracking JSONL.

```bash
python3 runtime/scripts/analyze_speech_apices.py hardware.jsonl daemon.log \
  --json speech-apices.json
```

The analysis compares measured head-pitch maxima of nod strokes with their
planned audio peaks. It excludes plans replaced before their stroke executes,
reflective tilts that need not have a pitch maximum at the stroke, and maxima
on a search-window boundary. Quantized apex plateaus use their midpoint and
retain their width. Positions are measured; the reference is the runtime's
software audio clock at 20 ms resolution. Acoustic/ALSA playback latency is
not independently measured. A compiled waypoint and a measured maximum are
different quantities.
