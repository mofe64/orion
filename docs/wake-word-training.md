# Change the Hey Orion wake model

The listener uses a Rustpotter candidate model and a Qwen check of the wake prefix. An optional acoustic phrase verifier can replace the prefix check. Qwen checks complete command recordings before agent dispatch. Changing one stage does not replace the others.

The installed model selection lives in `scripts/pi_service_config.py`. The verifier decision rule lives in `voice/models/verifier/config.json`.

## Prepare training audio

1. Obtain consent from each recorded speaker.
2. Keep private recordings outside Git.
3. Prepare mono 16 kHz WAV inputs.
4. Include the complete wake phrase in positive clips.
5. Include ordinary speech, similar phrases and household noise in negative clips.
6. Separate training and comparison inputs by speaker and recording session.

Rustpotter positive file names use labels such as `[hey_orion]001.wav`. Unlabelled file names represent the `none` class.

## Train and select a model

1. Follow the upstream [Rustpotter CLI guide](https://github.com/GiviMAD/rustpotter-cli#creating-a-wakeword-model).
2. Compare candidate behavior with unchanged input audio.
3. Select the model and its threshold together.
4. Update the managed selection in `scripts/pi_service_config.py`.
5. Keep standalone listener defaults and service templates consistent.
6. Build and deploy through the [release procedure](release-packaging.md).

The listener accepts wake-model, threshold and verifier-directory options. The [configuration reference](configuration.md#microphone-listener) defines their use.

The acoustic classifier uses openWakeWord features with a CC BY-NC-SA license. Preserve model and feature licenses when distributing replacements.
