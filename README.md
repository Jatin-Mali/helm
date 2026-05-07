# HELM

> The Rust agent for Linux operators.

HELM is a self-hosted AI agent that gives you natural-language control over a Linux machine. It runs entirely on your machine, talks to your shell, filesystem, services, package manager, browser, and LLM provider of your choice — and records every action in a local SQLite audit log.

---

## Install

```sh
curl -fsSL https://github.com/white-phantom/helm/releases/latest/download/install.sh | sh
```

Supports x86\_64 and ARM64. Installs to `~/.local/bin/helm` and adds it to your PATH automatically.

---

## First Run

```sh
helm init
```

Interactive setup — runs once, takes 60 seconds:

```
HELM Setup Wizard
=================

Choose a provider:
  1) Anthropic (Claude)
  2) Groq
  3) Google Gemini
  4) OpenRouter
  5) NVIDIA NIM
  6) OpenAI-compatible endpoint
  7) Ollama (local, no API key)

Provider [2]: 2

Get your free Groq API key at: https://console.groq.com/keys
Paste API key: gsk_...

Model [llama-3.3-70b-versatile]: (press Enter to use default)

Config written to ~/.helm/config.toml
Database created at ~/.helm/helm.db

  helm "show me what's eating my disk"
  helm tui
  helm doctor
```

Run `helm doctor` to verify everything is wired up.

---

## Commands

```sh
helm "<task>"                          # run a one-shot task
helm tui                               # interactive terminal UI
helm init                              # re-run setup (change provider/model/key)
helm doctor                            # health check: provider, DB, tools, quirks
helm episodes --limit 10               # list recent runs
helm replay <episode_id>               # inspect a previous run step by step
helm audit verify                      # verify the HMAC audit chain
helm audit show                        # tail the audit log
helm skills list|show|approve|test     # manage the skills library
helm permissions list|grant|revoke     # manage capability grants
helm models                            # list available models for active provider
```

---

## Providers

| Provider | Free tier | Key env var |
|----------|-----------|-------------|
| Groq | Yes (fast) | `GROQ_API_KEY` |
| Google Gemini | Yes | `GEMINI_API_KEY` |
| OpenRouter | Yes (pay-per-token) | `OPENROUTER_API_KEY` |
| Anthropic | No | `ANTHROPIC_API_KEY` |
| NVIDIA NIM | Yes (limited) | `NVIDIA_API_KEY` |
| Ollama | Local, free | — |
| OpenAI-compat | Varies | varies |

Keys set in `helm init` are stored in `~/.helm/config.toml`. Environment variables always take priority.

---

## What HELM Can Do

| Area | Tools |
|------|-------|
| Shell | `shell` — run any command |
| Filesystem | `fs_read`, `fs_write` — read/write files |
| Processes | `process` — list, inspect, kill, renice |
| Services | `service` — systemctl start/stop/status/journal |
| Packages | `package` — apt/dnf/pacman (auto-detected) |
| Disk | `disk` — df, du, lsblk, SMART |
| Networking | `network` — ip, routes, DNS, curl |
| Logs | `logs` — journalctl, tail, grep |
| Browser | `browser` — PinchTab-driven web browsing |

Dangerous operations require explicit capability grants (prompted at runtime). Every action is logged in an append-only HMAC-chained audit log.

---

## Security Model

HELM enforces five layers before any destructive action:

1. **Capability gate** — each tool declares required capabilities; checked before every call
2. **Taint tracking** — external content (browser, SSH, MCP) is tagged `External`; cannot escalate to `*.write` operations
3. **Confirmation prompts** — `*.write` and `ShellExec` prompt the user on first use per session
4. **Audit log** — append-only, HMAC-chained; `helm audit verify` checks integrity
5. **Evidence verifier** — Supervisor DAG verifies post-conditions after each step

Full threat model: `docs/threat-model.md`

---

## Build from Source

```sh
git clone https://github.com/white-phantom/helm
cd helm
cargo build --release
./target/release/helm init
```

Requires Rust 1.78+.

---

## Docs

- `docs/providers.md` — provider config and model IDs
- `docs/threat-model.md` — full security model
- `docs/troubleshooting.md` — common errors and fixes
- `CONTRIBUTING.md` — how to contribute

---

## License

Apache 2.0.
