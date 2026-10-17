"""Pinned shared model assets; update releases without downloading them twice."""
import json
from pathlib import Path
from prepare_pi_voice_stack import download


def prepare_vision_models(root, release):
    assets = json.loads((Path(__file__).with_name('release-models.json')).read_text())
    for entry in assets:
        source = download(root, entry['url'], 'models/payload/' + entry['path'], entry['sha256'])
        target = release / entry['path']
        target.parent.mkdir(parents=True, exist_ok=True)
        target.symlink_to(source)
