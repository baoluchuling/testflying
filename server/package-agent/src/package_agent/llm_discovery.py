from __future__ import annotations

import os
import shutil
from collections.abc import Callable
from dataclasses import dataclass


@dataclass(frozen=True)
class LlmAdapter:
    name: str
    executable: str


Which = Callable[[str], str | None]


def discover_llm_adapter(which: Which = shutil.which) -> LlmAdapter | None:
    injected = os.environ.get("TESTFLYING_PACKAGE_AGENT_LLM_PLANNER")
    if injected:
        return LlmAdapter(name="injected_llm_planner", executable=injected)

    for executable, name in (
        ("codex", "codex_cli"),
        ("claude", "claude_cli"),
        ("llm-runtime", "llm_runtime"),
    ):
        resolved = which(executable)
        if resolved:
            return LlmAdapter(name=name, executable=resolved)
    return None
