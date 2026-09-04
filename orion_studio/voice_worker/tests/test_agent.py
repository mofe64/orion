from types import SimpleNamespace
import os
import unittest
from unittest.mock import Mock, patch

from orion_voice_worker.agent import CodexAgentProvider, MAX_RESPONSE_CHARACTERS


class FakeThread:
    def __init__(self, response: str) -> None:
        self.response = response
        self.prompts: list[str] = []

    def run(self, text: str) -> object:
        self.prompts.append(text)
        return SimpleNamespace(final_response=self.response)


class CodexAgentProviderTests(unittest.TestCase):
    def test_runtime_override_preserves_model_and_thread_constraints(self) -> None:
        for executable in (None, '', '/Applications/ChatGPT.app/Contents/Resources/codex'):
            with self.subTest(executable=executable):
                runtime = Mock()
                runtime.account.return_value = SimpleNamespace(account=object())
                runtime.thread_start.return_value = FakeThread('Hello from Orion.')
                sdk = SimpleNamespace(
                    Codex=Mock(return_value=runtime), CodexConfig=SimpleNamespace,
                    ApprovalMode=SimpleNamespace(deny_all='deny_all'),
                    Sandbox=SimpleNamespace(read_only='read_only'),
                )
                environment = {} if executable is None else {'ORION_STUDIO_CODEX_BIN': executable}
                with patch.dict(os.environ, environment, clear=True), patch.dict('sys.modules', {'openai_codex': sdk}):
                    provider = CodexAgentProvider('gpt-6-astra')
                    try:
                        config = sdk.Codex.call_args.args[0]
                        self.assertEqual(config.codex_bin, executable or None)
                        options = runtime.thread_start.call_args.kwargs
                        self.assertEqual(options['model'], 'gpt-6-astra')
                        self.assertEqual(options['sandbox'], 'read_only')
                        self.assertEqual(options['approval_mode'], 'deny_all')
                        self.assertTrue(options['ephemeral'])
                        self.assertEqual(provider.respond('Say hello.'), 'Hello from Orion.')
                    finally:
                        provider.close()
                    runtime.close.assert_called_once()

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
