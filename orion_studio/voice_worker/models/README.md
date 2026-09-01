# Runtime voice models

Only small, commissioned runtime assets belong in this directory. Training
recordings, evaluators, generated datasets, and reports live in the sibling
`voice-model-lab` workspace and are not part of Orion Studio.

`hey_orion_reference.rpw` is the Rustpotter reference selected from the local
evaluation at a default runtime threshold of `0.400`. Qwen3-ASR, Chatterbox
Turbo 8-bit, and its tokenizer are downloaded into the user's Hugging Face
cache by `../.venv/bin/orion-voice-models`; their weights are not stored here.
See [Manage Studio Voice models](../../../docs/how-to/manage-studio-voice-models.md)
for the model lifecycle and fresh-device procedure.
