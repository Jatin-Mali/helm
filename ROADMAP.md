# HELM Roadmap - Monitoring-First DevOps Assistant
# Canonical revision: 2026-05-13

## Product Thesis

HELM is becoming a read-only-first DevOps assistant for Linux systems. Its
primary job is not automation. Its primary job is to understand system context,
surface issues that operators miss, explain the evidence, and guide the user
through troubleshooting with reviewed commands.

The product must feel safe on day one:

- It observes before it reasons.
- It reasons before it suggests.
- It suggests before it asks for permission.
- It explains exactly what approved commands will do to this specific system.
- It changes nothing without explicit permission.

The new center of gravity is monitoring plus guided troubleshooting. Automation
is a later execution layer, not the core product identity.

## Current Position

The existing codebase already contains a useful foundation:

- CLI and TUI entrypoints in `helm-cli`
- typed Linux tools in `crates/helm-tools`
- read-only diagnose mode from v1.6
- structured evidence reports
- capability gates and taint model
- audit chain and session memory
- remote target registry and remote audit
- provider routing, local Ollama support, and API providers
- sandbox support
- hooks, skills, MCP, and custom commands

What is wrong with the old direction:

- It still smells like "AI with shell access" to the market.
- It asks users to trust execution too early.
- It leads with automation features while comments clearly ask for safe
  visibility, context, and troubleshooting.
- It competes mentally with Claude, Hermes, Beszel, Netdata, and shell scripts
  without a clear wedge.
- It treats monitoring as a feature, but users are telling us monitoring and
  guided diagnosis are the wedge.

## Final Product State

HELM should feel like this:

```text
helm monitor
  Collects a local system snapshot:
  disk health, filesystems, inodes, SMART, services, containers, ports,
  load, memory, logs, backups, timers, certificates, package updates,
  firewall exposure, recent changes, and known risks.

  Outputs:
  issue list, severity, evidence, likely causes, confidence, missing data,
  suggested read-only follow-up checks, and safe next commands.
```

```text
helm troubleshoot "nginx is slow"
  First collects full context relevant to nginx:
  service state, recent journal entries, nginx config paths, listeners,
  disk, memory, CPU, container state if applicable, certs, DNS, firewall,
  recent package or config changes.

  Then produces:
  hypothesis tree, evidence for/against each hypothesis, commands to verify,
  commands to fix, blast radius, rollback status, and permission prompt.
```

```text
helm apply-plan <id>
  Shows exact commands again.
  Explains what each command will change on this system.
  Requires explicit approval.
  Executes only approved steps.
  Verifies outcome after execution.
  Records audit and rollback metadata.
```

## Market Position

HELM is not Beszel and not Netdata.

Beszel and Netdata are metric dashboards. They are good at continuous charts,
resource graphs, and alerts. HELM should be different:

- HELM understands system context across tools, files, logs, services, and
  history.
- HELM produces an operator-ready explanation, not just a graph.
- HELM finds neglected operational risks: old backups, failing timers, stale
  certs, restart loops, open ports, inode pressure, disk health warnings,
  config drift, broken restore assumptions, failed units, and recent changes.
- HELM creates guided troubleshooting plans with exact commands and review
  prompts.
- HELM can execute reviewed fixes later, but only after permission and with
  audit.

The core moat is context-aware troubleshooting, not charts.

## Product Principles

These rules apply to every version from this point onward.

1. Read-only first.
   Default value must come from inspection and explanation, not mutation.

2. No silent changes.
   File writes, package operations, service actions, process kills, remote
   mutations, and shell mutations require explicit approval.

3. No static keyword matching as primary diagnosis.
   Static strings may support known detectors, but every finding must be backed
   by typed observations, source references, confidence, and reason codes.

4. Full context before solution.
   HELM must collect the relevant system snapshot before proposing fixes.

5. Evidence before recommendation.
   Recommendations must include inspected sources, findings, assumptions,
   missing data, confidence, and blast radius.

6. Commands are reviewed artifacts.
   Every suggested command must say what it will do, why it is suggested, what
   can go wrong, and what rollback exists.

7. Production-safe by default.
   No auto-accept, no YOLO-first messaging, no broad fleet mutation as a lead
   feature.

8. Honest data boundaries.
   API models mean prompts leave the machine. Local models mean local inference.
   The product must say this clearly.

9. Idempotency over cleverness.
   Suggested actions should check current state and avoid compounding changes
   when repeated.

