from types import SimpleNamespace
import unittest

from orion_voice_worker.agent import CodexAgentProvider, MAX_RESPONSE_CHARACTERS


class FakeThread:
    def __init__(self, response: str) -> None:
        self.response = response
        self.prompts: list[str] = []

    def run(self, text: str) -> object:
        self.prompts.append(text)
        return SimpleNamespace(final_response=self.response)


class CodexAgentProviderTests(unittest.TestCase):
    def test_returns_normalized_spoken_response(self) -> None:
        thread = FakeThread("  Hello,\n  I am Orion.  ")
        provider = CodexAgentProvider("test-model", thread_factory=lambda _model: thread)

        self.assertEqual(provider.respond("  say hello  "), "Hello, I am Orion.")
        self.assertEqual(thread.prompts, ["say hello"])
        self.assertEqual(provider.provider, "codex")
        self.assertEqual(provider.model_name, "test-model")

    def test_rejects_empty_input_and_output(self) -> None:
        provider = CodexAgentProvider(thread_factory=lambda _model: FakeThread("  "))
        with self.assertRaisesRegex(ValueError, "cannot be empty"):
            provider.respond(" ")
        with self.assertRaisesRegex(RuntimeError, "no spoken response"):
            provider.respond("hello")

    def test_caps_unexpectedly_long_responses(self) -> None:
        provider = CodexAgentProvider(
            thread_factory=lambda _model: FakeThread("word " * MAX_RESPONSE_CHARACTERS)
        )
        response = provider.respond("hello")
        self.assertLessEqual(len(response), MAX_RESPONSE_CHARACTERS + 1)
        self.assertTrue(response.endswith("…"))


if __name__ == "__main__":
    unittest.main()
