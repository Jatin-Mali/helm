# HELM

> A terminal-native Linux monitoring and troubleshooting assistant.

HELM is a read-only-first ops console. It collects system context, surfaces
findings that operators miss, shows the evidence, and guides reviewed fixes.
It changes nothing unless you explicitly approve a plan step.

`helm` with no arguments opens the dashboard.

## What HELM Is Now

Current default product surface:

- dashboard-first TUI: `helm`
- typed system snapshots: `helm snapshot`
- local monitoring reports: `helm monitor`
- finding-driven troubleshooting: `helm troubleshoot`
- reviewed execution only after approval: `helm apply-plan`

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
helm init

# Open the monitoring dashboard
helm
```

Dashboard shortcuts:

- `F5` refresh system state
- `Tab` move between filters, queue, and detail
- `1` / `2` / `3` switch Review, Cleanup, and Remediate workflow tabs
- `Enter` open the selected finding
- `Alt+E` open evidence
- `Alt+F` run a read-only follow-up check for the selected finding
- `Alt+G` generate a troubleshooting plan
- `Alt+A` open the reviewed apply flow for the active plan
- `Alt+S` suppress a finding
- `Alt+R` mark a finding resolved
- `Alt+U` reopen a suppressed or resolved finding
- `Shift+Tab` cycle Dashboard → Chat → Plan → Diagnose

## Safe CLI Flows

```sh
# Collect a typed system snapshot
helm snapshot --profile standard

# Produce findings without allowing mutation
helm monitor --domain disk,services,containers,ports,load,logs,backups

# Explain one stored finding in detail
helm explain <finding-id>

# Build a guided troubleshooting plan
helm troubleshoot --from-finding <finding-id>

# Verify trust boundaries and active permissions
helm trust-report
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
./target/release/helm
```

### Gemini

```sh
export GOOGLE_API_KEY='...'
# GEMINI_API_KEY is also accepted
./target/release/helm init --force --provider gemini --model gemini-2.5-flash
./target/release/helm doctor
./target/release/helm
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
helm                                  # open the dashboard
helm tui --mode dashboard             # explicit dashboard launch
helm tui --mode chat                  # chat-first TUI
helm tui --mode diagnose              # read-only diagnose TUI
helm snapshot --profile standard      # typed system snapshot
helm monitor                          # findings report from latest snapshot
helm monitor --watch --interval 60s   # read-only local watch loop
helm explain <finding-id>             # show evidence and likely impact
helm troubleshoot "<problem>"         # build a guided plan
helm troubleshoot --from-finding <id> # finding-driven plan
helm apply-plan <plan-id>             # reviewed execution
helm change-set list                  # recent approved changes
helm diagnose "<question>"            # read-only system inspection
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
helm monitor --remote prod-1
helm config path                      # config file location
helm completion bash                  # shell completion
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
- `docs/detector-review-checklist.md`
- `docs/troubleshooting.md`
- `docs/release-notes-v1.0.md`
- `CONTRIBUTING.md`

## License

Apache-2.0.
