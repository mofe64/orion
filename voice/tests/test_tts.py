import unittest

from orion_voice.tts import _load_nano_model


class NanoModelLoadingTests(unittest.TestCase):
    def test_selects_nano_on_the_requested_device(self) -> None:
        class CompatibleModel:
            call = None

            @classmethod
            def from_pretrained(cls, **arguments):
                cls.call = arguments
                return "model"

        self.assertEqual(_load_nano_model(CompatibleModel, "cpu"), "model")
        self.assertEqual(CompatibleModel.call, {"device": "cpu", "nano": True})

    def test_explains_an_installed_package_without_nano_support(self) -> None:
        class LegacyModel:
            @classmethod
            def from_pretrained(cls, device):
                return device

        with self.assertRaisesRegex(RuntimeError, "does not support Nano"):
            _load_nano_model(LegacyModel, "cpu")


if __name__ == "__main__":
    unittest.main()
