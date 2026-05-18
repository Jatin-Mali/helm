# HELMOPS Gap-Closure Checklist

Status: drafted on 2026-05-18

## Reader

Internal engineer finishing the HELMOPS rebuild.

## Post-Read Action

Use this checklist to move the remaining milestones from partial or failed to
release-ready without guessing what "done" means.

## Scope

This document covers only the unfinished work after the current audit:

- `M2` Visual Structure and Triage Speed
- `M4` Continuous Monitoring and Fleet Evolution
- `M5` Evidence Breadth and Domain Expansion
- `M6` Alerting and Notification Pipelines

`M1` and `M3` are not repeated here because they already pass the current
product bar.

## Current Verdict

| Milestone | Status | Why it is not green yet |
|---|---|---|
| `M2` | Partial | The dashboard is operator-first, but raw views still dominate the top navigation and the incident-first information architecture is not finished. |
| `M4` | Fail | Daemon ownership, incident command surface, and real fleet collection are not complete. |
| `M5` | Partial | Runtime collectors and detectors exist, but the evidence model and incident narratives are not complete across the target domains. |
| `M6` | Partial | Alerting sinks exist, but policy, lifecycle propagation, and operator-facing configuration are not complete as a product workflow. |

## Completion Rule

A milestone is green only when all three are true:

1. the feature exists in the product surface, not only in a library
2. the acceptance tests for that milestone pass
3. the operator workflow matches the incident-first product model

## M2 Checklist

### Outcome

An operator opens HELMOPS and identifies the top problem, the impact, the
timing, and the next action in under 10 seconds.

### Remaining Work

#### 1. Finish the incident-first hierarchy

- rename the left pane mental model from finding inventory to incident queue
- keep the queue focused on severity, host, status, `open for`, and `last seen`
- remove any visual impression that services, processes, logs, network, disk,
  and changes are peer entry points to incident triage
- keep raw views reachable in at most two actions, but visually secondary

#### 2. Tighten the top bar and health strip

- show scope clearly: local host, selected host, or fleet
- keep AI state visible as `ai off`, `ai local`, or `ai api`
- make the critical count and service or disk risk obvious before the user
  looks at the queue
- ensure collector health is visible without overwhelming the top bar

#### 3. Reduce visual friction in the workspace

- keep section ordering fixed: what broke, impact, when, why, change context,
  evidence, verify, fix, rollback, execution status
- reduce low-signal labels and duplicate metadata
- ensure the selected incident workspace reads like one coherent story instead
  of several unrelated panes

#### 4. Match the target operator image more closely

- preserve the current terminal visual direction
- align labels, action wording, and pane emphasis to the final daily-ops
  console target
- keep footer shortcuts minimal and consistent with the surfaced actions

### Deliverables

- incident-first dashboard polish pass
- revised labels and pane emphasis
- updated render snapshots for main dashboard states
- one reference screenshot or HTML mock aligned to the implemented layout

### Verification

- screenshot review: top risk is obvious without tab hunting
- render tests cover empty, warning-heavy, and critical incident states
- raw evidence views are reachable in at most two actions
- selected incident workspace remains readable at standard terminal sizes

### Exit Criteria

- a beginner can answer "what broke and what do I do next" in under 10 seconds
- an expert can still reach logs, services, processes, network, storage, and
  changes without hunting

## M4 Checklist

### Outcome

The TUI becomes a client of long-running monitoring state, and fleet monitoring
works through real host registration instead of local-only refresh logic.

### Remaining Work

#### 1. Add daemon ownership

- implement a long-running local daemon loop
- move scheduled collection, detector runs, finding lifecycle updates, plan
  precomputation, and persistence into the daemon
- persist daemon heartbeat, collector health, and last successful run
- prevent overlapping collection cycles

#### 2. Add operator command surface

- add `helm daemon start`
- add `helm daemon stop`
- add `helm daemon status`
- add `helm dashboard`
- add `helm incident show <id>`
- add `helm incident plan <id>`
- add `helm incident apply <id|plan-id>`

#### 3. Make the dashboard a daemon client

- stop treating the TUI as the owner of scheduled collection
- read latest persisted state from daemon-managed storage
- keep manual refresh as a request to the daemon, not a local full collector
  owner path
- show daemon health and staleness clearly in the UI

#### 4. Implement real fleet collection

- replace the current fleet placeholder logic with real data flow
- support registered remote hosts
- aggregate host health, incident counts, and reachability
- support per-host drill-down and host selection

#### 5. Add installed agent workflow

