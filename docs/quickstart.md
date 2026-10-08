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

You need a Pi 5 with 8 GB RAM and 64-bit Linux, plus Git, Rust/Cargo, uv,
Python 3.11 or later and these packages: `build-essential`, `pkg-config`,
`libssl-dev`, `libdbus-1-dev`, `alsa-utils`, `python3-venv`, `ca-certificates`.

1. **Set up the hardware.** Calibrate the servos and set up audio and lighting.
   For V2 follow [V2 calibration and centring](hardware-versions.md); for V1 use
   the [servo](../hardware/servo_setup/README.md),
   [audio](../hardware/audio/README.md) and
   [lighting](../hardware/lighting/README.md) guides.
2. **Create the Studio pairing token** and keep the printed value for Studio:

   ```bash
   python3 orion_studio/gateway.py create-token --token-file ~/.config/orion/studio-token
   ```

3. **Prepare the first release** on the Pi from committed source:

   ```bash
   export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
   python3 scripts/deploy_pi_release.py --hardware v2 --source "$PWD" --revision HEAD --prepare-only
   ```

   Use `--hardware v1` for a V1 lamp. Preparation downloads the pinned Qwen,
   Piper, llama-server, Codex and Silero assets, checks their SHA-256 hashes,
   and prints the release path. Models live under
   `~/.local/share/orion/voice-stack/`.
4. **Sign in to Codex** as the Pi user and complete the device code in a browser:

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

## Connect Studio

Studio needs pnpm, Node.js, Rust and the Tauri prerequisites in
[Studio development](../orion_studio/README.md#development):

```bash
pnpm --dir orion_studio install
scripts/studio-dev.sh
```

Choose **Pair Orion** and enter the gateway address, for example
`http://ariadne-robot.local:7447`, and the token from [Prepare a new Pi](#prepare-a-new-pi) (print it on the Pi
with `cat ~/.config/orion/studio-token`). Studio stores one pairing; to switch
lamps, choose **Forget Orion on this computer** first.

## Deploy to the Pi

Commit and push your changes, then run from your computer:

```bash
scripts/deploy_pi.sh --hardware v2 --host mofe@ariadne-robot.local \
  --root /home/mofe/orion --branch main
```

- Use `--hardware v1` for V1. Without flags the script uses `mofe@orion.local`
  and `/home/mofe/dev/orion`.
- Stop any `oriond` you started by hand first; it holds the servo port and the
  runtime socket.
- SSH must already trust the Pi. The terminal stays open for the sudo password.
- Keep the area around the lamp clear: the deploy moves the arm.
- `--prepare-only` builds and tests the release without switching services.
  Activate it later with that release's `scripts/install_pi_voice_stack.py --release PATH`.

What the deploy does:

1. Tests and builds Studio on your computer.
2. On the Pi, extracts the pushed commit into a new release folder, prepares
   its Python environments, runs the tests and builds the binaries. The running
   services are untouched until this succeeds. Native MuJoCo tests are skipped
   on the Pi.
3. Checks calibration, pairing, Codex login and settings, and compiles the
   smoke-test motions against the Pi's calibration.
4. Stops the voice services, moves the arm to rest and confirms torque is off,
   then stops the runtime.
5. Updates service files and built-in YAML, starts the new runtime and runs the
   physical smoke test: rest, `home` (V2) or `zero_reference` (V1), the
   `deployment_smoke` light and audio check, `acknowledge_left`,
   `acknowledge_right`, `return_home`, then rest with lights and torque off. Each step waits for measured completion.
6. Starts the voice and Studio services and waits up to three minutes for them
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
- `releases/` holds every prepared release. Keep the active one and the
  rollback one; older releases can be deleted after a successful deploy and
  voice check. Build caches are optional and only speed up later builds.
