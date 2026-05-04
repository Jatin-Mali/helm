# HELM

> The Rust agent for Linux operators.

HELM is a self-hosted AI agent that gives you natural-language control over your machines. It runs as a small Rust daemon and talks to your shell, filesystem, system services, browser, and remote servers — entirely on your hardware.

## Why HELM

**Linux-first.** Built for people who live in `tmux`. SSH-friendly, headless by default, no desktop app required.

**True root-level control.** No VM sandbox. Capability tokens, source-tainted tool calls, and a tamper-evident audit log keep you in charge without taking your power away.

**Private.** Conversations, files, and skills never leave your machine unless you point them at a remote LLM yourself. Bring your own API key — Anthropic, OpenAI, Gemini, OpenRouter, NVIDIA NIM, Ollama, anything else.

**Open and hackable.** Apache 2.0. One static binary. Skills are code you can read.

**Learns what you do.** Every successful task is logged. Recurring patterns are extracted into reusable, parameterized skills you can review and edit.

## Status

Pre-alpha. v0.1 ships shell, filesystem, system services, and browser tools (via Pinchtab) with an MCP-compatible plugin interface.

## Quick start
curl -fsSL https://helm.sh/install | sh
helm init
helm "find services using more than 500MB and tell me what they do"

## License

Apache 2.0.
