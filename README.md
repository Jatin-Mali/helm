# HELM

> The Rust agent for Linux operators.

HELM is a local AI agent for Linux systems work. It can inspect files, run
shell commands, manage services, look at logs, inspect disk usage, and keep a
local audit trail of what it did.

`helm` with no arguments opens the TUI.

## What Ships In v1.6

- Local TUI and one-shot CLI task execution
- Provider/model switching with live provider catalogs where supported
- Built-in Linux ops tools plus executable skills
- Session history, snapshots, undo/redo, and audit verification
- SSH remote targets, bootstrap, remote agent execution, and TUI attach mode
- Optional OpenTelemetry export
- Trust ladder: Diagnose → Dry-Run → Local → Remote → Governed
- `helm diagnose` — read-only mode with 9 safe tools, enforced at registration
- `helm run --dry-run` — prints commands, executes nothing
- `helm trust-report` — audit chain, grants, sandbox, diagnose summary
- TUI modes: Chat → Plan → AutoAccept → Diagnose (Shift+Tab cycles)

## Install

### Release binary

Current tagged releases publish an x86_64 Linux binary plus the installer.

```sh
curl -fsSL https://github.com/Jatin-Mali/helm/releases/latest/download/install.sh | sh
helm init
helm
```

If your architecture does not have a published release asset yet, the installer
fails with source-build instructions instead of a silent 404.

### Build from source

Use this path on ARM64, on forks without release assets, or if you want to test
exact source locally.

```sh
git clone https://github.com/Jatin-Mali/helm.git
cd helm
cargo build --release -p helm-cli
./target/release/helm init
./target/release/helm doctor
./target/release/helm
```

Minimum supported Rust version: `1.85`.

## First Run

`helm init` asks for:

1. provider
2. model
3. API key if the provider needs one
4. optional crash-report consent

Stored keys go into `$XDG_CONFIG_HOME/helm/secrets.toml` (or
`~/.config/helm/secrets.toml`) with mode `0600`. `config.toml` stores
provider/model settings, not the raw key.

HELM now follows XDG paths:

- Config: `$XDG_CONFIG_HOME/helm/` or `~/.config/helm/`
- Data: `$XDG_DATA_HOME/helm/` or `~/.local/share/helm/`
- Cache: `$XDG_CACHE_HOME/helm/` or `~/.cache/helm/`

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

### Ollama

```sh
ollama pull qwen3:4b
./target/release/helm init --force --provider ollama --model qwen3:4b
./target/release/helm doctor
./target/release/helm
```

## Core Commands

```sh
helm                                  # open the TUI
helm "<task>"                         # run one task
helm run "<task>"                     # explicit one-shot mode
helm init                             # configure provider/model/key
helm doctor                           # verify provider, DB, tools, secrets store
helm models                           # list models for the active provider
helm episodes --limit 10              # recent runs
helm replay <episode_id>              # replay a run
helm audit verify                     # verify audit chain
helm permissions list                 # current grants
helm secrets list                     # stored provider keys
helm skills list                      # built-in and user skills
helm skills run git-status --dry-run  # inspect a skill before running it
helm remote list                      # registered SSH targets
helm bootstrap user@host --register-as prod-1
helm run --remote prod-1 "check nginx and journal errors"
helm serve --bind 127.0.0.1:8765
helm tui --attach 127.0.0.1:8765 --token "$HELM_REMOTE_TOKEN"
helm config path                      # config file location
helm completion bash                  # shell completion
```

## Safe Examples

Try HELM without granting write access:

```sh
# Diagnose mode — read-only, 9 safe tools
helm diagnose "why is /var/log filling up?"

# Dry-run — see what would happen without executing
helm run --dry-run "clean up old kernel packages"

# Trust report — verify audit chain and active grants
helm trust-report

# TUI in read-only plan mode
helm tui --read-only

# TUI with dry-run (no tool execution)
helm tui --dry-run
```

## Security Notes

- Stored provider keys live in `$XDG_CONFIG_HOME/helm/secrets.toml` (or `~/.config/helm/secrets.toml`) with mode `0600`.
- Environment variables stay ephemeral unless you explicitly import or save
  them. The TUI does not auto-import env keys into the secrets store anymore.
- Dangerous tools still require capability grants unless you run with explicit
  override flags such as `--yes`.
- HELM keeps local config, audit, session, and episode state under the XDG
  helm directories. Treat those directories as sensitive local state.

## Remote Mode

Remote agent execution in v1.5 uses NDJSON streamed over SSH. The local client
starts the remote `helm` binary, then replays remote `AgentEvent`s into the
local CLI or TUI. See:

- `docs/agent-on-remote.md`

## Use HELM If / Use Something Else If

| Use HELM if... | Use something else if... |
|---|---|
| You want a local Linux operations agent | You want a cloud-hosted managed agent |
| You need shell, services, logs, packages, and disk inspection in one tool | You need team collaboration and SaaS workflows |
| You want local audit and episode history | You do not want any local state at all |
| You prefer a terminal-first interface | You want a browser-only product |

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
