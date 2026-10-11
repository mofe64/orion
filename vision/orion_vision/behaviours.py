"""Opt-in preferences and an in-memory, capture-grounded object map."""
import json
import math
from pathlib import Path

KEYS = ('object_glances', 'mutual_gaze', 'arrival_notice', 'new_object_peek', 'presence_defers_rest')


def settings(path):
    off = dict.fromkeys(KEYS, False)
    try:
        value = json.loads(Path(path).read_text())
        if not isinstance(value, dict) or any(type(item) is not bool for item in value.values()):
            return off
        return {key: value.get(key, False) for key in KEYS}
    except (OSError, ValueError):
        return off


def overlaps(a, b):
    return (a[0] < b[0] + b[2] and b[0] < a[0] + a[2]
            and a[1] < b[1] + b[3] and b[1] < a[1] + a[3])


class ObjectMap:
    def __init__(self):
        self.entries = []
        self.next_id = 1

    def clear(self):
        self.entries.clear()  # Never reuse IDs within this service lifetime.

    def snapshot(self, stamp):
        # Readers may race a privacy clear; never write their old view back.
        return [dict(entry) for entry in self.entries
                if 0 <= stamp - int(entry['capture_boottime_ns']) < 600_000_000_000]

    def update(self, candidates, stamp):
        self.entries = self.snapshot(stamp)  # Called under FacePipeline.changed.
        used = set()
        added = []
        for candidate in candidates[:16]:
            matches = [entry for entry in self.entries if entry['category'] == candidate['category']
                       and entry['id'] not in used and math.hypot(
                           math.atan2(math.sin(entry['azimuth'] - candidate['azimuth']),
                                      math.cos(entry['azimuth'] - candidate['azimuth'])),
                           entry['elevation'] - candidate['elevation']) < .12]
            if matches:
                entry = min(matches, key=lambda entry: abs(entry['azimuth'] - candidate['azimuth']))
            elif len(self.entries) < 32:
                entry = {'id': f'object:{self.next_id}', 'interest_key':
                         f"object:{candidate['category']}:{round(candidate['azimuth'] / .2)}:{round(candidate['elevation'] / .2)}"}
                self.next_id += 1
                self.entries.append(entry)
                added.append((entry, candidate))
            else:
                continue
            entry.update({key: candidate[key] for key in (
                'category', 'azimuth', 'elevation', 'capture_boottime_ns', 'point')})
            used.add(entry['id'])
        return [{**entry, 'box': candidate.get('box'), 'confidence': candidate.get('confidence', 0.)}
                for entry, candidate in added]
