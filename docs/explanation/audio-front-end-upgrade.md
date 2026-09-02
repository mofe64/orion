# Raspberry Pi audio-front-end upgrade

- **Status:** Planned design
- **Commissioned path:** [Raspberry Pi audio hardware](../../hardware/audio/README.md)

The ReSpeaker mixer enables both physical microphone routes, but Orion's
commissioned Pi listener requests 16 kHz, 16-bit, one-channel pulse-code
modulation (PCM). Wake detection, voice activity detection (VAD), and
transcription therefore do not receive the microphones as separate signals.

The intended upgrade keeps both channels long enough to produce a cleaner mono
signal for the existing voice models:

```text
ReSpeaker left + right microphones
                │
                ▼
       48 kHz stereo capture
                │
                ▼
 calibration, channel health, and spatial combination
                │
                ▼
 noise suppression and optional echo cancellation
                │
                ▼
       16 kHz mono wake/VAD/speech-to-text
```

Wake detection, Silero VAD, and automatic speech recognition (ASR) continue to
consume 16 kHz mono audio. Stereo handling belongs in one front-end component
between Advanced Linux Sound Architecture (ALSA) capture and the voice
controller; individual models must not implement independent downmixing.

## Upgrade phases

1. **Stereo capture and diagnostics.** Capture 48 kHz, 16-bit interleaved
   stereo. Commission channel identity and record per-channel root-mean-square
   (RMS) level, clipping, and noise floor so wiring, obstruction, or microphone
   failure is visible.
2. **Adaptive mono combination.** Calibrate persistent gain mismatch and
   smoothly weight the cleaner channel. This improves obstruction and local
   noise handling without requiring direction estimation.
3. **Noise suppression.** Apply real-time speech denoising after combination
   and before 16 kHz resampling. RNNoise is a candidate because its native
   format is 48 kHz mono; selection still requires measurements.
4. **Spatial combination.** If evidence justifies it, estimate the small
   arrival-time difference, align channels, and apply delay-and-sum
   beamforming. Do not assume an unaligned average is safe.
5. **Playback-reference echo cancellation.** Provide the exact PCM played by
   `oriond` to an acoustic echo canceller alongside the microphone stream.
   Local cues, generated speech, and planned streamed speech need the same
   synchronized reference. WebRTC Audio Processing Module is one candidate.
6. **Digital level control.** Keep codec automatic gain control (AGC) disabled.
   Apply slow digital gain and limiting only after channel alignment and combination; retain the
   commissioned fixed 50 dB PGA baseline.

## Expected benefits and limits

Stereo diagnostics and adaptive combination should help when one side is
obstructed or exposed to more fan or motor noise. Spatial processing may offer
modest off-axis rejection, but two closely spaced microphones do not provide
strong 360-degree localisation. Echo cancellation is the phase most likely to
reduce self-triggering and enable reliable speech interruption.

The HAT contains two analogue microphones and a codec, not a dedicated
far-field DSP. Processing runs on the Pi, and enclosure openings, vibration,
speaker coupling, and microphone placement remain as important as software.

## Validation gate

Compare the commissioned mono path and proposed front end with the same phrase
set at several distances and directions, with fan and motors active, with one
microphone covered, and during Orion speech playback. Track:

- Wake misses and false activations.
- Transcription word error.
- Per-channel clipping and noise floor.
- Processing latency and audio dropouts.
- Pi CPU and memory use.

Promote the stereo path only when it improves the target metrics without
dropouts. Retain a configuration switch to the commissioned mono path until
the upgraded path has equivalent physical evidence.
