import json
import math
from pathlib import Path
import tempfile
import unittest
import wave

import numpy as np

from orion_voice.rustpotter import WakeDetection
from orion_voice.xvf_measure import (SAMPLE_RATE, analyse_session, circular_stats, mounting_offset,
                                     parser, wrap180)


def write_session(folder: Path, audio: np.ndarray, rows, *, events=None, expected=None, label="trial"):
    folder.mkdir(parents=True)
    with wave.open(str(folder / "capture.wav"), "wb") as sink:
        sink.setnchannels(audio.shape[1])
        sink.setsampwidth(2)
        sink.setframerate(SAMPLE_RATE)
        sink.writeframes(audio.astype("<i2").tobytes())
    (folder / "control.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
    (folder / "meta.json").write_text(json.dumps({"label": label, "events": events or {},
                                                   "expected_azimuth_deg": expected}))


def row(t, azimuth_deg, energy, converged=1):
    azimuth = math.radians(azimuth_deg)
    return {"t": t, "AEC_AZIMUTH_VALUES": [0.0, 0.0, 0.0, azimuth], "AEC_SPENERGY_VALUES": [0, 0, 0, energy],
            "AUDIO_MGR_SELECTED_AZIMUTHS": [azimuth, azimuth], "AEC_AECCONVERGED": [converged],
            "AEC_AECPATHCHANGE": [0], "DOA_VALUE": [round(azimuth_deg) % 360, 1]}


class StatsTest(unittest.TestCase):
    def test_circular_mean_wraps_through_zero(self):
        stats = circular_stats([350, 10, 355, 5])
        self.assertAlmostEqual(wrap180(stats["mean_deg"]), 0.0, delta=0.2)
        self.assertLess(stats["spread_deg"], 10)

    def test_empty_stats(self):
        self.assertEqual(circular_stats([])["count"], 0)


class SessionTest(unittest.TestCase):
    def test_playback_trial_reports_levels_aec_direction_and_wake_hits(self):
        seconds = 4
        audio = np.zeros((SAMPLE_RATE * seconds, 2), dtype=np.int16)
        audio[SAMPLE_RATE:3 * SAMPLE_RATE, 0] = 3000  # residual echo on ch0 during playback
        audio[SAMPLE_RATE:3 * SAMPLE_RATE, 1] = 300
        rows = [row(t / 10, 90, 0 if t < 15 else 5, converged=int(t >= 15)) for t in range(40)]
        events = {"play_start": 1.0, "play_end": 3.0}

        class Detector:
            def __init__(self):
                self.frames = 0

            def process(self, pcm):
                self.frames += 1
                return WakeDetection("hey_orion", 0.6) if self.frames == 100 else None

        with tempfile.TemporaryDirectory() as root:
            folder = Path(root) / "t1"
            write_session(folder, audio, rows, events=events)
            summary = analyse_session(folder, detector_factory=Detector)

        self.assertGreater(summary["levels_dbfs"]["ch0"]["during"], summary["levels_dbfs"]["ch1"]["during"])
        self.assertEqual(summary["aec"]["converged_fraction"], 0.75)
        self.assertAlmostEqual(summary["aec"]["first_converged_after_s"], 0.5)
        self.assertAlmostEqual(summary["direction"]["during"]["auto_select_beam"]["mean_deg"], 90.0, delta=0.1)
        self.assertEqual(summary["direction"]["before"]["auto_select_beam"]["count"], 0)
        self.assertEqual(summary["wake_hits"]["ch0"], [{"t": 2.0, "score": 0.6, "during_playback": True}])

    def test_six_channel_trial_reports_echo_reduction(self):
        audio = np.zeros((SAMPLE_RATE * 3, 6), dtype=np.int16)
        audio[SAMPLE_RATE:2 * SAMPLE_RATE, 2:] = 10000
        audio[SAMPLE_RATE:2 * SAMPLE_RATE, 0] = 100
        audio[SAMPLE_RATE:2 * SAMPLE_RATE, 1] = 1000
        with tempfile.TemporaryDirectory() as root:
            folder = Path(root) / "t6"
            write_session(folder, audio, [], events={"play_start": 1.0, "play_end": 2.0})
            reduction = analyse_session(folder)["echo_reduction_db"]
        self.assertAlmostEqual(reduction["ch0"], 40.0, delta=0.2)
        self.assertAlmostEqual(reduction["ch1"], 20.0, delta=0.2)


class MountingTest(unittest.TestCase):
    def summaries(self, sign, offset):
        result = []
        for expected in (0, 90, 180, 270):
            measured = (sign * expected + offset) % 360
            result.append({"label": f"at{expected}", "expected_azimuth_deg": expected,
                           "direction": {"all": {"auto_select_beam": {"mean_deg": measured}}}})
        return result

    def test_recovers_clockwise_board_with_offset(self):
        fit = mounting_offset(self.summaries(-1, 120))
        self.assertEqual(fit["sign"], -1)
        self.assertAlmostEqual(fit["offset_deg"], 120, delta=0.2)
        self.assertLess(fit["worst_residual_deg"], 0.5)

    def test_needs_three_positions(self):
        self.assertIsNone(mounting_offset(self.summaries(1, 0)[:2]))


class CliTest(unittest.TestCase):
    def test_record_arguments(self):
        args = parser().parse_args(["record", "--label", "front", "--seconds", "5", "--expected-azimuth", "0"])
        self.assertEqual((args.channels, args.device), (2, "plughw:CARD=Array,DEV=0"))


if __name__ == "__main__":
    unittest.main()