- define install flow for Linux hosts
- define registration flow back to the central console
- define host identity and trust model
- support snapshot exchange or query API
- keep remote execution behind explicit reviewed approval

### Deliverables

- daemon lifecycle implementation
- daemon CLI command surface
- daemon-backed dashboard reads
- first working remote host registration path
- first working fleet view with real host rollups

### Verification

- daemon runs without the TUI open
- daemon continues collecting and updating findings on schedule
- dashboard can render from existing daemon state without triggering a full
  local collection cycle
- at least one registered remote host appears in fleet view
- mixed local and remote audit chain still verifies

### Exit Criteria

- closing the TUI does not stop monitoring
- reopening the TUI shows persisted recent state immediately
- fleet view is backed by real host data, not placeholder rows or comments

## M5 Checklist

### Outcome

HELMOPS covers the systems operators actually manage, and each incident includes
useful, domain-specific evidence rather than generic summaries.

### Remaining Work

#### 1. Expand runtime coverage in the incident model

- strengthen Docker and Podman evidence
- strengthen compose and service correlation
- improve virtualization support
- define the first Kubernetes adapter boundary without pretending to solve the
  whole cluster problem at once

#### 2. Improve incident narratives by domain

- containers: restarts, health, image change context, port collisions
- storage: disk fullness, inode pressure, readonly mounts, SMART state
- services: failed units, inactive expected units, restart loops, timers
- backups: stale backups, failed schedules, restore-test evidence
- packages and security: update pressure and security-sensitive pending updates

#### 3. Improve change correlation

- connect incidents to recent approved changes
- connect incidents to recent deploy or release signals where available
- surface "what changed before it broke" with domain-specific detail

#### 4. Tune detectors and thresholds

- reduce noisy low-value incidents
- increase confidence for recurring high-signal incidents
- add fixture-backed tuning for edge cases

### Deliverables

- expanded collectors and detectors where gaps still exist
- richer evidence formatting by incident domain
- improved change-context summaries
- fixture sets for each supported runtime family

### Verification

- each supported runtime family has at least one fixture-backed incident test
- new domain evidence appears in the incident workspace, not only in raw tabs
- recurring incidents show credible root-cause clues instead of generic text

### Exit Criteria

- operators managing services, containers, storage, and backups can use the
  dashboard as the primary triage surface without immediately dropping into raw
  tables

## M6 Checklist

### Outcome

Critical incidents can notify external systems without turning HELMOPS into
spam or breaking the local-first trust model.

### Remaining Work

#### 1. Productize alert routing

- expose alerting configuration as a supported operator workflow
- define severity thresholding policy
- define dedup window behavior by incident fingerprint
- define rate limits and failure behavior for outbound sinks

#### 2. Connect lifecycle changes to notifications

- send open or critical notifications once per dedup window
- propagate resolve events
- propagate suppression decisions
- avoid repeated notifications for unchanged recurring incidents inside the same
  dedup period

#### 3. Add audit coverage for notification decisions

- audit when a notification is sent
- audit when a notification is skipped due to dedup or severity policy
- audit when a notification resolves or suppresses an external alert

#### 4. Surface alerting state to the operator

- show whether alerting is configured
- show recent delivery outcomes
- show sink failures without hiding local monitoring state

### Deliverables

- supported alerting configuration model
- outbound sink lifecycle integration
- audit coverage for send, skip, resolve, and suppress decisions
- operator-visible notification status

### Verification

- one critical incident emits one notification inside the dedup window
- the same fingerprint does not spam repeated notifications during the dedup
  period
- resolve emits the correct lifecycle update
- suppress prevents further sends according to policy
- sink delivery failures are visible and auditable

### Exit Criteria

- operators can trust that critical incidents notify exactly when they should,
  and not more often

## Suggested Execution Order

1. finish `M2` visual hierarchy so the final product target is stable
2. finish daemon ownership in `M4`
3. finish real fleet data flow in `M4`
4. expand runtime evidence and domain narratives in `M5`
5. productize external alerting policy and lifecycle propagation in `M6`

## Release Gate For Gap Closure

Do not mark these milestones complete until all of the following pass:

1. `cargo fmt --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace --all-targets`
4. `cargo build --release -p helm-cli`
5. screenshot review against the incident-first target dashboard
6. one operator walkthrough for local-only mode
7. one operator walkthrough for daemon-backed mode
8. one operator walkthrough for fleet mode

## Reader Test

If you pick up this document cold, you should be able to answer:

- what is still incomplete
- in what order to do it
- how to verify each milestone
- what counts as truly done

If any of those are unclear, revise this document before using it as the
milestone source of truth.
