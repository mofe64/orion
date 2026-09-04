# Orion Pi voice

`orion-listener` owns ReSpeaker stereo capture, Rustpotter, mono speech
endpointing and a token-authenticated WebSocket audio connection to Studio. Qwen ASR,
agent processing and Chatterbox run in Studio. The reference model and native
Rustpotter adapter are installed on the Pi only.

## Install on the Pi

Use the commissioned Pi 5 ReSpeaker V2 setup in
[Audio hardware](../hardware/audio/README.md). ALSA tools are installed by the deployment workflow. From the workstation
repository root, use the existing source deployment command:

```bash
scripts/deploy_pi.sh
```

On the commissioned Pi this installs missing ALSA/build dependencies and a pinned `uv`
bootstrap, retires this checkout's legacy voice services/processes, archives
known Sherpa/Moonshine/Silero/Piper downloads and the old voice environment,
then installs the locked Rustpotter stack with Python 3.11. It runs the Pi voice
tests, loads the actual reference through the native adapter, installs/restarts
the listener and checks that invalid authentication is rejected without opening
capture. It preserves pairing, calibration, microphone geometry and unrelated
files. A failed voice install restores the previous environment and leaves the
listener stopped; archived models remain in the backup for manual recovery.

When upgrading from a checkout with a locally generated, untracked
`voice/uv.lock`, deployment removes that file before the fast-forward merge
only if the incoming revision supplies its replacement. Tracked local edits,
symlinks and unrelated files remain protected by Git's normal merge checks.

The deployment command still requires an existing source-built, hardware-
commissioned Pi, its Rust toolchain, trusted SSH and permission to use sudo for
package installation and service management. Deployment opens an SSH terminal;
enter the Pi user's sudo password there when prompted. Passwordless sudo is
needed only for unattended runs. Credentials are never passed in script
arguments or captured by the installer. It runs
physical motion smoke tests; it does not image a blank SD card or install Mac
inference models on the Pi. For voice-only repair on the Pi checkout:

```bash
scripts/install_pi_voice.sh "$PWD"
scripts/install_pi_services.sh "$PWD" "$(id -un)" "$HOME"
sudo systemctl restart orion-listener.service
```

The voice repair command makes the same environment/model backup under
`~/.local/share/orion/backups/voice-*`. The optional legacy source remains for
explicit diagnostics but its dependencies and models are absent from the
active primary environment.

The source environment is editable: the native adapter is built for the Pi's
architecture and uses `models/wake/hey_orion_reference.rpw`. The default wake
threshold is 0.400. No ASR or speech-synthesis weights are required on the Pi
for this path. Wake accuracy and CPU usage still require Pi measurement.

## Configure the LAN connection

The listener uses `~/.config/orion/studio-token`, the same existing token used
by the Studio gateway. Create that token with the gateway's `create-token`
command if the Pi has not been paired. Paste it into Studio's connection panel;
Voice reuses that connection token automatically. Desktop Studio saves the
pairing in the operating system credential store and reconnects at launch or
after network loss. **Disconnect** pauses reconnect for this session;
**Forget Orion** deletes the saved pairing on this computer. A rejected token
requires pairing again. Connecting does not enable microphone capture or start
movement. Do not put tokens in URLs.

Studio connects to `ws://GATEWAY_HOST:7448/`. No certificates or TLS settings
are required. The token and audio travel unencrypted; use a trusted local
network. Authentication controls access but does not encrypt the connection.

Install the service templates from the repository root:

```bash
scripts/install_pi_services.sh "$PWD" "$(id -un)" "$HOME"
sudo systemctl enable orion-listener.service
sudo systemctl restart orion-listener.service
journalctl -u orion-listener.service -n 50
```

The service listens on TCP 7448. It opens the microphone only after Studio
connects and authenticates. Studio's **Stop Orion microphone** closes capture.
Character Stop is a separate control and does not mute the microphone.

For a foreground diagnostic run, first stop the service, then run:

```bash
sudo systemctl stop orion-listener.service
voice/.venv/bin/orion-listener --host 0.0.0.0 \
  --token-file ~/.config/orion/studio-token
```

