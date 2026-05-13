# Trust Ladder

HELM's trust model is a ladder — each rung grants more capability but requires more evidence.

```
Level 4: Governed Automation  (CI/CD pipelines, cron jobs, pre-signed tasks)
    ↑
Level 3: Remote Approved     (SSH targets, serve mode with bearer tokens)
    ↑
Level 2: Local Approved      (default: permission modal for every write/exec)
    ↑
Level 1: Dry-Run             (print what would happen, execute nothing)
    ↑
Level 0: Diagnose            (read-only, 9 safe tools, enforced at registration)
```

## Level 0 — Diagnose

**What it can do:** Read files, list processes, check disk, inspect network, git log/status, HTTP GET, shell commands without pipes/redirects.

**What it cannot do:** Write files, delete files, install packages, control services, run shell pipelines, redirect output, SSH/SCP, browser control.

**Enforcement:** Tool registration level. `ToolRegistry::with_diagnose_tools()` registers only 9 safe tools. The model literally never sees write-tool schemas. ShellTool additionally gates `ShellMode::Shell` and output redirection at runtime.

**Invocation:**
```bash
helm diagnose "why is /var/log growing?"
helm tui --read-only --dry-run
```

## Level 1 — Dry-Run

**What it does:** Prints intended tool calls and their synthetic output without executing anything. Every `tool.execute()` call is intercepted at the agent execution level before reaching OS commands.

**Invocation:**
```bash
helm run --dry-run "clean up /tmp"
helm tui --dry-run
```

## Level 2 — Local Approved (Default)

**What it does:** Full tool access with permission gating. Each write/exec/sudo tool call requires explicit approval via the TUI permission modal or CLI confirmation. Read tools auto-approve.

**Protections:**
- Capability gate checked before every tool call
- Taint level propagated through `Tainted<T>` — external content never auto-approved for writes
- Audit log append-only with HMAC chain verification

## Level 3 — Remote Approved

**What it does:** Same as Level 2 but tool execution happens on a remote host via SSH. Requires `helm remote add` registration and bearer token for `helm serve` mode.

**Invocation:**
```bash
helm --remote myserver tui
helm serve --bind 0.0.0.0:9090
helm tui --attach myserver:9090
```

## Level 4 — Governed Automation

**What it does:** Pre-signed task manifests with capability budgets. Used for CI/CD pipelines, cron jobs, and automated remediation. Each task carries an HMAC-signed manifest declaring allowed capabilities.

**Planned for v1.7.**

## Sandbox

Any level can add `--sandbox` to confine tool execution in a Bubblewrap container. The sandbox root defaults to the current working directory.
