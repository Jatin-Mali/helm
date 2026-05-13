# Data Boundary

HELM enforces a strict boundary between external and local data. This prevents injection, exfiltration, and accidental writes.

## Taint System

Every piece of data in HELM carries a `TaintLevel`:

| Level | Source | Write Allowed? |
|-------|--------|----------------|
| `Local` | User input, local files | Yes |
| `External` | Browser output, SSH, MCP servers | **No** |

Taint is propagated through `Tainted<T>`. Operations that mix external and local data produce `External`-tainted results.

## Write Gate

The capability gate enforces: **`*.write` capabilities are blocked on `TaintLevel::External` tainted inputs.**

This means:
- A browser's HTML output cannot be written to a local file
- SSH command output cannot seed a package install
- MCP server responses cannot control service management

## Stripping Taint

Taint can only be stripped by explicit user action — approving a permission modal or passing `--yes`. There is no programmatic taint strip. The `Tainted<T>` type has no `into_inner()` — only `map()` and `and_then()` for safe transformation.

## Diagnose Boundary

In diagnose mode (Level 0), the data boundary is absolute:
- Only 9 read tools are registered
- Shell output cannot be redirected to files
- No tool can modify the filesystem
- No package installation, service control, or network writes

## Audit Trail

Every tool call is logged with:
- Tool name and input schema
- Taint level at call time
- Capability gate result
- Timestamp and session ID

The audit log is append-only. The HMAC chain must verify from episode 0 to current — any gap or mismatch is reported by `helm trust-report`.

## Trust Report

```bash
helm trust-report
```

Reports:
1. **Grants** — active capability grants and their scope (session/permanent)
2. **Audit** — HMAC chain verification, episode count, last episode timestamp
3. **Sandbox** — whether Bubblewrap is available and configured
4. **Diagnose** — current diagnose-mode status, tool allowlist
5. **Integrity** — binary checksum, config hash
