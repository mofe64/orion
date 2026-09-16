import unittest
import numpy as np

from orion_voice.endpoint import EndpointConfig
from orion_voice.vad import SileroActivity, SileroEndpoint


class Model:
    def __init__(self, probability=.9):
        self.probability = probability
        self.inputs = []

    def run(self, _, inputs):
        self.inputs.append({k: v.copy() for k, v in inputs.items()})
        return np.array([[self.probability]], dtype=np.float32), inputs['state'] + 1


class VadTests(unittest.TestCase):
    def test_vad_gain_is_bounded_and_does_not_modify_capture(self):
        model = Model()
        activity = SileroActivity(model, input_gain=2)
        pcm = np.full(512, 20000, dtype='<i2').tobytes()
        activity.accept(pcm)
        self.assertEqual(np.frombuffer(pcm, dtype='<i2')[0], 20000)
        self.assertTrue(np.all(model.inputs[0]['input'][0, 64:] == 1))
        with self.assertRaises(ValueError): SileroActivity(model, input_gain=float('nan'))

    def test_capture_boundaries_preserve_every_sample_and_context(self):
        model = Model()
        activity = SileroActivity(model)
        pcm = np.arange(2560, dtype='<i2').tobytes()
        for offset in range(0, len(pcm), 640):
            activity.accept(pcm[offset:offset+640])
        restored = np.concatenate([x['input'][0, 64:] for x in model.inputs])
        np.testing.assert_array_equal(restored, np.arange(2560) / 32768.)
        self.assertEqual(len(activity.pending), 0)
        np.testing.assert_array_equal(model.inputs[1]['input'][0, :64], model.inputs[0]['input'][0, -64:])

    def test_quiet_speech_does_not_trigger_old_energy_floor(self):
        model = Model()
        detector = SileroEndpoint(EndpointConfig(), model)
        detector.prime_detected_speech()
        pcm = np.tile([300, -300], 160).astype('<i2').tobytes()
        for _ in range(400):
            self.assertFalse(detector.accept(pcm))
        model.probability = .01
        for _ in range(55):
            if detector.accept(bytes(640)):
                break
        self.assertEqual(detector.end_reason, 'silence')
        self.assertGreaterEqual(detector.capture_ms, 9000)

    def test_recurrent_state_is_not_shared_between_turns(self):
        model = Model()
        first, second = SileroActivity(model), SileroActivity(model)
        first.accept(bytes(2048))
        second.accept(bytes(1024))
        self.assertTrue(np.all(model.inputs[-1]['state'] == 0))

    def test_hysteresis_and_continuous_speech_limit(self):
        model = Model()
        detector = SileroEndpoint(EndpointConfig(max_utterance_ms=1600), model)
        for _ in range(79):
            self.assertFalse(detector.accept(bytes(640)))
        self.assertTrue(detector.accept(bytes(640)))
        self.assertEqual(detector.end_reason, 'max_duration')
        activity = SileroActivity(model)
        self.assertTrue(activity.accept(bytes(1024)))
        model.probability = .4
        self.assertTrue(activity.accept(bytes(1024)))
        model.probability = .2
        self.assertFalse(activity.accept(bytes(1024)))
