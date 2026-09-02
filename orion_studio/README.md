# Orion Character Studio v2

Studio is a dark, accessible creative workspace for everyday Orion owners and
motion builders. It authors pose, motion, and scene v2 assets while the Pi
remains the only hardware authority.

```text
Studio / Chatterbox ── authenticated HTTP v2 ──> Pi gateway
                                                   │ private Unix socket
                                                   v
                                                oriond
                                   motion + character + light + sound
```

Editing is inert. A slider, keyframe, or timeline drag never moves Orion.
Explicit **Run on Orion** and **Publish v2 asset** actions cross the gateway.

## Current editor model

- **Pose** — all five joints, semantic tags, idle profile, default lighting,
  and live calibrated ranges while connected.
- **Motion** — absolute or anchor-relative space, named character style,
  through/settle arrivals, legal holds, partial offsets, and named markers.
- **Scene** — parallel motion, lighting, and audio tracks with seconds or
  marker triggers and an explicit finish policy.
- **Preview** — samples the Rust compiler's exact calibrated 50 Hz spline,
  displays retimed markers and peak speed, and can evaluate relative clips
  around any powered anchor.
- **Character** — explicit start/stop plus listening and thinking reactions;
  status exposes state, immutable anchor, active clip, and next idle category.
- **Diagnostics** — live calibration, 7.4 V STS3215 profile, runtime mode,
  torque state, active character state, and deterministic preview seed.

The interface targets WCAG AA: visible keyboard focus, semantic controls,
44 px primary targets, reduced-motion support, responsive editing layouts, and
navy surfaces with restrained blue, cyan, and violet status/accent color.

## Development

Use Node.js 20 or newer, pnpm, stable Rust, and the Tauri 2 prerequisites for
your platform.

```bash
cd orion_studio
pnpm install
pnpm test
pnpm build
pnpm tauri dev
```

`pnpm dev` runs the UI-only frontend on `http://localhost:1420`. Voice worker
startup and other native commands require Tauri. macOS, Windows, and Linux
packages must be built and signed on their respective target platforms.

## Connect Studio to the Pi

The Pi runs `oriond.service` and `orion-studio-gateway.service`. Create the
private pairing token once:

```bash
python3 orion_studio/gateway.py create-token \
  --token-file ~/.config/orion/studio-token
```

For source development, start the gateway with the Pi calibration and installed
trajectory compiler:

```bash
python3 orion_studio/gateway.py serve \
  --bind 0.0.0.0 --port 7447 \
  --socket /tmp/oriond.sock \
  --token-file ~/.config/orion/studio-token \
  --project-root /home/mofe/dev/orion \
  --calibration ~/.config/orion/servo_calibration.json \
  --trajectory-compiler /home/mofe/dev/orion/runtime/target/release/orion-trajectory
```

Connect Studio to `http://orion.local:7447` and paste the token. The URL is
stored locally; the token stays in session storage. The API accepts semantic
v2 operations only and never exposes arbitrary paths, registers, or joint
streams.

## Studio Voice playback

The workstation still performs wake detection, Qwen transcription, agent
response, and Chatterbox Turbo synthesis. Playback is Pi-owned:

```text
Chatterbox PCM16
  -> exact mono 24 kHz RIFF/WAV
  -> authenticated POST /api/v2/speech
  -> random Pi spool identifier
  -> oriond/ReSpeaker playback
  -> energy-driven speaking motion + warm RGBW light
  -> terminal status
  -> Studio playback acknowledgement
```

Studio polls the run through queued, playing, and terminal states and reports
completion to the voice worker only after Pi playback completes. Cancellation
is run-scoped. The runtime deletes spool files after completion, cancellation,
or failure. Pi-local Piper uses the same speech coordinator, so it receives the
same motion and lighting behavior.

Prepare the optional Apple Silicon voice models separately:

```bash
cd orion_studio/voice_worker
uv sync --python 3.12
.venv/bin/orion-voice-models
```

The agent receives confirmed text only. Agent-generated prose cannot issue raw
robot commands; Studio maps known voice phases deterministically to listening,
thinking, and neutral character states.

## Atomic deployment

`scripts/deploy_pi.sh` validates this Studio build before updating the Pi. The
remote phase returns the running robot to mechanical rest, releases torque,
fast-forwards the selected branch, validates the user asset catalog, builds
runtime and trajectory binaries, installs both services, and runs light/audio
plus left/right expressive physical smoke tests. All components therefore come
from one Git revision.

See the [system architecture](../docs/explanation/system-architecture.md),
[motion architecture](../docs/explanation/motion-and-animation-architecture.md),
and [scene reference](../scenes/README.md).
