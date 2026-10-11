# Orion quickstart

Orion has two halves:

- **The Pi in the lamp** runs everything the lamp needs: the servo runtime
  (`oriond`), wake word and speech recognition, the Codex agent and speech
  synthesis. It keeps working when no computer is connected.
- **Studio on your computer** pairs with the Pi to author scenes, control the
  lamp and change voice settings.

Run repository commands from the Orion source root unless a step says otherwise.

## Prepare a new Pi

Skip this section when the Pi already runs Orion.

You need a Pi 5 with 8 GB RAM running Ubuntu 24.04 ARM64, system Python 3.12,
Git, uv, an installed Python 3.11, and these runtime packages:
`libssl3t64`, `libdbus-1-3`, `libusb-1.0-0`, `alsa-utils`, `python3-venv`,
`ca-certificates`. Install the speech interpreter once with
`uv python install --no-cache 3.11`. Routine deployment requires no Rust compiler.
Camera setup provisions native libraries separately.

1. **Set up the hardware.** Calibrate the servos and set up audio and lighting.
   For V2 follow [V2 calibration and centring](hardware-versions.md); for V1 use
   the [servo](../hardware/servo_setup/README.md),
   [audio](../hardware/audio/README.md) and
   [lighting](../hardware/lighting/README.md) guides.
