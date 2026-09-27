import importlib.util
import json
import math
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts/analyze_tracking.py'
spec = importlib.util.spec_from_file_location('analyze_tracking', SCRIPT)
analysis = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analysis)


class TrackingAnalysisTests(unittest.TestCase):
    def test_lag_ignores_bias_and_amplitude_loss(self):
        times = [i * .02 for i in range(251)]
        commanded = [.06 * math.sin(t * math.tau / 3) for t in times]
        measured = [.035 * math.sin((t - .12) * math.tau / 3) - .04 for t in times]
        lag, correlation = analysis.approximate_lag(times, commanded, measured)
        self.assertAlmostEqual(lag, 120, delta=10)
        self.assertGreater(correlation, .999)

    def test_stationary_feedback_has_no_identifiable_lag(self):
        self.assertEqual(analysis.approximate_lag(list(range(20)), [0] * 20, [.04] * 20), (None, None))

    def test_replacement_clock_and_terminal_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'capture.jsonl'
            rows = [{'kind': 'header', 'schema_version': 1}]
            for i in range(151):
                t = i * .02
                command = .05 * math.sin(math.pi * min(t, 2) / 2)
                measured = command * .5 - .04
                rows.append({
                    'kind': 'sample', 'sequence': i, 'run_id': 1, 'run_name': 'speaking_performance',
                    'motion_name': 'speaking_performance' if t < 2 else 'speak_settle',
                    'runtime_time_seconds': t, 'elapsed_seconds': t,
                    'trajectory_elapsed_seconds': t if t < 1 else t - 1,
                    'trajectory_revision': int(t >= 1),
                    'phase': 'executing' if t < 2 else 'settling', 'control_work_seconds': .001,
                    'commanded_start': {'shoulder': 0}, 'measured_start': {'shoulder': -.04},
                    'joints': {'shoulder': {'commanded_position_rad': command,
                                          'measured_position_rad': measured, 'measured_velocity_rad_s': 0}},
                })
            rows += [{'kind': 'end', 'run_id': 1, 'state': 'completed'},
                     {'kind': 'summary', 'dropped_records': 0}]
            path.write_text('\n'.join(json.dumps(r) for r in rows))
            report = analysis.analyze(path)
            self.assertEqual(len(report['runs']), 1)
            run = report['runs'][0]
            joint = run['joints']['shoulder']
            self.assertEqual(run['trajectory_revisions'], 2)
            self.assertAlmostEqual(joint['amplitude_ratio'], .5)
            self.assertTrue(joint['loses_over_one_third'])
            self.assertAlmostEqual(joint['steady_state_error_rad'], .04)
            rows[-2]['state'] = 'timed_out'
            path.write_text('\n'.join(json.dumps(r) for r in rows))
            result = analysis.analyze(path)
            self.assertIsNone(result['runs'][0]['joints']['shoulder']['steady_state_error_rad'])
            self.assertEqual(result['diagnostics']['settle_timeouts'], [1])

    def test_corrupted_json_is_fatal(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'capture.jsonl'
            path.write_text('{broken')
            with self.assertRaisesRegex(ValueError, 'line 1'):
                analysis.analyze(path)


if __name__ == '__main__':
    unittest.main()
