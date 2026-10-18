"""Scheduling and score policy for each detector backend."""
from dataclasses import dataclass


@dataclass(frozen=True)
class Rates:
    idle_hz: int
    voice_hz: int
    objects_seconds: float


RATES = {'cpu': Rates(10, 3, 60.), 'hailo': Rates(15, 15, 5.)}
# SCRFD probabilities have their own scale. The Model Zoo SCRFD configuration
# uses .02 for evaluation; .5 is the model's upstream inference default.
SCORES = {'cpu': {'faces': .75, 'palms': .6, 'objects': .4},
          'hailo': {'faces': .5, 'palms': .6, 'objects': .4}}


def backend(detector):
    value = getattr(detector, 'backend', 'cpu')
    return value if isinstance(value, str) and value in RATES else 'cpu'


def rates(detector):
    return RATES[backend(detector)]
