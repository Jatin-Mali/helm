# HELM Technical Requirements Document
# Monitoring-First DevOps Assistant
# Canonical revision: 2026-05-13

## 1. Technical Thesis

HELM must become a typed monitoring and troubleshooting engine with optional
approved execution. The LLM is not the source of truth. The source of truth is
the system snapshot, detector output, evidence model, audit trail, and user
approval.

The execution pipeline is:

```text
collect -> normalize -> detect -> explain -> plan -> preview -> approve -> execute -> verify -> audit
```

Only the last three stages can mutate state, and only after explicit approval.

## 2. Existing Codebase Map

Current modules to keep and evolve:

- `helm-cli/src/main.rs`: CLI commands, provider setup, command rendering
- `helm-cli/src/tui.rs`: terminal UI, permission and evidence surfaces
- `helm-cli/src/paths.rs`: XDG paths
- `helm-cli/src/remote.rs`: remote target registry
- `helm-cli/src/sandbox.rs`: Bubblewrap policy
- `crates/helm-core`: capabilities, messages, taint, validation, redaction
- `crates/helm-tools`: typed tool implementations
- `crates/helm-agent`: ReAct loop, supervisor, budget, evidence
- `crates/helm-memory`: episodes, audit, sessions, graph, skills, profile
- `crates/helm-providers`: model providers and quirks

Current modules to add:

- `crates/helm-monitor`
- `crates/helm-monitor/src/snapshot.rs`
- `crates/helm-monitor/src/collectors/*`
- `crates/helm-monitor/src/detectors/*`
- `crates/helm-monitor/src/findings.rs`
- `crates/helm-monitor/src/baseline.rs`
- `crates/helm-monitor/src/report.rs`
- `crates/helm-agent/src/troubleshoot.rs`
- `crates/helm-agent/src/plan_preview.rs`
- `crates/helm-memory/src/snapshots.rs`
- `crates/helm-memory/src/findings.rs`
- `crates/helm-memory/src/change_sets.rs`

## 3. Core Architecture

```text
CLI/TUI
  -> Monitor Profile
  -> Snapshot Engine
  -> Detector Engine
  -> Finding Store
  -> Troubleshooting Planner
  -> Command Preview
  -> Approval Gate
  -> Tool Execution
  -> Verification
  -> Audit and ChangeSet Store
```

The snapshot and detector engine must work without any LLM provider. The LLM
may summarize, correlate, or generate guided plans, but it must not invent
findings without detector evidence.

## 4. New Core Data Models

### 4.1 SystemSnapshot

```rust
pub struct SystemSnapshot {
    pub id: SnapshotId,
    pub host: HostIdentity,
    pub collected_at: DateTime<Utc>,
    pub profile: MonitorProfile,
    pub domains: SnapshotDomains,
    pub collector_errors: Vec<CollectorError>,
    pub redaction_version: String,
}
```

Required domains:

```rust
pub struct SnapshotDomains {
    pub host: HostSnapshot,
    pub load: LoadSnapshot,
    pub disks: DiskSnapshot,
    pub services: ServiceSnapshot,
    pub containers: ContainerSnapshot,
    pub ports: PortSnapshot,
    pub logs: LogSnapshot,
    pub backups: BackupSnapshot,
    pub packages: PackageSnapshot,
    pub timers: TimerSnapshot,
    pub network: NetworkSnapshot,
}
```

### 4.2 Finding

```rust
pub struct Finding {
    pub id: FindingId,
    pub snapshot_id: SnapshotId,
    pub severity: Severity,
    pub confidence: Confidence,
    pub category: FindingCategory,
    pub title: String,
    pub affected_resource: String,
    pub evidence: Vec<EvidenceRef>,
    pub assumptions: Vec<String>,
    pub missing_data: Vec<String>,
    pub impact: String,
    pub read_only_checks: Vec<CommandPreview>,
    pub fix_plan: Option<PlanId>,
}
```

### 4.3 Detector

```rust
pub trait Detector {
    fn id(&self) -> &'static str;
    fn domain(&self) -> MonitorDomain;
    fn detect(&self, snapshot: &SystemSnapshot) -> Vec<Finding>;
}
```

Rules:

- Detectors must consume typed snapshot fields.
- Regex and keyword matching may only support evidence extraction.
- A detector must never call shell directly.
- A detector must output confidence and evidence references.

### 4.4 TroubleshootingPlan

```rust
pub struct TroubleshootingPlan {
    pub id: PlanId,
    pub source: PlanSource,
    pub snapshot_id: SnapshotId,
    pub hypotheses: Vec<Hypothesis>,
    pub read_only_steps: Vec<PlanStep>,
    pub proposed_fix_steps: Vec<PlanStep>,
    pub approval_required: bool,
}
```

### 4.5 CommandPreview

```rust
pub struct CommandPreview {
    pub tool: String,
    pub input: serde_json::Value,
    pub command_text: Option<String>,
    pub expected_effect: String,
    pub risk: RiskLevel,
    pub blast_radius: BlastRadius,
    pub rollback: RollbackStatus,
    pub verification: Vec<CommandPreview>,
}
```

## 5. Snapshot Engine

Collectors must be typed, bounded, and partial-failure tolerant.

Collector requirements:

- timeout per collector
- no mutation
- no secret persistence
- structured parser tests
- domain-specific error reporting
- raw output retained only when redacted and needed for evidence

Initial collectors:

- host identity collector
- load collector
- disk collector
- service collector
- container collector
- port collector
- log collector
- backup collector
- timer collector
- package collector
- network collector

Command examples by collector:

