# Orion known limitations

## Hardware and safety

- Software torque disable is not an emergency stop. Physical trials require an
  accessible hardware power or torque interruption.
- The source-backed Pi services assume a trusted development checkout and
  commissioned calibration under the Pi user's configuration directory.
- ReSpeaker capture is 16 kHz mono with fixed gain and no commissioned echo
  cancellation, beamforming, or noise suppression.
- The RGBW path has no 3.3 V-to-5 V level shifter. The runtime fails closed if
  the output device cannot be opened, but electrical margin remains a hardware
  limitation.

## Studio and networking

- The gateway bearer token and HTTP transport are suitable only for a trusted
  development LAN. Traffic is not encrypted.
- Studio's implemented local speech-recognition and speech-synthesis adapters
  require Apple Silicon because they use Apple's MLX machine-learning framework.
- Signed desktop installers omit the Python worker, native wake
  extension, or large model weights.
- Offline desktop user assets are staging copies. The Pi library becomes
  authoritative after connection.

## Voice and agents

- Voice weights require a separate first-device download and several gigabytes
  of cache space.
- Enabling Voice before prefetching may trigger model downloads during startup
  and exceed the worker connection timeout.
- The initial endpoint detector still needs evidence-based tuning for varied
  rooms, microphones, and speaking styles.
- The Codex provider sends the confirmed text command to a cloud service. Raw
  microphone audio remains local.
- Agent output can generate speech only. It has no supported path to movement,
  lights, cues, or scenes.
- The Pi-local fallback publishes transcripts but does not interpret them.

## State and history

- Runtime run IDs and the retained last result reset when `oriond` restarts.
- The runtime is not an audit log or durable movement database.
- Voice audio and pre-roll are intentionally transient and are not recorded by
  the runtime.
