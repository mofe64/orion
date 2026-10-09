# Train and evaluate “Hey Orion”

Orion's managed listener uses the six-example Rustpotter reference model,
followed by the openWakeWord “Hey Orion” phrase verifier. The
[listener configuration](configuration.md#pi-runtime-and-listener) lists the
active model, both wake thresholds and the retained trained model for comparison.
Rustpotter can load either `.rpw` without changing the listener's audio path.

The verifier's classifier was trained on screened Piper speech and reviewed
original recordings. Its models inherit the non-commercial CC BY-NC-SA licence
of openWakeWord's training features.

## Record the baseline

Measure the active reference model on the Pi with its normal capture path and
[managed listener settings](configuration.md#pi-runtime-and-listener), including
acoustic phrase verification and Qwen's complete-recording wake check. Log each
intended wake attempt,
whether Rustpotter proposed it, whether the verifier accepted it, whether Qwen confirmed it,
the time until confirmation, and the speaker's distance and room conditions.
Also count Rustpotter candidates and confirmed false wakes per hour of ordinary
non-wake audio. The listener does not save microphone audio by default; recording
sessions must be explicitly started and labelled.

## Build a private dataset

Record multiple speakers saying “Hey Orion” at different distances, directions,
volumes and speaking rates. Include quiet rooms, normal household noise, music,
TV and Orion's own speaker. Add non-wake clips with silence, ordinary speech,
similar sounding phrases and the false candidates found in the baseline. Use the
Pi's microphone path for a representative portion of both classes. Get consent
from anyone whose voice is recorded.

Keep recordings outside Git in a private experiment directory. Use mono 16 kHz
WAV clips of similar length, with the wake phrase fully inside each positive
clip. Rustpotter expects a label in positive filenames such as
`[hey_orion]001.wav`; untagged filenames are the `none` class. Split training
and held-out test data by speaker, recording session and room where possible,
so near-duplicate clips cannot make the test look better than deployment.
Track each file's speaker/session, microphone, condition, label, split and
retention decision in a private manifest.

The [Rustpotter CLI example](https://github.com/GiviMAD/rustpotter-cli#creating-a-wakeword-model)
has roughly 250 positive and 1,800 negative training clips. Those counts are an
example, not a minimum or a promised accuracy. Collect enough variety to expose
the failures seen in Orion's baseline.

## Train and compare

Train an initial `small` classifier on a development computer with
[`rustpotter-cli train`](https://github.com/GiviMAD/rustpotter-cli#creating-a-wakeword-model).
Keep the CLI version, input manifest, command, result file hash and training
metrics with the experiment. Try more than one initialization because training
results vary between runs. Test each candidate against the same untouched audio
as the reference, then replay longer continuous recordings to measure false
wakes per hour. File-level accuracy alone does not capture accidental activations
or delayed detection in a live stream.

Tune Rustpotter's threshold and minimum positive frames on validation audio,
then freeze them before the held-out test. Compare wake misses, candidate and
confirmed false wakes per hour, confirmation latency, and Pi CPU and memory use.
Keep Qwen's complete-recording wake check in the pipeline: Rustpotter must propose a phrase
before either verifier can check it, so neither can recover a missed candidate. Promote a
trained `.rpw` only when the same test set shows a useful improvement without a
material regression in the other measures. Keep the reference for comparison and rollback.

## Deploy and clean up

The listener accepts `--wake-model PATH` and `--threshold VALUE`. Both the
trained model and reference are present in every managed release. Select the
model and its matching threshold together in `ACTIVE_WAKE_MODEL` and
`ACTIVE_WAKE_THRESHOLD` in `scripts/pi_service_config.py`, the listener service
template, and the standalone listener defaults. Then commit, push, and run the
[Pi deployment procedure](quickstart.md#deploy-to-the-pi). Record model hash and
threshold with each release. With the
verifier active, the chime, light pulse and body waking wait for its acceptance;
Qwen still checks the complete recording before any command runs.

The experiment manifest should list all recordings, temporary WAVs, candidate
models, logs and capture tools. Delete rejected models and raw recordings at the
agreed retention date, and confirm the remaining files against that manifest.
Keep both models, their reproducibility metadata and non-audio
evaluation summary in the project.

Rustpotter's [library overview](https://github.com/GiviMAD/rustpotter#overview)
describes reference and trained models; its
[CLI guide](https://github.com/GiviMAD/rustpotter-cli#basic-usage) documents
training, live spotting and file tests.