2. **Create the Studio gateway token.** Studio obtains it through
   [spoken-code pairing](configuration.md#studio-gateway); the printed value is
   available for manual entry:

   ```bash
   python3 orion_studio/gateway.py create-token --token-file ~/.config/orion/studio-token
   ```

3. **Prepare the first release** from your computer. Commit and push the intended
   source first, start Docker, and run:

   ```bash
   scripts/deploy_pi.sh --hardware v2 --prepare-only
   ```

   Use `--hardware v1` for a V1 lamp. The computer checks the Pi's OS, builds and
   tests the pushed commit in Ubuntu 24.04 ARM64, and transfers its binaries and
   prebuilt Python packages. The Pi installs those packages without compilation
   or persistent package caches. Preparation downloads missing pinned Qwen,
   Piper, llama-server, Codex and Silero assets, checks their SHA-256 hashes,
   and prints the prepared release path. Models live under
   `~/.local/share/orion/voice-stack/`.
4. **On the Pi, sign in to Codex** as the Pi user and complete the device code in a browser:

   ```bash
   ~/.local/share/orion/voice-stack/codex-0.157.0/bin/codex login --device-auth
   ```

5. **Activate the release**, replacing the path with the one printed in step 3:

   ```bash
   python3 /path/to/prepared-release/scripts/install_pi_voice_stack.py \
     --release /path/to/prepared-release --runtime-project "$PWD"
   systemctl is-active oriond orion-studio-gateway orion-listener orion-voice-stack
   ```

   All four services should report `active`. Add `--plan` to list the files the
   installer would change without activating anything.

Later updates use the [deploy script](#deploy-to-the-pi).

For access away from configured Wi-Fi, install the optional
[fallback hotspot](networking.md) after verifying which network manager owns
the Pi's Wi-Fi interface.

## Connect Studio

Studio needs pnpm, Node.js, Rust and the Tauri prerequisites in
[Studio development](../orion_studio/README.md#development):

```bash
pnpm --dir orion_studio install
scripts/studio-dev.sh
```

Choose **Pair Orion**, enter the gateway address, for example
`http://ariadne-robot.local:7447`, and click **Get code**. Listen to the lamp,
type its spoken code, then click **Pair**. Studio verifies the connection and
saves the returned token securely on this computer.

**Say it again** asks for a replacement code. See [Studio gateway](configuration.md#studio-gateway)
for expiry, attempt limits and the gateway journal fallback. **Use a token instead**
opens manual entry for the token created during Pi setup.

Use **Pair another lamp** to add a lamp, then switch between saved lamps in the
connection panel. **Change address** verifies an address with the saved token.

## Deploy to the Pi

Commit and push your changes, then run from your computer:

```bash
scripts/deploy_pi.sh --hardware v2 --host mofe@ariadne-robot.local \
  --root /home/mofe/orion --branch main
```

- Use `--hardware v1` for V1. Without flags the script uses
  `mofe@ariadne-robot.local` and `/home/mofe/orion`.
- The computer needs a running Docker engine, Python 3.11 or later, Node.js
  22.12 or later and pnpm. Docker runs the Linux build environment locally;
  the Pi runs ordinary systemd services.
- The computer builds what is pushed. Studio checks, Linux builds, tests and
  deployment helpers all come from the same selected commit. Uncommitted
  changes are excluded. Commit and push the pipeline changes before first use.
- The deploy refuses to start while an `oriond` you started by hand is running,
  because it holds the servo port and the runtime socket. It also refuses a
  calibration that crosses raw 0/4095 or uses the wrong servo IDs.
- Installation output is saved on the Pi in
  `~/.local/share/orion/voice-stack/logs/`.
- SSH must already trust the Pi. The terminal stays open for the sudo password.
- Keep the area around the lamp clear: the deploy moves the arm.
- `--prepare-only` builds, tests, transfers and installs the release without switching services.
  Activate it later with that release's `scripts/install_pi_voice_stack.py --release PATH`.

What the deploy does:

1. Reads the Pi's OS, architecture, glibc and system Python version. It requires
   Ubuntu 24.04 ARM64, glibc 2.39 and system Python 3.12, matching the commissioned
   robot. A mismatch stops before building or installing.
2. Tests and builds Studio from the selected commit in a temporary local folder.
   Builds production Rust binaries and Python wheels in a local Linux Docker
   environment. Runs software tests there; native MuJoCo tests remain excluded
   from the hardware-release build. Compiler and package caches stay on the computer.
3. Transfers a checksummed release bundle. On the Pi, checks platform compatibility
   again and checks free space for unpacking, package installation and reserved
   headroom. Creates Python environments at their permanent paths and installs
   only the supplied wheels, with source builds, registry access and package caching
   disabled. Reuses shared model/native-tool assets, verifies the camera imports
   and voice models, and deletes temporary installation files. Running services
   stay active throughout preparation.
4. Checks calibration, pairing, Codex login and settings, and compiles the
   smoke-test motions against the Pi's calibration.
5. Stops the voice services, moves the arm to rest and confirms torque is off,
   then stops the runtime.
6. Updates service files and built-in YAML, starts the new runtime and runs the
   physical smoke test: rest, `home` (V2) or `zero_reference` (V1), the
   `deployment_smoke` light and audio check, `acknowledge_left`,
   `acknowledge_right`, `return_home`, then rest with lights and torque off. Each step waits for measured completion.
7. Starts the voice and Studio services and waits up to three minutes for them
   to report ready.

If anything fails, the installer restores the previous release automatically.
When it cannot confirm the arm is safely at rest, it stops instead and asks you
to run `--rollback` (see below). After a successful deploy, say "Hey Orion" and
complete one spoken turn to check the microphone, agent and speaker together.

### What an update keeps

Voice preferences, microphone settings, calibration, pairing, Codex login,
personality, memory, and user-authored poses, motions and scenes are kept.
Built-in YAML under `motion/config/`, `motion/motions/` and `scenes/` is
replaced with the commit's version; local edits to built-ins are backed up for
rollback. Service command-line tuning and environment overrides are kept.
The Pi checkout's Git `HEAD` and index are not changed. See
[installed release boundaries](system-architecture.md#assets-and-installed-releases).

## Logs and recovery

On the Pi:

```bash
systemctl status oriond orion-studio-gateway orion-listener orion-voice-stack
journalctl -u oriond -u orion-studio-gateway -u orion-listener -u orion-voice-stack -n 60 --no-pager
```

A running service is not proof that speech is ready. Studio's Debug page shows
loaded models, and **View conversation history** shows saved turns with
transcripts, agent actions and timings. For coordinator, speech-worker and
agent start-up problems, read `journalctl -u orion-voice-stack`.

To restore the previous installation:

```bash
python3 /path/to/prepared-release/scripts/install_pi_voice_stack.py --rollback
```

Rollback restores the service files and service states from just before the
update; models, login and preferences stay. An interrupted deploy blocks the
next one until `--rollback` finishes. When both activation and recovery fail,
the installer prints both errors and saves them as `activation_error` and
`recovery_error` in the transaction's `state.json`.

## Files the installer keeps

In `~/.local/share/orion/voice-stack/`:

- `installation.json` names the active release and its rollback snapshot.
- `pending-installation.json` exists only while a deploy is unfinished.
- `downloads.json` and `model-files.json` list shared downloaded assets.
- `releases/` holds prepared releases. A successful deploy keeps the active
  release and the one rollback would restore, and deletes older ones. A deploy
  checks the incoming bundle's storage needs plus 512 MiB of reserved headroom.
- `logs/` holds the output of each deploy.

Deployment creates no compiler cache or persistent Python package cache on the Pi.
After a successful migration, remove an old compiler cache while holding the
deployment lock:

```bash
flock -n ~/.local/share/orion/voice-stack/deployment.lock \
  rm -rf -- ~/.local/share/orion/voice-stack/build
```

Inspect `~/.cache` before removing legacy package caches; uv-managed Python
interpreters under `~/.local/share/uv/python` are runtime dependencies to retain.
Local Docker build caches live in the `orion-pi-build-cache` volume.
The builder clears that volume's disposable contents when they exceed 6 GiB,
before and after builds. Docker images and the workstation's pnpm store are separate.
