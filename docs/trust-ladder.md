# Trust Ladder

HELM's trust model is a ladder. Each rung grants more capability and demands
more evidence, review, and audit.

```
Level 4: Governed Automation  (future policy-driven automation)
    ↑
Level 3: Remote Approved     (SSH targets, per-target audit)
    ↑
Level 2: Local Approved      (reviewed local execution)
    ↑
Level 1: Troubleshoot        (read-only checks + reviewed fix plans)
    ↑
Level 0: Dashboard / Monitor (read-only snapshots and findings)
```

## Level 0 - Dashboard / Monitor

**What it does:** Collects typed system context and renders findings without
mutation.

**What it can do:**

- collect typed snapshots
- run deterministic detectors
- render findings, evidence, and impact
- launch read-only follow-up checks from the dashboard

**What it cannot do:**

- write files
- restart services
- install packages
- kill processes
- apply fixes

**Invocation:**

```bash
helm
helm snapshot
helm monitor
helm monitor --watch --interval 60s
```

## Level 1 - Troubleshoot

**What it does:** Builds hypotheses and read-only verification steps from a
finding or user-reported problem. Fix steps are rendered but not executed.

**Protections:**

- troubleshooting starts from a stored snapshot or a freshly collected one
- every fix step is tied to findings and evidence
- every command preview includes expected effect, risk, blast radius, rollback,
  and verification

**Invocation:**

```bash
helm troubleshoot "nginx is slow"
helm troubleshoot --from-finding finding-123
helm explain finding-123
```

## Level 2 - Local Approved

**What it does:** Executes a reviewed plan step by step after explicit approval.

**Protections:**

- capability gate checked before every tool call
- taint level propagated through `Tainted<T>` so external content never
  auto-approves writes
- audit log append-only with HMAC chain verification
- command-by-command approval in CLI and TUI apply flows

**Invocation:**

```bash
helm apply-plan plan-123
helm change-set list
```

## Level 3 - Remote Approved

**What it does:** Same as Level 2, but inspection and approved execution happen
on a registered remote host over SSH.

**Invocation:**

```bash
helm monitor --remote prod-1
helm troubleshoot --remote prod-1 --from-finding finding-123
helm --remote prod-1 tui --mode dashboard
```

## Level 4 - Governed Automation

**What it does:** Policy-bound scheduled automation with predeclared capability
bounds, audit, and rollback expectations.

This is explicitly not the day-one product surface.

## Sandbox

Any level that executes local commands can add `--sandbox` to confine tool
execution in a Bubblewrap container. The sandbox root defaults to the current
working directory.
