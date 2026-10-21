# HELM completion specification

Date: 2026-10-21. Evidence baseline: `HELM_CODEBASE_AUDIT.md`, commit `fbbbec3631b23965f692893e0a35b24e43f9f680`.

## Required outcome

Deliver a usable Linux operations product and a general AI agent. A new user can install it, configure a provider, inspect their host, understand findings, approve specific changes, verify results, recover supported changes, and ask the agent to complete multi-step tasks. CLI, TUI and web must describe actual capabilities accurately. The user explicitly requires the general agent in addition to monitoring and remediation.

Initial release support is Linux x86-64. Other architectures need tested release artifacts before claiming binary support. Fleet support means actual SSH observations with host identity and freshness, never local results relabeled as remote. Provider adapters remain available; only providers that pass live validation may be described as live-tested.

## Non-negotiable behavior

1. A denied, expired, changed or unreviewed operation cannot mutate the host. Approval binds the exact operation and target, including the host and working directory.
2. Read-only and dry-run policies cover every frontend, tool, hook and rollback. A requested sandbox must be effective or the command must fail before execution. Read-only mode allows trusted observation operations, not arbitrary shell commands labeled read-only by a model.
3. Missing observations mean unknown, never healthy or recovered. Recovery requires a successful comparable observation.
4. Applied changes have durable records. Supported rollback restores original bytes, permissions and existence; unsupported or irreversible actions say so before approval. Execution and filesystem effects cannot be made atomically transactional together: interruption must remain visible and recoverable.
5. Secrets stay in the designated secrets store. Persistence, logs, exports and provider prompts minimize/redact sensitive data. Executable input is kept separate from redacted display text.
6. The local dashboard authenticates clients and checks request origin/host. Authentication does not automatically authorize a mutation. Pipeline ingress verifies sender authenticity and replay resistance.
7. Agent work is bounded by time, steps, output, context and configured cost limits. Cancellation terminates descendants. Unknown model pricing must not silently bypass a configured spend limit.
8. Agent success requires evidence. Tool output, retrieved documents and model suggestions cannot grant permissions. Commands are not executed merely because a model says they are safe.
9. Existing databases upgrade without silent loss. Unsupported old plan formats fail visibly or migrate explicitly. Global runtime paths apply consistently.
10. Both binary names work. Release URLs retain `Jatin-Mali/helm`; install assets are checked before replacing a working installation.

## Working demonstrations

**Operations:** clean install → collect → show a controlled fault → explain evidence → review exact plan → deny one step → apply an approved reversible change → verify → restart HELM → restore original state. Show a failed collector as unknown and a failed restoration as failed.

**General agent:** in a temporary workspace, inspect a small project, explain its structure, make an approved edit, run a check, recover from a failed check and summarize evidence. Repeat with all writes denied, cancellation, malicious instructions embedded in a file, provider failure and exhausted budget. A real configured provider must complete the positive scenario; deterministic mocks cover repeatable failures.

**Fleet:** two disposable SSH hosts produce distinguishable observations; disconnecting one marks it unavailable without substituting local data.

## Boundaries

General agent means multi-step reasoning with approved filesystem, command and HTTP tools, persistent sessions and useful CLI/TUI access. A functioning browser interaction tool is included as a separately permissioned adapter to an installed browser backend, with a real browser acceptance check. Autonomous background self-modification, unlimited delegation and arbitrary third-party MCP execution are not release requirements. Missing optional backends must be reported clearly.

The first public demonstration runs on a disposable Linux VM. Successful mocks alone do not establish operational readiness. No plan can guarantee the absence of every vulnerability; completion is measured against these observable requirements, all audit findings, and the release checks.
