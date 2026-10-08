import math
import struct
import unittest

from orion_voice.xvf_control import PARAMETERS, STATUS_RETRY, XvfControl, decode


class DecodeTest(unittest.TestCase):
    def test_azimuths_are_four_little_endian_floats(self):
        reply = bytes([0]) + struct.pack("<4f", 0.5, 0.0, 1.5707964, 0.5)
        self.assertEqual(decode("AEC_AZIMUTH_VALUES", reply)[2], struct.unpack("<f", struct.pack("<f", 1.5707964))[0])

    def test_nan_azimuth_means_no_speech(self):
        reply = bytes([0]) + struct.pack("<2f", math.nan, 1.0)
        self.assertEqual(decode("AUDIO_MGR_SELECTED_AZIMUTHS", reply), (None, 1.0))

    def test_doa_value_is_two_uint16(self):
        self.assertEqual(decode("DOA_VALUE", bytes([0]) + struct.pack("<2H", 135, 1)), (135, 1))

    def test_rejects_wrong_length_and_error_status(self):
        with self.assertRaises(ValueError):
            decode("AEC_AECCONVERGED", bytes([0, 1, 0]))
        with self.assertRaises(ValueError):
            decode("AEC_AECCONVERGED", bytes([3]) + struct.pack("<i", 1))


class ControlTest(unittest.TestCase):
    def test_read_sets_read_flag_and_retries_while_busy(self):
        calls = []
        replies = [bytes([STATUS_RETRY]) + bytes(4), bytes([0]) + struct.pack("<i", 1)]

        def transfer(value, index, length):
            calls.append((value, index, length))
            return replies.pop(0)

        control = XvfControl(transfer, sleep=lambda _: None)
        self.assertEqual(control.read("AEC_AECCONVERGED"), (1,))
        parameter = PARAMETERS["AEC_AECCONVERGED"]
        self.assertEqual(calls, [(0x80 | parameter.command, parameter.resource, 5)] * 2)

    def test_gives_up_when_board_stays_busy(self):
        control = XvfControl(lambda *_: bytes([STATUS_RETRY]) + bytes(4), retries=3, sleep=lambda _: None)
        with self.assertRaises(RuntimeError):
            control.read("AEC_AECCONVERGED")


if __name__ == "__main__":
    unittest.main()
