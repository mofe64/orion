# Orion Studio

Studio edits scenes and controls a connected Orion. The desktop application uses Tauri and React. Its Pi gateway forwards authenticated requests to local services.

## Development

Install Rust, Node.js, pnpm and the Tauri system prerequisites for your operating system. `package.json` specifies the pnpm version. Run these steps from the repository root.

1. Install desktop dependencies.

   ```bash
   pnpm --dir orion_studio install --frozen-lockfile
   ```

2. Run interface tests.

   ```bash
   pnpm --dir orion_studio test
   ```

3. Build the interface.

   ```bash
   pnpm --dir orion_studio build
   ```

4. Start the desktop application.

   ```bash
   scripts/studio-dev.sh
   ```

## Connect Studio to the Pi

1. Install Orion with the [quickstart](../docs/quickstart.md).
2. Select **Pair Orion** in Studio.
3. Enter the Pi gateway address.
4. Request a spoken code.
5. Enter the code.
6. Select **Pair**.

The default gateway port is 7447. **Use a token instead** accepts the Pi's Studio token. The [configuration reference](../docs/configuration.md#studio-gateway) defines credentials and saved pairing.

### Hostname discovery

A `.local` hostname uses multicast DNS on the local network. The Pi and computer must share a network that permits this traffic. An IPv4 address can replace the hostname.

1. Check gateway connectivity without sending a token.

   ```bash
   curl --noproxy '*' --max-time 5 -i http://HOST.local:7447/api/v2/status
   ```

An HTTP 401 response means authentication is required. It also shows that the gateway answered the request.

### Gateway source development

The gateway defaults to loopback access. `--bind ::` accepts IPv4 and IPv6 when the operating system supports a dual-stack socket.

An installed gateway reads poses and scenes from the asset catalog. It finds `orion-trajectory` under its own release folder. `--trajectory-compiler` selects a different compiler.

1. Stop the installed gateway before using its port.

   ```bash
   sudo systemctl stop orion-studio-gateway
   ```

2. Build the trajectory compiler required by the gateway.

   ```bash
   cargo build --release --manifest-path runtime/Cargo.toml --locked
   ```

3. Run the source gateway with the V2 catalogue and compiled trajectory program.

   ```bash
   uv run --project orion_studio --locked --managed-python python orion_studio/gateway.py serve      --bind :: --port 7447 --socket /tmp/oriond.sock      --token-file ~/.config/orion/studio-token --project-root "$PWD"      --hardware v2 --calibration "$PWD/hardware/profiles/v2-joints.json"      --trajectory-compiler "$PWD/runtime/target/release/orion-trajectory"
   ```

4. Run the gateway tests in its managed environment.

   ```bash
   uv run --project orion_studio --locked --managed-python python -m unittest discover -s orion_studio/tests -v
   ```

## Orion

Orion shows mode, listening, lamp controls, expressions and active alerts. Disconnected Studio keeps local scene editing available.

**Idle mode** permits automatic rest. **Lamp mode** keeps Orion awake. **Pause mode** stops automatic movement without changing the selected mode. **Go to rest** starts the runtime's rest sequence.

The V2 tracking selector offers Off, Idle, Faces and Hands. This setting differs from the lamp's Idle/Lamp mode. Camera privacy and microphone mute remain independent.

Lamp controls apply released slider values. A scene, speech or rest can override visible output. The home model is a pose preview; it does not display live joint feedback.

### Camera feed

Orion and Debug can show the V2 camera feed. **Turn camera off** changes the Pi privacy setting for every viewer. **Save snapshot** saves one image on the Studio computer only after selection.

The gateway sends the bearer token in an Authorization header. It does not put credentials in image URLs. Detection boxes require exact capture timestamp matches and expire after 300 ms. Leaving the view ends its stream.

The runtime shows a cyan ring band while viewing permission remains active. Rest blocks camera viewing. See the [runtime indicator](../runtime/README.md#camera-viewer-indicator).

## Animation library and scene editor

A pose contains five joint positions. A motion describes travel between positions. A scene combines motion, light and audio. Built-in assets remain read-only.

Local drafts save on the Studio computer. **Publish to Orion** sends a scene and its owned poses and motions to the Pi. Publication requires completed trajectory compilation.

1. Select **Create scene**.
2. Choose a scene copy or starting pose.
3. Edit the Movement, Light and Sound tracks.
4. Select **Preview** to inspect the local model.
5. Select **Publish to Orion** to save the scene on the Pi.

WARNING: Hardware playback moves powered joints. Keep the movement area clear before playback.

6. Select **Play on Orion** to run the published scene.

**Preview from** changes the model's starting pose. Physical playback starts at Orion's measured position. **Split into poses** exposes motion components for editing.

Light and sound events use elapsed time or motion markers. **Edit pose** creates a pose owned by the scene. Standalone pose and motion creation is outside this editor's workflow.

Deleting published user scenes checks the loaded revision. It removes scene-owned dependencies. A rejected runtime reload restores previous files. See [scene schema](../scenes/README.md).

## Voice observation

Studio reads voice state, transcripts and timing through the gateway. The Pi owns recognition, agent requests, synthesis and playback.

History shows saved conversations and behavior events. Camera images and microphone audio do not enter those records. See [conversation storage](../history/README.md).

## Settings and Debug

Settings controls personality, memory, character behaviors, location, voice and desktop appearance. Most choices save automatically. Voice settings use **Save** and restart the voice coordinator.

Memory and personality changes refresh agent context on the next request. Removing local memory does not erase information already sent to Codex.

Debug shows service errors, camera controls and technical details. The desktop app requires a connected Pi for remote status and saved history.

## System readings

Open **System** to see processor use, temperature, memory, storage, Orion's parts and the busiest programs.
The gateway samples Linux counters every two seconds. It keeps 15 minutes of readings in memory.
The page pauses requests when hidden. Warnings show low power, slowdowns, temperatures above 75 °C and storage above 90% use.
Unavailable counters show no value. CPU percentages for services and processes can exceed 100% across multiple cores.

## Home

The **Orion** tab contains Orion's own controls. The **Home** tab contains household devices and setup.
Follow the [Home setup procedure](../docs/quickstart.md#home-assistant) to install Home Assistant and create its owner account.

Home groups device entities by room. Simple devices have on and off controls.
**Orion can control** changes the Home Assistant assistant exposure setting for that entity.
**Discovered** lists devices that need setup. **Add device** searches available integrations.
Studio displays supported Home Assistant configuration forms. Unsupported forms provide **Continue in Home Assistant**.
**Open Home Assistant** opens the full interface on port 8123.
