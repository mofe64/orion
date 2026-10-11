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
        self.entries = [entry for entry in self.entries
                        if 0 <= stamp - int(entry['capture_boottime_ns']) < 600_000_000_000]
        return [dict(entry) for entry in self.entries]

    def update(self, candidates, stamp):
        self.snapshot(stamp)
        used = set()
        for candidate in candidates[:16]:
            matches = [entry for entry in self.entries if entry['category'] == candidate['category']
                       and entry['id'] not in used and math.hypot(
                           math.atan2(math.sin(entry['azimuth'] - candidate['azimuth']),
                                      math.cos(entry['azimuth'] - candidate['azimuth'])),
                           entry['elevation'] - candidate['elevation']) < .12]
            if matches:
                entry = min(matches, key=lambda entry: abs(entry['azimuth'] - candidate['azimuth']))
            elif len(self.entries) < 32:
                entry = {'id': f'object:{self.next_id}'}
                self.next_id += 1
                self.entries.append(entry)
            else:
                continue
            entry.update({key: candidate[key] for key in (
                'category', 'azimuth', 'elevation', 'capture_boottime_ns', 'point')})
            used.add(entry['id'])
