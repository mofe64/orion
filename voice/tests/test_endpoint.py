from array import array
import sys
import unittest

from orion_voice.endpoint import EndpointConfig, EnergyEndpointDetector, ListeningNoise, pcm16_rms


def frame(energy=0, dc=0):
    samples = array('h', [dc + energy, dc - energy] * 160)
    if sys.byteorder != 'little': samples.byteswap()
    return samples.tobytes()


class EndpointTests(unittest.TestCase):
    def test_dc_offset_does_not_count_as_speech(self):
        self.assertEqual(pcm16_rms(frame(1000, 6000)), 1000)
        self.assertEqual(pcm16_rms(frame(0, 12000)), 0)
        detector = EnergyEndpointDetector()
        detector.prime_detected_speech()
        for _ in range(59): self.assertFalse(detector.accept(frame(0, 12000)))
        self.assertTrue(detector.accept(frame(0, 12000)))
        self.assertEqual(detector.end_reason, 'silence')
        self.assertEqual(detector.capture_ms, 1200)

    def test_noise_estimate_excludes_wake_and_resists_intermittent_speech(self):
        noise = ListeningNoise()
        self.assertEqual(noise.threshold(), 500)
        for _ in range(200): noise.accept(frame(1000, 6000))
        for _ in range(100): noise.accept(frame(6000, 6000))
        self.assertEqual(noise.threshold(), 3000)
        # A later quieter room replaces the old history rather than growing it forever.
        for _ in range(300): noise.accept(frame(100))
        self.assertEqual(noise.threshold(), 500)

    def test_noisy_room_ends_one_second_after_speech(self):
        detector = EnergyEndpointDetector(EndpointConfig(speech_rms=2500))
        detector.prime_detected_speech()
        for _ in range(100): self.assertFalse(detector.accept(frame(6000, 6000)))
        for _ in range(49): self.assertFalse(detector.accept(frame(1000, 6000)))
        self.assertTrue(detector.accept(frame(1000, 6000)))
        self.assertEqual(detector.capture_ms, 3000)

    def test_short_pauses_and_resumed_speech_do_not_end_capture(self):
        detector = EnergyEndpointDetector()
        detector.prime_detected_speech()
        for energy, count in [(2000, 50), (0, 45), (2000, 50), (0, 49)]:
            for _ in range(count): self.assertFalse(detector.accept(frame(energy)))
        self.assertTrue(detector.accept(frame()))

    def test_isolated_spikes_do_not_restart_silence(self):
        detector = EnergyEndpointDetector()
        detector.prime_detected_speech()
        for _ in range(50): detector.accept(frame(2000))
        for i in range(49):
            self.assertFalse(detector.accept(frame(3000 if i in (10, 20, 30) else 0)))
        self.assertTrue(detector.accept(frame()))

    def test_continuous_speech_still_has_fifteen_second_limit(self):
        detector = EnergyEndpointDetector()
        detector.prime_detected_speech()
        for _ in range(749): self.assertFalse(detector.accept(frame(2000)))
        self.assertTrue(detector.accept(frame(2000)))
        self.assertEqual(detector.end_reason, 'max_duration')

    def test_followup_needs_speech_and_has_bounded_empty_wait(self):
        detector = EnergyEndpointDetector()
        for _ in range(749): self.assertFalse(detector.accept(frame()))
        self.assertTrue(detector.accept(frame()))
        self.assertEqual(detector.end_reason, 'max_duration')

    def test_chunk_boundaries_do_not_change_endpoint(self):
        pcm = frame(2000) * 50 + frame() * 50
        for size in (137, 640, len(pcm)):
            detector = EnergyEndpointDetector()
            detector.prime_detected_speech()
            for start in range(0, len(pcm), size): detector.accept(pcm[start:start + size])
            self.assertEqual((detector.capture_ms, detector.end_reason), (2000, 'silence'))
