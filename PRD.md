# HELM Product Requirements Document
# Monitoring-First DevOps Assistant
# Canonical revision: 2026-05-13

## 1. Executive Summary

HELM is a local-first DevOps assistant for Linux operators. It monitors system
state, finds operational issues, builds system context, and guides the user
through troubleshooting. It can suggest commands and later execute approved
changes, but execution is not the primary value proposition.

The product should be useful even when it has zero write permissions.

One-sentence positioning:

HELM is a read-only-first DevOps assistant that finds missed operational risks
and guides safe troubleshooting with evidence-backed commands.

## 2. User Research Signal

Recent feedback changes the product direction:

- Users do not want broad AI automation on servers.
- Users may trust a read-only tool that reports issues and suggests commands.
- Day-one useful checks are disk health, running services, containers, open
  ports, system load, logs, and backups.
- Users want system context before solutions.
- Users want exact commands plus a plain explanation of what those commands
  will do to their system.
- Users want permission prompts before any change.
- Users are skeptical of tools that behave like junior engineers: confident,
  rushed, and under-informed.

Conclusion:

HELM must become a monitoring and guided troubleshooting product first. Approved
automation becomes a later capability.

## 3. Product Goals

Primary goals:

- Detect operational issues that users often miss.
- Build a complete, typed system snapshot before reasoning.
- Provide structured, evidence-backed troubleshooting guidance.
- Suggest commands with exact effects, risks, and rollback notes.
- Execute changes only after explicit user approval.
- Work locally by default and disclose API data boundaries clearly.

Secondary goals:

- Generate runbooks and handoff summaries from live state.
- Track baselines and drift over time.
- Support remote read-only monitoring.
- Support approved change execution after trust is established.

Non-goals:

- Full autopilot.
- SaaS-first control plane.
- A chart-heavy Netdata clone.
- A generic Claude wrapper.
- Multi-host writes as a headline feature.
- Any workflow that hides commands before execution.

## 4. Target Users

Primary users:

- self-hosters running Docker, Compose, Proxmox, nginx, databases, and backups
- solo Linux operators maintaining one to ten hosts
- small DevOps teams with stale runbooks and repeated handoff work
- cautious SREs who may accept read-only incident context first

Non-target users for now:

- high-compliance enterprises needing centralized policy integrations
- users who want zero terminal interaction
- users who want unsupervised repair
- teams already satisfied with full observability stacks and internal runbooks

## 5. Market Position

HELM is different from Beszel and Netdata:

- Beszel and Netdata answer "what is the metric over time?"
- HELM answers "what is wrong, what did we inspect, what might be causing it,
  what should I check next, and what would this command do?"

HELM is different from Claude, Hermes, and generic agents:

- It starts from typed system context, not chat.
- It has read-only monitoring as a first-class mode.
- It gives evidence, blast radius, and rollback before commands.
- It keeps audit and local state.
- It blocks mutation unless the user approves.

HELM's moat:

- context graph of the local system
- typed collectors and detectors
- missed-ops finding library
- guided troubleshooting plans
- exact command previews tied to system evidence
- local-first storage, audit, and rollback
- remote read-only monitoring without a SaaS dependency

## 6. Current Product State

Current shipped foundation:

- CLI and TUI
- dashboard-first default entrypoint
- morning-triage dashboard layout with filters, queue, and detail pane
- provider support
- local Ollama path
- read-only diagnose mode
- evidence reports
- trust-report
- typed tools for disk, logs, services, processes, network, files, search,
  HTTP, git, shell, package, and remote
- capability grants
- source taint
- audit chain
- sessions and snapshots
- remote registry
- sandbox support
- hooks and skills

Current product gap:

- Monitoring is now the main UX, but backup confidence, fleet monitoring, and
  deeper self-hosted integrations are still ahead.
- The dashboard must remain the default product surface as later features land.
- Evidence quality, read-only follow-up checks, and command previews must stay
  ahead of any new automation features.
- Dashboard and CLI flows must continue to expose the local-vs-API provider
  boundary clearly.

## 7. Core Workflows

### 7.1 First Safe Run

Command:

```sh
helm
```

Expected behavior:

- opens the dashboard
- loads the latest snapshot or asks the user to collect one
- shows a triage queue with lifecycle state and sample evidence
- shows evidence and drill-down paths for each issue
- keeps all dashboard actions read-only until the user opens an apply flow

