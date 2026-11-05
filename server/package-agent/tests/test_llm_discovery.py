from __future__ import annotations

from package_agent.llm_discovery import discover_llm_adapter


def test_discovery_prefers_codex_cli() -> None:
    def fake_which(name: str) -> str | None:
        return {
            "codex": "/tmp/codex",
            "claude": "/tmp/claude",
            "llm-runtime": "/tmp/llm-runtime",
        }.get(name)

    adapter = discover_llm_adapter(which=fake_which)

    assert adapter is not None
    assert adapter.name == "codex_cli"


def test_discovery_falls_back_to_claude_cli() -> None:
    def fake_which(name: str) -> str | None:
        return {
            "claude": "/tmp/claude",
            "llm-runtime": "/tmp/llm-runtime",
        }.get(name)

    adapter = discover_llm_adapter(which=fake_which)

    assert adapter is not None
    assert adapter.name == "claude_cli"


def test_discovery_falls_back_to_llm_runtime() -> None:
    def fake_which(name: str) -> str | None:
        return {"llm-runtime": "/tmp/llm-runtime"}.get(name)

    adapter = discover_llm_adapter(which=fake_which)

    assert adapter is not None
    assert adapter.name == "llm_runtime"


def test_discovery_returns_none_when_nothing_available() -> None:
    adapter = discover_llm_adapter(which=lambda _name: None)

    assert adapter is None
