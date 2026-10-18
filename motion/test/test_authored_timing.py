"""V2 clips retain their authored timing while their poses can change."""
import json
from pathlib import Path
import pytest
import yaml

ROOT = Path(__file__).resolve().parents[2]
TIMING = json.loads((Path(__file__).parent / 'fixtures/authored_timing.json').read_text())


@pytest.mark.parametrize('name,spec', TIMING.items())
def test_original_duration_and_holds(name, spec):
    frames = yaml.safe_load((ROOT / 'motion/motions/v2' / (name + '.yaml')).read_text())['motion']['keyframes']
    actual = [(frame['duration'], frame.get('hold', 0)) for frame in frames]
    original = spec['original']
    if spec['proportions'] is None:
        assert actual == [tuple(pair) for pair in original]
    else:
        total = lambda pairs: sum(duration + hold for duration, hold in pairs)
        assert total(actual) == pytest.approx(total(original))
        ratio = total(original) / total(spec['proportions'])
        for (duration, hold), (previous_duration, previous_hold) in zip(actual, spec['proportions'], strict=True):
            assert duration == pytest.approx(previous_duration * ratio)
            assert hold == pytest.approx(previous_hold * ratio)