- load: `/proc/loadavg`, `/proc/pressure/*`, `free -b`
- disk: `df`, `findmnt`, `lsblk`, `smartctl` if present
- services: `systemctl list-units`, `systemctl --failed`, journal windows
- containers: `docker ps`, `docker inspect`, `docker compose ps`
- ports: `ss -tulpn`
- logs: `journalctl` bounded windows
- backups: detect restic, borg, rsync, tar, Proxmox backup paths

## 6. Detector Engine

Initial detectors:

- root filesystem usage high
- inode usage high
- SMART health warning
- filesystem remounted read-only
- large deleted-open files
- failed systemd units
- service restart loop
- enabled service inactive
- unhealthy container
- container restart loop
- exposed unexpected listener
- high CPU load
- memory pressure
- swap exhaustion
- OOM killer event
- journal error burst
- backup stale
- backup schedule missing
- restore test missing
- certificate near expiry
- package security updates available

Every detector must include:

- fixture input
- expected finding output
- false-positive notes
- remediation hint or "no automatic fix"

## 7. LLM Role

The LLM may:

- summarize findings
- rank hypotheses
- explain command effects
- generate troubleshooting plans from findings
- translate evidence into human language

The LLM may not:

- create findings without evidence
- bypass detectors for monitor mode
- execute commands
- approve its own plan
- hide missing data
- turn a denied command into a different command

## 8. Execution Modes

### Monitor Mode

- command: `helm monitor`
- read-only only
- no approval prompts because no mutation is possible
- stores snapshot and findings
- works without provider keys

### Troubleshoot Mode

- command: `helm troubleshoot "<problem>"`
- collects snapshot first
- builds hypotheses and plan
- read-only checks can run
- mutating fix steps are preview-only unless approved

### Apply Mode

- command: `helm apply-plan <id>`
- renders command previews
- requires approval
- creates change-set
- verifies outcome
- writes audit events

### Daemon Mode

- command: `helm daemon`
- read-only monitoring loop
- no mutation path
- local notification or webhook output only

## 9. Permission And Safety Invariants

These invariants are release-blocking:

- Monitor mode cannot register write tools.
- Diagnose mode cannot register write tools or execute mutating sub-actions.
- Troubleshoot mode cannot propose fixes before snapshot collection.
- Apply mode cannot execute a command without preview.
- `--yes` cannot apply to monitor mode.
- Remote fleet mode is read-only until a later governed release.
- Secrets are redacted before persistence.
- HELM local sensitive paths are denied by default.
- Provider API boundary is displayed in trust-report and first-run docs.

## 10. Evidence And Preview Requirements

Before a mutating command, the user must see:

- what HELM inspected
- what HELM found
- what HELM assumes
- what data is missing
- exact command or tool input
- expected effect on this system
- affected files, services, processes, hosts, or containers
- rollback status
- verification command
- audit target

No command may be represented only as "fix issue". It must be concrete.

## 11. Persistence

New tables:

- `snapshots`
- `snapshot_domains`
- `findings`
- `finding_evidence`
- `baselines`
- `troubleshooting_plans`
- `plan_steps`
- `command_previews`
- `change_sets`
- `change_set_steps`

Persistence rules:

- store redacted structured data
- raw command output is optional and bounded
- every finding links to snapshot ID
- every plan links to finding or user task
- every change-set links to audit event IDs

## 12. TUI Requirements

The TUI must support:

- dashboard-first startup when `helm` is run with no arguments
- monitor dashboard
- finding list
- finding detail view
- evidence view
- troubleshoot plan view
- command preview modal
- approval modal
- change-set result view

Design constraints:

- dense terminal-first layout
- no decorative UI
- clear severity markers
- every dashboard panel must have a drill-down detail view
- no overlapping text at small terminal sizes
- keyboard-first navigation
- visible local/API boundary
- dashboard refresh and follow-up checks remain read-only

## 13. Remote Requirements

Remote monitoring must:

- use registered SSH targets
- collect read-only snapshots
- store per-target findings
- include host identity in every finding
- tolerate partial host failures
- never run multi-host mutation in read-only fleet mode

Remote execution remains later and must require:

- explicit target
- exact command preview
- per-target approval
- per-target audit
- per-target rollback where supported

## 14. Testing Requirements

Unit tests:

- every parser
- every detector
- every command preview builder
- every permission invariant

Integration tests:

- `helm snapshot --json`
- `helm monitor --json`
- `helm troubleshoot --from-finding`
- denied apply leaves no state changes
- audit verifies after apply

Fixture tests:

- full disk
- inode pressure
- failed service
- restart loop
- unhealthy container
- unexpected port
- OOM event
- stale backup
- missing restore test

Release gate:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build --release -p helm-cli
rg -n "white-phantom|github.com/helm|helm.sh/install" .
```

## 15. Security Requirements

- no plaintext provider keys in config
- secrets only in secrets store
- no silent env-to-store copying
- protected HELM state denied by `fs_read`
- redaction before persistence and tracing
- network requests are domain-policy controlled
- remote actions include target in audit domain
- logs and findings must avoid secrets

## 16. Implementation Sequence

1. Create `helm-monitor` crate and snapshot model.
2. Add collectors with fixture tests.
3. Add detector framework and MVP detectors.
4. Add `helm snapshot`.
5. Add `helm monitor`.
6. Persist snapshots and findings.
7. Add troubleshoot planner from findings.
8. Add command preview and approval flow.
9. Add change-set execution.
10. Make the TUI dashboard the default product surface.
11. Add remote read-only monitoring.
12. Add daemon and notifications.

This sequence keeps the product safe while moving from assistant automation to
monitoring and guided troubleshooting.