10. Audit everything important.
    Findings, plans, approvals, denials, commands, outputs, and verification
    results must be traceable.

## What To Keep

- CLI and TUI
- typed tools: disk, logs, process, service, network, fs_read, search, http,
  git, package, shell
- diagnose mode
- evidence reports
- capability gates
- taint and redaction
- audit chain
- sessions and snapshots
- remote target registry
- local Ollama and API providers
- skills and MCP as advanced integrations
- sandbox and allowlists

## What To Strip Down Or De-Emphasize

- YOLO mode: keep for development only, hide from normal docs.
- broad automation language: remove from README and future posts.
- remote patching as a headline: move behind monitored context and approval.
- multi-agent autonomy: reframe later as second-opinion verification.
- skills marketplace thinking: defer until core monitoring is trusted.
- generic chat-first UX: make monitor and troubleshoot the first-class flows.
- command generation without context: disallow in product behavior.

## What To Add

### Core Monitoring Domains

Day-one monitoring must cover:

- disk health: filesystem usage, inodes, SMART where available, read-only
  mounts, mount errors, largest growth candidates, deleted-open files
- services: failed units, restart loops, enabled-but-inactive services, recent
  journal errors, timers
- containers: Docker/Podman/Compose status, restart counts, health checks,
  image age, exposed ports, volume mounts, recent logs
- open ports: listening sockets, owning processes, public exposure hints,
  firewall mismatch
- system load: CPU, memory, swap, pressure stall information, IO wait, OOM
  killer traces
- logs: journal anomalies, kernel errors, auth failures, service-specific
  failures, rate spikes
- backups: backup tool detection, latest backup age, schedule status, restore
  test evidence, excluded critical paths

Additional monitoring domains:

- certificates and TLS expiry
- package updates and security update availability
- DNS and network route sanity
- cron jobs and systemd timers
- config drift and recent file modifications
- filesystem permissions on sensitive files
- SSH exposure and auth posture
- database health probes for common self-hosted stacks
- Proxmox and virtualization summaries
- Grafana/InfluxDB health checks

### New Core Objects

- `SystemSnapshot`: typed inventory of system state at a time.
- `Finding`: one issue, risk, anomaly, or useful observation.
- `Detector`: deterministic analyzer that converts snapshots into findings.
- `Hypothesis`: possible cause of a user-reported problem.
- `TroubleshootingPlan`: ordered read-only checks and optional fix commands.
- `CommandPreview`: exact command, expected effect, risk, rollback.
- `ApprovalRequest`: permission prompt tied to a plan step.
- `ChangeSet`: approved executed mutation with audit and rollback metadata.
- `Baseline`: previous known-good snapshot for drift comparison.
- `MonitorProfile`: what domains to check and how deep to inspect.

## Roadmap

### v1.6 - Trust Foundation

Status: shipped in the current working tree, pending release packaging.

Purpose:

- Make HELM safe to try.
- Provide real read-only diagnose.
- Show structured evidence before risky actions.
- Report trust boundaries.

Deliverables:

- `helm diagnose "<question>"`
- `--dry-run`
- `--evidence`
- `/diagnose` and `/evidence` in TUI
- `helm trust-report`
- diagnose-mode mutation gates
- structured evidence reports
- trust ladder and data-boundary docs

Verification:

- `helm diagnose` executes real read-only tools, not dry-run stubs.
- diagnose mode blocks write tools and mutating sub-actions.
- `trust-report` reports provider, secrets, permissions, sandbox, audit, paths,
  and diagnose safety.
- full gate passes:
  - `cargo fmt --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace --all-targets`
  - `cargo build --release -p helm-cli`

### v1.7 - System Snapshot Engine

Purpose:

- Build the non-LLM observability core.
- Collect whole-system context before the model reasons.
- Make monitoring useful without provider keys.

Deliverables:

- `helm snapshot`
- `helm snapshot --json`
- `helm snapshot --profile quick|standard|deep`
- typed `SystemSnapshot` model
- collectors for:
  - host identity and OS
  - CPU, memory, swap, load, PSI
  - disks, mounts, inodes, SMART availability
  - systemd services and timers
  - journal summary
  - processes
  - network listeners and routes
  - firewall hints
  - Docker/Podman/Compose if installed
  - backup tool detection
  - package update status
- snapshot persistence in SQLite
- redacted JSON export
- snapshot diff against previous run