Acceptance criteria:

- no write capability is registered
- no mutation is possible
- report is useful without provider API keys
- findings cite specific snapshot fields

### 7.2 User-Guided Troubleshooting

Command:

```sh
helm troubleshoot "site is slow"
```

Expected behavior:

- collects relevant full context first
- builds hypotheses
- shows evidence for and against each hypothesis
- asks user before any mutating command
- shows command effects, risk, blast radius, rollback, and verification

Acceptance criteria:

- no fix command is proposed before context collection
- every command preview includes expected effect
- every mutating command requires approval
- denied steps leave zero state changes

### 7.3 Finding-Based Troubleshooting

Command:

```sh
helm troubleshoot --from-finding <id>
```

Expected behavior:

- uses a monitor finding as the entry point
- expands context around the affected resource
- creates a focused troubleshooting plan

Acceptance criteria:

- finding ID resolves to stored evidence
- plan cites the original finding
- unrelated system areas are not over-scanned unless needed

### 7.4 Approved Fix

Command:

```sh
helm apply-plan <id>
```

Expected behavior:

- shows exact commands again
- explains effect on this host
- requests approval per risky step
- executes approved steps
- verifies outcome
- records audit and change-set

Acceptance criteria:

- user can deny any step
- unsupported rollback is clearly labeled
- post-change verification is mandatory
- audit chain remains valid

### 7.5 Runbook And Handoff

Commands:

```sh
helm runbook generate
helm handoff
```

Expected behavior:

- generate Markdown from system snapshot
- summarize services, ports, containers, backups, recent findings, and common
  recovery commands

Acceptance criteria:

- read-only only
- no secrets in output
- docs include timestamp and host identity

## 8. Monitoring Domains

MVP domains:

- disk health
- services
- containers
- open ports
- system load
- logs
- backups

Expanded domains:

- certificates
- package updates
- firewall
- DNS
- cron and systemd timers
- config drift
- database health
- Proxmox
- Grafana and InfluxDB
- SSH exposure
- security posture hints

Each domain must provide:

- collector
- typed snapshot struct
- detector set
- finding examples
- read-only follow-up commands
- fix-plan templates where appropriate
- tests with fixtures

## 9. Finding Model

Every finding must include:

- stable ID
- title
- severity
- confidence
- affected resource
- category
- evidence
- likely impact
- assumptions
- missing data
- suggested read-only checks
- optional fix plan reference
- source snapshot ID

Severity levels:

- info
- warning
- critical

Confidence levels:

- low
- medium
- high

## 10. Hard Product Rules

- Read-only is the default trust path.
- Monitor mode never mutates.
- Troubleshoot mode collects context before proposing fixes.
- No static keyword matching as primary diagnosis.
- No command executes without preview and permission.
- Every mutating command explains effect, risk, rollback, and verification.
- Every approved change is audited.
- Every persisted field is redacted.
- API model boundaries are always disclosed.
- Local model path must remain supported.
- Missing data is shown as missing, not guessed.

## 11. Success Metrics

Adoption metrics:

- successful first `helm monitor` run
- time from install to first useful finding
- percentage of users who run read-only twice
- diagnose-to-troubleshoot conversion

Trust metrics:

- number of denied plans
- number of approved plans
- rollback success rate
- trust-report usage
- user-reported false positives
- user-reported scary or surprising behavior

Product metrics:

- findings per domain
- false-positive rate by detector
- average snapshot duration
- average monitor report duration
- backup-check coverage
- troubleshooting plan completion rate

## 12. Release Requirements

Every release must include:

- updated docs
- safe examples first
- threat-model review if boundaries changed
- full release gate
- no stale fork identity
- no plaintext provider keys in config
- no mutation in monitor mode
- no hidden command execution

Release gate:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build --release -p helm-cli
rg -n "white-phantom|github.com/helm|helm.sh/install" .
```

## 13. Version Plan

v1.6:

- trust foundation, real diagnose, evidence, trust-report

v1.7:

- system snapshot engine

v1.8:

- issue detection and `helm monitor`

v1.9:

- guided troubleshooting

v2.0:

- approved change execution and change sets

v2.1:

- local TUI monitoring dashboard

v2.2:

- backup confidence and recovery checks

v2.3:

- remote and fleet read-only monitoring

v2.4:

- self-hosted integrations

v2.5:

- second-opinion reviewer and governed automation
