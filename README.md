# HELM

> A read-only-first DevOps assistant for Linux operators. Finds missed
> operational risks and guides safe troubleshooting with evidence-backed
> commands.

HELM observes your system, finds issues operators often miss, explains the
evidence, and suggests reviewed commands. It changes nothing without your
permission.

`helm` with no arguments opens the TUI.

## What Ships In v1.6

- `helm diagnose` — read-only inspection with 9 typed tools, enforced at
  registration
- `helm run --dry-run` — prints planned commands, executes nothing
- `helm run --evidence` — structured evidence reports before risky actions
- `helm trust-report` — provider boundary, secrets status, permissions, audit,
  sandbox, and diagnose safety
- `/diagnose` and `/evidence` slash commands in the TUI
- Local Ollama and API providers (Anthropic, Gemini, Groq, OpenRouter, Nvidia)
- Remote SSH targets with per-target audit
- Session history, snapshots, and audit verification
- Skills and hooks as advanced integrations
- TUI modes: Chat → Plan → Diagnose (Shift+Tab cycles)

## First Safe Run

```sh
# Inspect the system without granting write access
helm diagnose "why is /var/log filling up?"

# See what HELM would do before executing anything
helm run --dry-run "clean up old kernel packages"

# Verify trust boundaries and active permissions
helm trust-report

# Open the TUI in read-only diagnose mode
helm tui --mode diagnose
```

## Install

### Release binary

```sh
curl -fsSL https://github.com/Jatin-Mali/helm/releases/latest/download/install.sh | sh
helm init
helm
```

If your architecture does not have a published release asset yet, the installer
fails with clear source-build instructions.

### Build from source

```sh
git clone https://github.com/Jatin-Mali/helm.git
cd helm
cargo build --release -p helm-cli
./target/release/helm init
./target/release/helm doctor
./target/release/helm
```

Minimum supported Rust version: `1.85`.

## First Run Setup

`helm init` asks for:

1. provider
2. model
3. API key if the provider needs one
4. optional crash-report consent

Keys go into `$XDG_CONFIG_HOME/helm/secrets.toml` (or
`~/.config/helm/secrets.toml`) with mode `0600`. `config.toml` stores provider
and model settings, not the raw key.

HELM follows XDG paths:

| Type | Path |
|------|------|
| Config | `$XDG_CONFIG_HOME/helm/` or `~/.config/helm/` |
| Data | `$XDG_DATA_HOME/helm/` or `~/.local/share/helm/` |
| Cache | `$XDG_CACHE_HOME/helm/` or `~/.cache/helm/` |

## Provider Examples

### OpenRouter

```sh
export OPENROUTER_API_KEY='sk-or-...'
./target/release/helm init --force --provider openrouter
./target/release/helm doctor
./target/release/helm "Reply with exactly: ok"
```

### Gemini

```sh
export GOOGLE_API_KEY='...'
# GEMINI_API_KEY is also accepted
./target/release/helm init --force --provider gemini --model gemini-2.5-flash
./target/release/helm doctor
./target/release/helm "Reply with exactly: ok"
```

### Ollama (local, no API key needed)

```sh
ollama pull qwen3:4b
./target/release/helm init --force --provider ollama --model qwen3:4b
./target/release/helm doctor
./target/release/helm
```

## Core Commands

```sh
helm                                  # open the TUI
helm diagnose "<question>"            # read-only system inspection
helm run "<task>"                     # one-shot agent task
helm run --dry-run "<task>"           # preview without execution
helm run --evidence "<task>"          # emit evidence before risky actions
helm trust-report                     # verify trust boundaries
helm init                             # configure provider/model/key
helm doctor                           # verify provider, DB, tools, secrets
helm models                           # list models for the active provider
helm episodes --limit 10              # recent runs
helm replay <episode_id>              # replay a run
helm audit verify                     # verify audit chain integrity
helm permissions list                 # current capability grants
helm secrets list                     # stored provider keys
helm skills list                      # built-in and user skills
helm remote list                      # registered SSH targets
helm bootstrap user@host --register-as prod-1
helm run --remote prod-1 "check nginx and journal errors"
helm config path                      # config file location
helm completion bash                  # shell completion
```

## Security Notes

- Stored provider keys live in `$XDG_CONFIG_HOME/helm/secrets.toml` (mode
  `0600`). Environment variables stay ephemeral unless you explicitly save them.
- The TUI does not auto-import env keys into the secrets store.
- Read-only diagnose mode blocks all write tools and mutating sub-actions.
- Dangerous tools require explicit capability grants.
- HELM keeps local config, audit, session, and episode state under XDG helm
  directories. Treat those directories as sensitive local state.
- API models mean prompts leave your machine. Local Ollama models mean local
  inference. The trust-report and TUI provider display make this boundary clear.

## Remote Mode

Remote targets use SSH for read-only inspection and, when explicitly approved,
execution. The local client starts the remote `helm` binary and replays
`AgentEvent` results into the local CLI or TUI. See `docs/agent-on-remote.md`.

## Use HELM If

| Use HELM if... | Use something else if... |
|---|---|
| You want read-only system inspection with evidence | You want a cloud-hosted managed agent |
| You need typed disk, service, log, and process tools | You need team collaboration and SaaS workflows |
| You want local audit, session history, and rollback data | You do not want any local state at all |
| You prefer a terminal-first interface | You want a browser-only product |
| You want to review every command before it runs | You want unsupervised repair |

## Docs

- `docs/providers.md`
- `docs/agent-on-remote.md`
- `docs/threat-model.md`
- `docs/trust-ladder.md`
- `docs/data-boundary.md`
- `docs/troubleshooting.md`
- `docs/release-notes-v1.0.md`
- `CONTRIBUTING.md`

## License

Apache-2.0.
