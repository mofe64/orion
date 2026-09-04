from __future__ import annotations

import os
from pathlib import Path
import tempfile
from typing import Callable, Protocol


ORION_INSTRUCTIONS = """You are Orion, a conversational desk-lamp companion.
Answer the user's spoken request in at most two concise sentences suitable for speech.
Return only the words Orion should say. Do not use tools, inspect files, modify anything,
or claim that a physical action happened."""
MAX_RESPONSE_CHARACTERS = 800


class AgentProvider(Protocol):
    provider: str
    model_name: str

    def respond(self, text: str) -> str: ...
    def close(self) -> None: ...


class ThreadRunner(Protocol):
    def run(self, text: str) -> object: ...


class CodexAgentProvider:
    provider = "codex"

    def __init__(
        self,
        model: str | None = None,
        thread_factory: Callable[[str | None], ThreadRunner] | None = None,
    ) -> None:
        self.model_name = model or "configured-default"
        self._runtime = None
        self._workspace: tempfile.TemporaryDirectory[str] | None = None

        if thread_factory is not None:
            self._thread = thread_factory(model)
            return

        from openai_codex import ApprovalMode, Codex, CodexConfig, Sandbox

        # The SDK ships its own runtime; updating a PATH CLI does not update it.
        runtime = Codex(CodexConfig(codex_bin=os.environ.get("ORION_STUDIO_CODEX_BIN") or None))
        account = runtime.account()
        if account.account is None:
            runtime.close()
            raise RuntimeError("Codex is not signed in. Run `codex login` and restart Studio.")

        self._workspace = tempfile.TemporaryDirectory(prefix="orion-agent-")
        self._runtime = runtime
        self._thread = runtime.thread_start(
            approval_mode=ApprovalMode.deny_all,
            base_instructions=ORION_INSTRUCTIONS,
            cwd=str(Path(self._workspace.name)),
            ephemeral=True,
            model=model,
            sandbox=Sandbox.read_only,
        )

    def respond(self, text: str) -> str:
        prompt = text.strip()
        if not prompt:
            raise ValueError("Agent input cannot be empty.")
        result = self._thread.run(prompt)
        response = getattr(result, "final_response", None)
        if not isinstance(response, str) or not response.strip():
            raise RuntimeError("Codex returned no spoken response.")
        response = " ".join(response.split())
        if len(response) > MAX_RESPONSE_CHARACTERS:
            response = response[:MAX_RESPONSE_CHARACTERS].rsplit(" ", 1)[0].rstrip() + "…"
        return response

    def close(self) -> None:
        if self._runtime is not None:
            self._runtime.close()
            self._runtime = None
        if self._workspace is not None:
            self._workspace.cleanup()
            self._workspace = None
