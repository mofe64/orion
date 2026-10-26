import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from recorder import Recorder, day_of, merge_rows, servo_faults, turn_latency

DAY = 1_800_000_000.0


def joint(name, temp=40.0, current=200.0, volts=6.0, status=0):
    return {'name': name, 'temperature_c': temp, 'current_ma': current, 'voltage_v': volts, 'status': status}


class Fake:
    def __init__(self):
        self.status = {'joints': [joint('base_yaw_joint')], 'loop': {'worst_work_ms': 3.0, 'worst_delay_ms': 1.0,
                       'late_ticks': 0, 'bus_reconnects': 0}, 'bus_fault': None, 'bus_error_count': 0, 'mode': 'holding'}
        self.character = {'rest': {'state': 'awake'}, 'character': {'state': 'home_idle'}}
        self.down = False

    def request(self, command):
        if self.down:
            raise OSError('connection refused')
        return self.status if command == 'status' else self.character


class RecorderTests(unittest.TestCase):
    def recorder(self, folder, history=None):
        self.fake, self.now, self.tick = Fake(), [DAY], [0.0]
        metrics = Mock(latest={'cpu': {'temperature_c': 50.0, 'percent': 20.0, 'throttling': {'now': {}}},
                               'ram': {'percent': 40.0}, 'disk': {'data': {'percent': 30.0}},
                               'services': [{'name': 'oriond', 'active_state': 'active', 'restarts': 0}]})
        recorder = Recorder(self.fake.request, metrics, folder, history or Path(folder) / 'history',
                            clock=lambda: self.now[0], monotonic=lambda: self.tick[0])
        self.metrics = metrics
        return recorder

    def step(self, recorder, seconds=1):
        for _ in range(seconds):
            self.now[0] += 1
            self.tick[0] += 1
            recorder.tick()

    def test_rows_keep_the_worst_second_and_survive_in_files(self):
        with tempfile.TemporaryDirectory() as folder:
            recorder = self.recorder(folder)
            self.step(recorder, 4)
            self.fake.status['joints'] = [joint('base_yaw_joint', temp=52.0, current=-900.0, volts=5.2)]
            self.step(recorder)
            self.fake.status['joints'] = [joint('base_yaw_joint')]
            self.step(recorder, 60)
            files = sorted(path.name for path in (Path(folder) / 'recorder').iterdir())
            self.assertIn(f'readings-{day_of(DAY)}.jsonl', files)
            rows = recorder.snapshot(1)['readings']
            self.assertGreaterEqual(len(rows), 5)
            worst = rows[0]['joints']['base_yaw_joint']
            self.assertEqual((worst['temp'], worst['current'], worst['volt_min']), (52.0, 900.0, 5.2))
            merged = merge_rows(rows)
            self.assertEqual(merged['joints']['base_yaw_joint']['temp'], 52.0)
            # Old days are removed after 7 days.
            old = Path(folder) / 'recorder' / f'readings-{day_of(DAY - 9 * 86400)}.jsonl'
            old.write_text('{}\n')
            recorder.prune(self.now[0])
            self.assertFalse(old.exists())

    def test_incidents_start_and_recover_once(self):
        with tempfile.TemporaryDirectory() as folder:
            recorder = self.recorder(folder)
            self.step(recorder)
            self.fake.status['bus_fault'] = 'read timeout on servo 3'
            self.fake.status['joints'] = [joint('head_roll_joint', temp=66.0, status=36)]
            self.step(recorder, 3)
            self.fake.status['bus_fault'] = None
            self.fake.status['joints'] = [joint('head_roll_joint', temp=62.0)]
            self.step(recorder)
            self.fake.status['joints'] = [joint('head_roll_joint', temp=59.0)]
            self.fake.status['loop'] = {'late_ticks': 40, 'worst_delay_ms': 30.0, 'worst_work_ms': 25.0, 'bus_reconnects': 1}
            self.metrics.latest['services'] = [{'name': 'oriond', 'active_state': 'active', 'restarts': 1}]
            self.step(recorder)
            self.fake.down = True
            self.step(recorder, 3)
            messages = [item['message'] for item in reversed(recorder.snapshot(1)['incidents'])]
            self.assertEqual(messages, [
                'Orion lost contact with its servos.',
                'Servo head_roll_joint reported a fault: temperature, overload.',
                'Servo head_roll_joint is hot: 66 °C.',
                'Orion has contact with its servos again.',
                'Movement and lights restarted.',
                'The servo board disconnected and came back.',
                'Movement timing slipped: 40 late steps in 10 s.',
                'Servo head_roll_joint cooled down.',
                'Orion’s movement service stopped answering.',
            ])
            # Incidents are on disk at once and come back after a restart.
            again = Recorder(self.fake.request, self.metrics, folder, clock=lambda: self.now[0])
            self.assertEqual(len(again.snapshot(1)['incidents']), len(messages))

    def test_reply_speed_comes_from_saved_voice_turns(self):
        turn = {'sessionId': 's1', 'events': [
            {'at': '2026-10-26T20:00:00.000Z', 'event': {'type': 'transcript.final', 'durationMs': 1500}},
            {'at': '2026-10-26T20:00:03.000Z', 'event': {'type': 'agent.response', 'durationMs': 2600}},
            {'at': '2026-10-26T20:00:03.400Z', 'event': {'type': 'speech.started'}}]}
        self.assertEqual(turn_latency(turn)['first_voice_ms'], 3400)
        self.assertEqual(turn_latency(turn)['total_ms'], 4900)
        self.assertIsNone(turn_latency({'events': turn['events'][:2]}))
        with tempfile.TemporaryDirectory() as folder:
            history = Path(folder) / 'history'
            path = history / 'conversations/c1/turns/s1.json'
            path.parent.mkdir(parents=True)
            path.write_text(json.dumps(turn))
            recorder = self.recorder(folder, history)
            self.now[0] = turn_latency(turn)['t'] + 60
            recorder.scan_latency()
            recorder.scan_latency()
            self.assertEqual([item['think_ms'] for item in recorder.snapshot(1)['latency']], [2600])

    def test_servo_fault_names(self):
        self.assertEqual(servo_faults(1 | 32), 'voltage, overload')
        self.assertEqual(servo_faults(64), 'code 64')


if __name__ == '__main__':
    unittest.main()
