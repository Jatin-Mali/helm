# HELMOPS

> A web-first Linux, DevOps, and fleet monitoring console.

HELMOPS is a read-only-first ops console. It collects system context, surfaces
findings that operators miss, shows the evidence, and guides reviewed fixes.
It changes nothing unless you explicitly approve a plan step.

`helmops` with no arguments opens the web dashboard. `helm` remains as a compatibility alias.

## What HELM Is Now

Current default product surface:

- web console: `helmops`
- terminal dashboard: `helmops tui`
- typed system snapshots: `helmops snapshot`
- local monitoring reports: `helmops monitor`
- finding-driven troubleshooting: `helmops troubleshoot`
- reviewed execution only after approval: `helmops apply-plan`

Current shipped core:

- morning-triage dashboard with briefing cards, sidebar filters, finding queue,
  detail pane, and deep host/service/disk/log/backup drill-down
- finding lifecycle states: open, new, recurring, suppressed, resolved,
  self-resolved
- read-only monitor and diagnose flows
- structured evidence with risk, rollback, and exact command previews
- troubleshooting plans with expected output and interpretation guidance
- approved execution with audit trail and change-set history
- local Ollama and API providers
- remote SSH targets with per-target audit
- sessions, snapshots, stored findings, and audit verification

## First Run

```sh
# Configure provider/model once
helmops init

# Open the web dashboard
helmops

# Open the terminal dashboard
helmops tui
```

Web console shortcuts:

- `1-9` switch workspaces
- `Enter` open the selected incident
- `Escape` clear the selection
- `Alt+G` load plan guidance for the selected incident
- `F5` reload the page

### Web Dashboard

```sh
# Start the live web dashboard
helmops web

# Custom port
helmops web --port 8080
```

Open http://localhost:9120 in your browser. The web dashboard provides:

- **Incident-first layout**: a larger queue, richer evidence panel, and guided remediation context
- **Infrastructure coverage**: services, containers, compose, Kubernetes, databases, websites, network, storage, certificates, and fleet
- **Live operator cards**: daemon status, update pressure, storage risk, runtime counts, and collector health
- **Keyboard shortcuts**: `1-9` tabs, `Enter` select, `Escape` clear, `Alt+G` plan, `F5` refresh
- **Single-file frontend**: no JS framework, no build step, live updates over WebSocket

The web dashboard connects via WebSocket to the helm daemon for real-time data.
Start the daemon first for continuous data: `helmops daemon start`

## Safe CLI Flows

```sh
# Collect a typed system snapshot
helmops snapshot --profile standard

# Produce findings without allowing mutation
helmops monitor --domain disk,services,containers,ports,load,logs,backups

# Explain one stored finding in detail
helmops explain <finding-id>

# Build a guided troubleshooting plan
helmops troubleshoot --from-finding <finding-id>

# Verify trust boundaries and active permissions
helmops trust-report
```

## Install

### Release binary

```sh
curl -fsSL https://github.com/Jatin-Mali/helm/releases/latest/download/install.sh | sh
helmops init
helmops
```

If your architecture does not have a published release asset yet, the installer
fails with clear source-build instructions.

### Build from source

```sh
git clone https://github.com/Jatin-Mali/helm.git
cd helm
cargo build --release -p helm-cli
./target/release/helmops init
./target/release/helmops doctor
./target/release/helmops
```

Minimum supported Rust version: `1.85`.

## First Run Setup

`helmops init` asks for:

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
./target/release/helmops init --force --provider openrouter
./target/release/helmops doctor
./target/release/helmops
```

### Gemini

```sh
export GOOGLE_API_KEY='...'
# GEMINI_API_KEY is also accepted
./target/release/helmops init --force --provider gemini --model gemini-2.5-flash
./target/release/helmops doctor
./target/release/helmops
```

### Ollama (local, no API key needed)

```sh
ollama pull qwen3:4b
./target/release/helmops init --force --provider ollama --model qwen3:4b
./target/release/helmops doctor
./target/release/helmops
```

## Core Commands

```sh
helmops                               # open the web dashboard
helmops tui                           # open the terminal dashboard
helmops snapshot --profile standard   # typed system snapshot
helmops monitor                       # findings report from latest snapshot
helmops explain <finding-id>          # show evidence and likely impact
helmops troubleshoot "<problem>"      # build a guided plan
helmops troubleshoot --from-finding <id>
helmops apply-plan <plan-id>          # reviewed execution
helmops change-set list               # recent approved changes
helmops trust-report                  # verify trust boundaries
helmops init                          # configure provider/model/key
helmops doctor                        # verify provider, DB, tools, secrets
helmops models                        # list models for the active provider
helmops audit verify                  # verify audit chain integrity
helmops permissions list              # current capability grants
helmops secrets list                  # stored provider keys
helmops remote list                   # registered SSH targets
helmops bootstrap user@host --register-as prod-1
helmops web --port 9120               # explicit web launch
helmops daemon start                  # background collector loop
helmops completion bash               # shell completion
```

## Security Notes

- Stored provider keys live in `$XDG_CONFIG_HOME/helm/secrets.toml` (mode
  `0600`). Environment variables stay ephemeral unless you explicitly save them.
- The TUI does not auto-import env keys into the secrets store.
- Dashboard, monitor, snapshot, and diagnose flows are read-only by default.
- Read-only diagnose mode blocks all write tools and mutating sub-actions.
- Dangerous tools require explicit capability grants.
- HELM keeps local config, audit, session, and episode state under XDG helm
  directories. Treat those directories as sensitive local state.
- API models mean prompts leave your machine. Local Ollama models mean local
  inference. The trust-report and dashboard status bar make this boundary clear.

## Remote Mode

Remote targets use SSH for read-only inspection and, when explicitly approved,
execution. The local client starts the remote `helmops` binary and replays
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
- `docs/detector-review-checklist.md`
- `docs/helmops-gap-closure-checklist.md`
- `docs/troubleshooting.md`
- `docs/release-notes-v1.0.md`
- `CONTRIBUTING.md`

## License

Apache-2.0.
