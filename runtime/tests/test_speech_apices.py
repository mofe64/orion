import importlib.util
import math
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('apices', Path(__file__).resolve().parents[1]/'scripts/analyze_speech_apices.py')
apices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(apices)


class SpeechApicesTests(unittest.TestCase):
    def test_software_clock_offset_measured_lag_and_unperformed_future(self):
        event = dict(event='speech.motion_compiled', motion_run_id=4, trajectory_start_runtime_seconds=10.0,
                     apices=[dict(marker='gesture_1_speak_emphasis_nod@peak=1.17', commanded_apex_seconds=1.0, peak_seconds=1.17),
                             dict(marker='gesture_3_speak_emphasis_nod@peak=4.17', commanded_apex_seconds=4.0, peak_seconds=4.17)])
        samples = [dict(kind='sample', run_id=4, motion_name='speaking_performance', runtime_time_seconds=10+i*.02,
                        joints={'head_pitch_joint':dict(measured_position_rad=.2*math.exp(-((i*.02-1.08)/.12)**2))}) for i in range(150)]
        result = apices.analyze(samples, [event])
        self.assertAlmostEqual(result['compiled']['median_ms'], -170.0)
        self.assertAlmostEqual(result['measured_nod_pitch']['median_ms'], -90.0)
        self.assertEqual(result['strokes'][1]['measurement'], 'unperformed_future')

    def test_boundary_maximum_is_not_reported_as_a_measured_apex(self):
        event = dict(event='speech.motion_compiled', motion_run_id=4, trajectory_start_runtime_seconds=0.0,
                     apices=[dict(marker='speak_emphasis_nod@peak=1.17', commanded_apex_seconds=1., peak_seconds=1.17)])
        samples = [dict(kind='sample', run_id=4, motion_name='speaking_performance', runtime_time_seconds=i*.02,
                        joints={'head_pitch_joint':dict(measured_position_rad=i*.02)}) for i in range(150)]
        result = apices.analyze(samples, [event])
        self.assertEqual(result['measured_nod_pitch']['count'], 0)
        self.assertEqual(result['strokes'][0]['measurement'], 'maximum_on_window_boundary')


if __name__ == '__main__':
    unittest.main()
