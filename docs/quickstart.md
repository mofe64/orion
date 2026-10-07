# Orion quickstart

Orion runs voice and agent processing on its Raspberry Pi. Studio connects for
controls, authoring and settings. Run repository commands from the Orion source
root unless a step specifies another directory.

## Connect Studio

The desktop needs pnpm, a supported Node.js version, Rust and the Tauri
prerequisites listed in [Studio development](../orion_studio/README.md#development).
Install dependencies and open the app:

```bash
pnpm --dir orion_studio install
scripts/studio-dev.sh
```

Select **Pair Orion**, enter the gateway address and token, and save the pairing.
The default gateway address is `http://orion.local:7447`. Settings and profile
changes are sent to the Pi. The Pi services continue running when Studio closes.

## Deploy to the Pi

Choose the lamp hardware explicitly. For V2.1, complete
[V2 calibration and rest capture](hardware-versions.md) and prepare the release before activation.
The existing v1 procedure is:

Commit and push the intended branch, then run from the workstation:

```bash
scripts/deploy_pi.sh --hardware v1 --host mofe@orion.local \
  --root /home/mofe/dev/orion --branch main
```

Use your Pi account, checkout path and branch if they differ. SSH must already
trust the host. The script keeps terminal input available for sudo authentication.
Unattended deployment requires the Pi account's existing sudo policy to allow
service control without prompting.
For a first installation, create the Pi user's Studio pairing token and complete
the [Codex login](#pi-local-voice-and-agent) before deploying. These credentials
are separate from servo calibration. If activation stops before switching
services, fix the reported prerequisite and activate the printed prepared
release directly; preparation does not need repeating.
Use `--prepare-only` to build and test the immutable release while the installed
services continue running. The script prints the release path; activate it later
with that release's `scripts/install_pi_voice_stack.py --release PATH` after the
area around Orion is clear.

The workstation tests and builds Studio. The Pi fetches the chosen commit into
a separate release directory, prepares locked Python environments, runs tests,
and builds the runtime, trajectory compiler and voice host. The old services
remain available during preparation. The installer then checks calibration,
pairing, Codex login and saved settings. It compiles both expressive acknowledgement
motions against a temporary copy of the planned YAML catalog and Pi calibration,
before stopping services or changing live assets.

Pi runtime checks skip native MuJoCo integration tests, which require the
workstation's root simulation environment. All other runtime tests run during
release preparation.

For activation, the installer stops the voice companions, cancels active scene
and speech playback (already-inactive playback is accepted), returns Orion to rest
and confirms torque is off before stopping the runtime. An installed but inactive
runtime is started for this rest sequence; a torque-off robot in another pose is
configured and enabled to reach measured rest.

With the runtime stopped, activation updates built-in YAML and service paths. It
starts the new runtime and keeps voice/Studio companions stopped during the physical
smoke test. The test establishes rest, moves to `zero_reference`, runs
`deployment_smoke` for light/audio, then `acknowledge_left`, `acknowledge_right`
and `return_home`. Each pose and scene waits for measured completion. The final
`character rest` waits for its own movement run to complete, lights to fade off,
and torque to be disabled. Movement and torque come from runtime `status`;
the resting lifecycle and light state come from `character status`. Deployment
prints each smoke pose and scene and includes the observed states if rest times
out. Keep Orion's surroundings clear during activation.
Normal character startup may also move Orion home.

Readiness requires the expected runtime revision, a running coordinator with
Qwen, the selected TTS model and Codex ready, and a gateway connected to that
same voice service. It also requires the runtime to remain in its voice-wakeable
`resting` state with lights and torque off after the smoke test.
A failed activation or smoke test restores the immediately previous configuration,
built-in YAML and service state. A failed mechanical rest leaves the runtime
running for recovery. Complete a spoken turn afterward to check the microphone,
generation and speech playback together.

### Settings preserved by an update

The installer preserves voice preferences, microphone settings, calibration,
pairing, Codex login, personality, memory and user-authored poses, motions and
scenes. Built-in `.yaml`/`.yml` files under `motion/config/`, `motion/motions/`
and `scenes/` update from the selected
commit. Local edits to those built-ins are replaced and backed up for rollback.
Retired tracked built-ins are removed; untracked local files are preserved.
Each hardware version has separate directories. User subdirectories are excluded
from built-in replacement; the installer relocates assets from the former user
paths into their version's directory as part of the rollback transaction.
It changes release paths while retaining installed command arguments
and environment overrides, including sleep and microphone tuning. Existing
service enablement is retained; missing services are enabled. Restarting the agent
service begins a fresh conversation with its saved profile and memory available.

An update converts older saved voice choices to Piper Alba Medium. Turn listening
back on in Studio and complete a spoken turn to check the physical speaker.
Release rollback preserves saved voice settings; an older release may require
its own compatible settings.

The Pi checkout is used to fetch Git objects and supply the asset catalog.
Deployment keeps the checkout's Git HEAD and index unchanged; it updates only
the managed built-in YAML paths in its working tree. Put intended code and YAML
changes in the selected remote commit. See
[installed release boundaries](system-architecture.md#assets-and-installed-releases).

## Pi local voice and agent

The installation targets a Pi 5 with 8 GB RAM and 64-bit Linux. It requires working
[servo calibration](../hardware/servo_setup/README.md),
[audio setup](../hardware/audio/README.md) and
[lighting setup](../hardware/lighting/README.md). Git, Rust/Cargo, uv, Python 3.11
or later, and native build/audio dependencies must be installed. These include
`build-essential`, `pkg-config`, `libssl-dev`, `libdbus-1-dev`, `alsa-utils`,
`python3-venv` and `ca-certificates`.

Calibration and pairing belong to the Pi user under `~/.config/orion/`.
Select the matching calibration and audio setup in the
[hardware-version guide](hardware-versions.md). The ReSpeaker 2-Mics V2 HAT
setup belongs to the v1 lamp; v2 uses USB audio.
If the Pi has no pairing token, create one once and save the displayed value for
Studio:

```bash
python3 orion_studio/gateway.py create-token \
  --token-file ~/.config/orion/studio-token
```

For the first onboard installation, prepare a complete release from committed
source on the Pi:

```bash
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
python3 scripts/deploy_pi_release.py --hardware v1 --source "$PWD" --revision HEAD --prepare-only
```

The command prints the release path. The preparer downloads pinned Qwen GGUF,
Piper Alba Medium, native llama-server, Codex and Silero assets and verifies
their SHA-256 hashes. It synthesizes a short Piper check at 24 kHz before
activation.
Matching assets are reused; a mismatched file stops preparation for inspection.
The speech environment uses Python 3.11 and the listener uses Python 3.12.
Their models and download inventories
live under `~/.local/share/orion/voice-stack/`.

Sign in as the Pi user over SSH:

```bash
~/.local/share/orion/voice-stack/codex-0.157.0/bin/codex login --device-auth
~/.local/share/orion/voice-stack/codex-0.157.0/bin/codex login status
```

Complete the displayed device code in your browser. The account credentials belong
to the Pi user. Run activation as that user; the installer requests sudo for system
files and service control. Replace `/path/to/prepared-release` below with the path
printed during preparation:

```bash
python3 /path/to/prepared-release/scripts/install_pi_voice_stack.py \
  --release /path/to/prepared-release
systemctl is-active oriond orion-studio-gateway orion-listener orion-voice-stack
```

Use `--runtime-project` when the existing catalog root differs from
`~/dev/orion`. Add `--plan` to list the proposed files without activation.
Subsequent workstation updates use `scripts/deploy_pi.sh`. The compatibility entry
point `scripts/install_pi_services.sh ROOT USER HOME v1` also builds a complete release
from committed `HEAD`; the standalone listener installer is reserved for older
listener-only installations.

## Logs and recovery

On the Pi, inspect all four services:

```bash
systemctl status oriond orion-studio-gateway orion-listener orion-voice-stack
journalctl -u oriond -u orion-studio-gateway -u orion-listener \
  -u orion-voice-stack -n 60 --no-pager
```

An active service process does not establish speech readiness. Studio Debug shows
loaded models. **View conversation history** in its Voice card reads saved turns,
including transcript, agent actions and timings, from the Pi. Its gateway log view includes the
runtime, gateway and listener; use `journalctl -u orion-voice-stack` for coordinator,
speech-worker and agent startup diagnostics.

To restore the previous installation, run the installer from the most recently
prepared release:

```bash
python3 /path/to/prepared-release/scripts/install_pi_voice_stack.py --rollback
```

Rollback restores the service files and active/enabled states captured immediately
before the update. Models, login credentials and saved preferences remain available.
A pending deployment journal blocks another update until `--rollback` completes.
Recovery does not force termination when mechanical rest cannot be confirmed.
If activation and recovery both fail, the installer reports both errors and
saves them as `activation_error` and `recovery_error` in the transaction's
`state.json`. Captured runtime command output is included so the configuration
or movement rejection remains available after the daemon exits.

## Retained files

`installation.json` identifies the active release and recent transaction backup.
`pending-installation.json` records an interrupted switch.
`downloads.json` and `model-files.json` inventory shared assets.

Keep the active release, one usable rollback, their Python environments and the
managed interpreters those environments use. The existing catalog root also
remains a runtime dependency. A successful update prunes old transaction snapshots;
it retains release directories so binaries and environments remain available for
recovery.

After successful deployment and a physical voice check, older unused releases,
superseded manual backups, download archives and compiled test outputs can be
removed after checking their references. Build caches are optional; removing them
makes future builds slower. Tests for supported behavior belong in the source
repository and run during validation. The installer extracts a complete
source commit, so test sources also remain in each prepared release.
