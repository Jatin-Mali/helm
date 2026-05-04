# HELM — Project Promise

This file is a public commitment. It is updated when scope changes; old entries are kept.

## v0.1 — target ship date: 2026-07-27 (12 weeks)

User-visible features:
- `helm "<task>"` runs a natural-language task to completion using an Anthropic API key.
- Three first-class tools: shell execution, filesystem read, filesystem write.
- SQLite-backed episode log of every task.
- ReAct loop with iteration cap and token budget enforcement.
- Apache 2.0 source on GitHub.

Non-goals for v0.1:
- Multi-provider (Anthropic only in v0.1; OpenAI/Ollama in v0.2).
- Browser automation (v0.2).
- Skill learning (v0.2).
- Permission/capability system (v0.2 — v0.1 has a confirm-y/n gate only).
- macOS/Windows.

## History
- 2026-05-04 — v0.1 promise written.