Hard rules:

- Collectors are typed and bounded.
- No shell pipeline is allowed where a typed parser exists.
- No static keyword matching as the primary detector.
- Snapshot collection must succeed partially if one domain fails.
- Snapshot output must never persist secrets or sensitive HELM paths.

Verification:

- unit tests for every collector parser
- integration test with mocked command outputs
- `helm snapshot --json` schema snapshot test
- no mutation in snapshot mode
- bounded timeout per collector

### v1.8 - Issue Detection And Monitor Report

Purpose:

- Turn snapshots into useful findings.
- Make HELM a monitoring tool, not just a diagnostic chat command.

Deliverables:

- `helm monitor`
- `helm monitor --json`
- `helm monitor --domain disk,services,containers,ports,load,logs,backups`
- `helm monitor --watch --interval 60s` read-only local loop
- severity model: info, warning, critical
- confidence model: low, medium, high
- finding model:
  - title
  - severity
  - confidence
  - affected resource
  - evidence
  - likely impact
  - suggested read-only checks
  - optional fix plan reference
- detectors for day-one domains:
  - disk usage and inodes
  - SMART warnings when available
  - failed services
  - restart loops
  - unhealthy containers
  - unexpected open ports
  - high load and memory pressure
  - journal error bursts
  - backup freshness and restore-test gaps
- local baseline comparison
- markdown and JSON report export

Hard rules:

- Detectors must cite snapshot fields, not raw guesswork.
- Keyword or regex log detectors must include source, window, count, and
  confidence.
- A finding with missing evidence must be labeled as uncertain.
- Monitor mode is read-only always.
- Watch mode cannot execute fixes.

Verification:

- golden tests for detector input/output
- fixture tests for common Linux failure modes
- monitor report contains no secret values
- watch mode does not mutate state
- detector false-positive review checklist before release

### v1.9 - Guided Troubleshooting

Purpose:

- Convert findings and user questions into structured troubleshooting.
- Only propose solutions after enough context is collected.

Deliverables:

- `helm troubleshoot "<problem>"`
- `helm troubleshoot --from-finding <id>`
- `helm explain <finding-id>`
- hypothesis tree:
  - hypothesis
  - evidence for
  - evidence against
  - missing evidence
  - confidence
- guided check plan:
  - read-only commands first
  - expected output
  - interpretation guide
- solution plan:
  - exact commands
  - expected effect on this system
  - blast radius
  - rollback
  - verification command
- approval prompt for each mutating step
- no execution by default

Hard rules:

- Troubleshooting starts with `SystemSnapshot`.
- User question alone is never enough to propose a fix.
- Every fix command must be tied to a finding or hypothesis.
- Every mutating command must have a verification step.
- If rollback is unsupported, the plan must say so before approval.

Verification:

- end-to-end tests using mocked snapshots
- no fix suggestion without evidence
- command previews include effect, risk, rollback, and verification
- denied approvals leave zero state changes

### v2.0 - Approved Change Execution

Purpose:

- Add controlled execution as a reviewed final step.
- Keep monitoring and troubleshooting as the main product identity.

Deliverables:

- `helm apply-plan <plan-id>`
- `helm change-set list|show|rollback`
- pre-change file snapshots
- service/package/process approval prompts
- command-by-command approval in TUI
- post-change verification
- audit chain entries for:
  - plan shown
  - approval granted or denied
  - command executed
  - output observed
  - verification result
- idempotency checks for supported actions

Hard rules:

- No plan can execute unless it was first rendered to the user.
- No hidden command expansion.
- No broad shell mutation without explicit command preview.
- No service restart without service name, reason, expected disruption, and
  rollback note.

Verification:

- mutation tests prove denied plans do not change state
- rollback tests for file changes
- audit verification after every change-set test
- TUI permission modal snapshot tests

### v2.1 - Local TUI Monitoring Dashboard

Purpose:

- Make HELM useful as an always-open ops console.

Deliverables:

- TUI dashboard with panels:
  - health summary
  - findings
  - services
  - containers
  - disk
  - ports
  - logs
  - backups
  - plans
- keyboard flow:
  - open finding
  - view evidence
  - run read-only follow-up check
  - generate troubleshoot plan
  - approve or deny fix
- no chart-heavy clone of Netdata
- terminal-native compact summaries

Hard rules:

- TUI dashboard is read-only unless user opens an apply flow.
- Findings must remain traceable to snapshot evidence.
- No UI surface may hide local-vs-API provider boundary.

Verification:

- ratatui render tests for small, normal, and wide terminals
- evidence panel must show sources, finding, risk, rollback, exact commands
- no overlapping text in constrained terminal sizes

### v2.2 - Backups And Recovery Confidence

Purpose:

- Own a painful missed-ops category: backups that exist but are not trustworthy.

Deliverables:

- backup detector framework
- support for common patterns:
  - restic
  - borg
  - rsync
  - tar archives
  - database dumps
  - Docker volume backup folders
  - Proxmox backup hints
- backup freshness findings
- backup coverage findings
- restore-test evidence field
- `helm backups check`
- `helm backups report`
- suggested restore test commands with approval gates

Hard rules:

- HELM must distinguish "backup found" from "backup verified".
- No destructive restore command may execute in monitor mode.
- Restore tests must run into temporary paths unless user approves otherwise.

Verification:

- fixtures for each supported backup style
- reports mark unknown restore status clearly
- no backup secret leakage

### v2.3 - Remote And Fleet Read-Only Monitoring

Purpose:

- Expand monitoring safely across multiple hosts.

Deliverables:

- `helm monitor --remote <name>`
- `helm monitor --fleet <group>`
- host inventory
- per-host snapshots
- per-host findings
- fleet summary
- SSH-only read-only mode
- per-target audit and snapshot storage

Hard rules:

- Fleet mode is read-only by default.
- No multi-host mutation in this release.
- Every remote finding must include host identity.
- A failing host must not block the fleet report.

Verification:

- mocked SSH tests
- partial-failure fleet test
- per-target audit partition verification
- no local host confusion in reports

### v2.4 - Integrations For Self-Hosted Stacks

Purpose:

- Build value around the users most likely to adopt early.

Deliverables:

- Docker and Compose deep inspection
- Podman support
- nginx site summary
- Caddy summary
- Grafana/InfluxDB health checks
- Postgres/MySQL basic health probes
- Proxmox read-only summary
- certificate expiry checks
- DNS checks

Hard rules:

- Integrations must degrade gracefully when tools are missing.
- Credentials must never be scraped from config into prompts.
- API tokens must come only from the secrets store or explicit env.

Verification:

- fixture-driven parser tests
- integration smoke tests where available
- redaction tests for config and token patterns

### v2.5 - Governed Automation And Second Opinion

Purpose:

- Add verification roles only after monitoring and guided troubleshooting are
  trusted.

Deliverables:

- Planner, Reviewer, Executor roles
- reviewer must approve risky plans before user approval prompt
- disagreement protocol
- `helm review-plan <id>`
- policy file for which actions require second opinion
- optional local model reviewer

Hard rules:

- Multi-agent must reduce risk, not increase autonomy.
- Parallel agents may collect read-only data.
- Parallel writes are prohibited.
- Reviewer must cite evidence, not just say yes or no.

Verification:

- disagreement tests
- reviewer cannot approve unsupported rollback silently
- policy tests for required second opinion

### v3.0 - Notifications And Long-Running Local Monitor

Purpose:

- Add useful background monitoring without becoming a SaaS.

Deliverables:

- `helm daemon`
- local schedule
- desktop notifications or webhook output
- finding history
- baseline drift alerts
- report rotation
- local-only by default

Hard rules:

- Daemon cannot execute mutations.
- Notifications must include evidence links.
- Telemetry remains opt-in.

Verification:

- daemon lifecycle tests
- schedule tests
- no-mutation daemon invariant test

## Release Gate For Every Phase

Every phase must pass:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build --release -p helm-cli
rg -n "white-phantom|github.com/helm|helm.sh/install" .
```

Every phase must also include:

- docs update
- threat-model update when boundaries change
- examples that start read-only
- no stale command examples
- clean install path
- no plaintext provider key persistence
- redaction review
- audit review

## Critical Path

The next work should happen in this order:

1. Finish and commit the v1.6 trust foundation.
2. Build `SystemSnapshot`.
3. Build deterministic detectors and `helm monitor`.
4. Build guided troubleshooting from findings.
5. Add approved execution as a reviewed final step.
6. Add dashboard and remote/fleet read-only monitoring.

This order matters because users do not trust automation yet. They may trust a
tool that quietly observes, explains, and lets them decide.
