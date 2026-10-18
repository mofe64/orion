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
    def jitter_samples(self):
        return [{'phase': 'executing', 'sequence': index, 'trajectory_revision': 0,
                 'runtime_time_seconds': index * .02,
                 'joints': {'head': {'commanded_position_rad': command,
                                     'measured_position_rad': command + .04}}}
                for index in range(101)
                for command in [abs(index - 50) * .002]]

    def test_directional_jitter_separates_lowering_stutter_from_offset(self):
        samples = self.jitter_samples()
        for index, row in enumerate(samples):
            if index > 55:
                row['joints']['head']['measured_position_rad'] += .003 * (-1) ** index
        result = analysis.directional_jitter(samples, 'head')
        self.assertLess(result['negative']['rms_rad'], 1e-12)
        self.assertGreater(result['positive']['rms_rad'], .002)
        self.assertGreater(result['positive']['peak_to_peak_rad'], .004)

    def test_directional_jitter_excludes_discontinuities_and_settling(self):
        for change in ['missing', 'delayed', 'replacement', 'settling']:
            with self.subTest(change=change):
                samples = self.jitter_samples()
                for row in samples[50:]:
                    row['joints']['head']['measured_position_rad'] += 1
                    if change == 'missing': row['sequence'] += 1
                    if change == 'delayed': row['runtime_time_seconds'] += .1
                    if change == 'replacement': row['trajectory_revision'] = 1
                    if change == 'settling': row['phase'] = 'settling'
                result = analysis.directional_jitter(samples, 'head')
                self.assertTrue(all(value['rms_rad'] < 1e-12 for value in result.values()))

    def test_directional_jitter_retains_stationary_holding_noise(self):
        samples = self.jitter_samples()
        for index, row in enumerate(samples):
            row['joints']['head'] = {'commanded_position_rad': 0,
                                     'measured_position_rad': .003 * (-1) ** index}
        result = analysis.directional_jitter(samples, 'head')
        self.assertEqual(set(result), {'stationary'})
        self.assertAlmostEqual(result['stationary']['rms_rad'], .0024)

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
            self.assertAlmostEqual(joint['terminal_settling_error_rad'], .04)
            rows[-2]['state'] = 'timed_out'
            path.write_text('\n'.join(json.dumps(r) for r in rows))
            result = analysis.analyze(path)
            self.assertIsNone(result['runs'][0]['joints']['shoulder']['steady_state_error_rad'])
            self.assertAlmostEqual(result['runs'][0]['joints']['shoulder']['terminal_settling_error_rad'], .04)
            self.assertAlmostEqual(result['runs'][0]['joints']['shoulder']['settle_position_std_rad'], 0)
            self.assertEqual(result['diagnostics']['settle_timeouts'], [1])
            self.assertIn('Settling error', analysis.markdown(result))
            rows[-2]['state'] = 'cancelled'
            path.write_text('\n'.join(json.dumps(r) for r in rows))
            result = analysis.analyze(path)
            self.assertIsNone(result['runs'][0]['joints']['shoulder']['terminal_settling_error_rad'])

    def test_corrupted_json_is_fatal(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'capture.jsonl'
            path.write_text('{broken')
            with self.assertRaisesRegex(ValueError, 'line 1'):
                analysis.analyze(path)


if __name__ == '__main__':
    unittest.main()
