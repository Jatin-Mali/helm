# HELM

> The Rust agent for Linux operators.

HELM is a self-hosted AI agent that gives you natural-language control over a
Linux machine. It runs from the terminal, talks to your shell, filesystem,
system services, package manager, browser, and provider APIs, and records what
happened in a local SQLite database.

## Why HELM

**Linux-first.** Built for people who live in terminals, SSH, tmux, and servers.

**Real machine control.** HELM can run shell commands, use typed system tools,
read/write files, inspect services, inspect processes, and drive a browser via
PinchTab.

**Permissioned.** Dangerous operations go through capabilities, taint checks,
and an append-only audit chain.

**Provider-flexible.** Use Groq, OpenRouter, Gemini, NVIDIA NIM, Anthropic,
OpenAI-compatible endpoints, or Ollama.

**Terminal UI.** `helm tui` gives a Claude Code / Codex / OpenCode-style
terminal interface with chat, model status, tool timeline, replay, and
permission prompts.

## Quick Start

```sh
export GROQ_API_KEY=gsk_...
cargo run -p helm-cli -- init --force --provider groq --model openai/gpt-oss-20b
cargo run -p helm-cli -- doctor
cargo run -p helm-cli -- "find services using more than 500MB and tell me what they do"
```

Fully local mode:

```sh
ollama pull qwen3:4b
cargo run -p helm-cli -- init --force --provider ollama --model qwen3:4b
```

Interactive terminal UI:

```sh
cargo run -p helm-cli -- tui
```

## Commands

- `helm "<task>"` or `helm run "<task>"`: run an agent task.
- `helm tui`: interactive terminal UI.
- `helm doctor`: provider, memory, tool, model, and quirk checks.
- `helm replay <episode_id>`: inspect a previous run.
- `helm episodes --limit 10`: list recent runs.
- `helm permissions list|grant|revoke`: manage capability grants.
- `helm audit verify|show`: verify or inspect the audit log.
- `helm skills list|show|approve|disable|test`: review local skills.

## Docs

- Provider setup: `docs/providers.md`
- Threat model: `docs/threat-model.md`
- Troubleshooting: `docs/troubleshooting.md`

## Verification Gates

Normal local gate:

```sh
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

Release gates:

```sh
cargo test --workspace --test reliability_suites -- --ignored
GROQ_API_KEY=... OPENROUTER_API_KEY=... GOOGLE_API_KEY=... NVIDIA_API_KEY=... \
  cargo test --workspace --test provider_matrix_live -- --ignored --test-threads=1
```

## Status

Linux-first v1 foundation. GUI desktop control and IoT control are v2.

## License

Apache 2.0.