Only one process may own capture. No legacy listener should be running.

## Checks after deploying to the Pi

1. **Confirm deployment completed.** The deployment command installs the voice
   environment, retires the previous stack and verifies Rustpotter automatically.
   Do not run a second manual installation after a successful deployment.
2. **Check the services.** From the Pi repository root, run:

   ```bash
   systemctl is-active oriond.service orion-studio-gateway.service orion-listener.service
   journalctl -u orion-listener.service -n 50 --no-pager
   ```

   All three services should report `active`. The listener waits without
   opening the microphone until Studio authenticates. An old unit containing
   `--cert` or `--key` needs reinstalling. Existing certificate files can stay
   on disk; the listener no longer reads them.
3. **Prepare Studio.** Follow the [Studio Voice tutorial](../docs/tutorials/first-studio-voice-run.md)
   to sync the processing worker, prefetch Qwen/Chatterbox weights and check
   Codex sign-in. Remove an old `ORION_PI_VOICE_CA` launch setting and unset
   any `wss://` voice URL override. Connect to the existing gateway using its
   token once using **Pair and remember Orion**, then select **Enable Orion microphone**.
4. **Test speech before direction.** Leave microphone geometry at zero. Try
   “Hey Orion, what time is it?” and “Hey Orion”, pause, then a command.
   Expect Pi wake detection, Qwen confirmation, a transcript, an agent reply
   and playback through Orion. Check that rejected wake candidates do not
   reach the agent, playback completes before listening rearms, and
   **Stop Orion microphone** closes capture. Disconnect/reconnect Studio and
   verify that an old session is not replayed.
5. **Check character startup separately.** The deployment smoke test finishes
   at rest with torque disabled. Start Character from Studio for the first
   check. With Orion clear to move and an operator present, restart `oriond`
   and verify that it reaches home and enables character mode. Studio
   Character Stop should keep character mode off until it is started again
   or the daemon restarts. Character Stop does not mute Voice or release torque.
6. **Configure direction last.** Complete the stereo and physical checks below,
   then test left/right attention, interruption and return to the prior anchor.
   Record wake accuracy, false positives and response latency in the actual room;
   local tests do not establish Pi inference performance or physical quality.

## Commission left and right attention

Direction estimates require independent stereo channels and a microphone
baseline aligned with the robot's left/right axis. The default spacing and
channel sign are zero, which disables directional attention while leaving
voice operational. Do not infer channel orientation from the board label.

1. Record stereo samples with `hardware/audio/configure-capture.sh` applied.
2. Verify independent left/right channels, no clipping, and fixed matched gain.
3. Measure the distance between microphone acoustic centres in metres.
4. Evaluate known left, centre and right speech in the assembled enclosure.
5. Set the sign so negative estimates mean robot-left, positive mean robot-right.

Store the measured settings in `~/.config/orion/voice.env`:

```text
ORION_MIC_SPACING=MEASURED_DISTANCE_IN_METRES
ORION_CHANNEL_SIGN=COMMISSIONED_SIGN
```

Replace both placeholders: sign must be `1` or `-1`, and spacing must be a
positive measured value no greater than 0.3. Restart the listener after editing.
The estimator needs at least five consistent speech observations and confidence
at least 0.75. It rejects ambiguous peaks and silence; its confidence is an
agreement score, not a calibrated probability. Two microphones do not provide
unambiguous front/back or full 3D location.

Follow the [attention brief](../docs/explanation/voice-attention.md) and
[physical acceptance procedure](../docs/how-to/validate-character-v2.md) before
accepting autonomous turns. Speech, room reflections and motor noise require
real-hardware evaluation; unit tests establish only software properties.

## Legacy offline tools

The optional diagnostic stack is installed explicitly with:

```bash
cd voice
uv sync --python 3.11 --extra fallback
./install-models.sh
```

It provides `orion-voice listen-worker` (Sherpa/Silero/Moonshine),
`orion-voice wake-worker`, transcript socket diagnostics and
`orion-voice tts-worker` (Piper). These are not part of the primary Rustpotter
service. Stop `orion-listener` before using a legacy microphone worker.
