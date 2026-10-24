# Change the Hey Orion wake model

The listener uses openWakeWord for continuous wake detection. Qwen checks one prefix before the command reaches the agent.
The prefix contains three seconds before detection and 300 ms after detection. An accepted prefix remains accepted for the complete command recording.

The detector configuration lives in `voice/models/verifier/config.json`. It defines the threshold, startup frames and model file hashes.

## Record your voice

The recording tool runs on the Pi. It stops the listener during capture and starts it again on exit.
Recordings stay on the lamp. The tool does not upload them.

1. Get consent from the person before recording.
2. Run the recorder from a Pi terminal.

   This command records the prompted sets for one person.

   ```bash
   orion-record-voice --speaker NAME
   ```

3. Use only letters and digits in `NAME`.
4. Press Enter to start each take.
5. Speak when the screen shows `SPEAK NOW`.
6. Press Enter to keep the take.
7. Enter `r` to record the take again.
8. Enter `q` to save progress and exit.

The same command continues an incomplete set on the same day. Files use `~/.local/share/orion/wake-samples/NAME/YYYYMMDD/`.
The manifest stores prompts, recording levels and take choices. The tool keeps previous takes.

This command records 30 minutes of background sound.

```bash
orion-record-voice --speaker room --background 30
```

Do not say the wake phrase during background capture. Use `--out DIR` to select another directory.

## Prepare training files

1. Obtain consent from each recorded speaker.
2. Keep private recordings outside Git.
3. Prepare mono 16 kHz WAV inputs.
4. Include the complete wake phrase in positive clips.
5. Include ordinary speech, similar phrases and household noise in negative clips.
6. Separate training and comparison inputs by speaker and recording session.

## Select a model

1. Prepare the openWakeWord feature models and phrase classifier in `voice/models/verifier/`.
2. Put each model file hash in `config.json`.
3. Set the threshold and startup frames in `config.json`.
4. Keep the model files and configuration together.
5. Use the [release procedure](release-packaging.md) to install them.

`--wake-engine rustpotter` selects the previous detector. Its model uses `--wake-model` and `--threshold`.
The [configuration reference](configuration.md#microphone-listener) defines these options.

`ORION_WAKE_CLIPS_DIR` saves accepted and rejected prefixes for inspection. The [listener reference](../voice/README.md#troubleshooting) defines storage limits.

The acoustic classifier uses openWakeWord features with a CC BY-NC-SA license. Preserve model and feature licenses when distributing replacements.
