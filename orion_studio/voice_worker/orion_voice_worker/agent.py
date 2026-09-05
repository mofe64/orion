from __future__ import annotations

import os
import shutil
from pathlib import Path
import tempfile
from typing import Callable, Protocol


ORION_INSTRUCTIONS = """You are Orion, a conversational desk-lamp companion.
Answer the user's spoken request in at most two concise sentences suitable for speech.
Return only the words Orion should say. Do not use tools, inspect files, modify anything,
or claim that a physical action happened."""
MAX_RESPONSE_CHARACTERS = 800
DEFAULT_AGENT_MODEL = "gpt-5.6-sol"
DEFAULT_AGENT_EFFORT = "medium"

def runtime_candidates():
    override = os.environ.get("ORION_STUDIO_CODEX_BIN")
    if override:
        return [override]
    candidates = [str(path) for path in (
        Path("/Applications/Codex.app/Contents/Resources/codex"),
        Path("/Applications/ChatGPT.app/Contents/Resources/codex"),
    ) if path.is_file()]
    cli = shutil.which("codex")
    if cli and cli not in candidates:
        candidates.append(cli)
    return [*candidates, None]



class AgentProvider(Protocol):
    provider: str
    model_name: str

    def respond(self, text: str) -> str: ...
    def close(self) -> None: ...


class ThreadRunner(Protocol):
    def run(self, text: str, *, model: str, effort: str) -> object: ...


class CodexAgentProvider:
    provider = "codex"

    def __init__(
        self,
        model: str | None = None,
        thread_factory: Callable[[str | None], ThreadRunner] | None = None,
        *, effort: str = DEFAULT_AGENT_EFFORT,
    ) -> None:
        self.model_name = model or DEFAULT_AGENT_MODEL
        self.effort = effort
        self.runtime_path = "test"
        self.available_models = []
        self._runtime = None
        self._workspace: tempfile.TemporaryDirectory[str] | None = None

        if thread_factory is not None:
            self._thread = thread_factory(model)
            return

        from openai_codex import ApprovalMode, Codex, CodexConfig, Sandbox

        failures = []
        for candidate in runtime_candidates():
            runtime = None
            try:
                runtime = Codex(CodexConfig(codex_bin=candidate))
                if runtime.account().account is None:
                    raise RuntimeError("Codex is not signed in; run codex login")
                catalog = runtime.models().data
                selected = next((item for item in catalog if item.model == self.model_name), None)
                if selected is None or effort not in [item.reasoning_effort for item in selected.supported_reasoning_efforts]:
                    raise RuntimeError(f"Runtime does not advertise {self.model_name} with {effort} effort")
                self.runtime_path = candidate or "SDK bundled runtime"
                self.available_models = [{"model": item.model, "name": item.display_name,
                    "efforts": [option.reasoning_effort for option in item.supported_reasoning_efforts]}
                    for item in catalog if not item.hidden]
                break
            except Exception as error:
                if runtime is not None:
                    runtime.close()
                failures.append(f"{candidate or 'SDK bundled runtime'}: {error}")
        else:
            raise RuntimeError("No compatible Codex runtime. " + "; ".join(failures))

        self._workspace = tempfile.TemporaryDirectory(prefix="orion-agent-")
        self._runtime = runtime
        try:
            self._thread = runtime.thread_start(
                approval_mode=ApprovalMode.deny_all,
                base_instructions=ORION_INSTRUCTIONS,
                cwd=str(Path(self._workspace.name)),
                ephemeral=True,
                model=self.model_name,
                sandbox=Sandbox.read_only,
            )
        except Exception:
            self.close()
            raise

    def respond(self, text: str) -> str:
        prompt = text.strip()
        if not prompt:
            raise ValueError("Agent input cannot be empty.")
        result = self._thread.run(prompt, model=self.model_name, effort=self.effort)
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
