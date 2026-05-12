# Agent-on-Remote Transport (NDJSON over SSH)

HELM's `--remote <host>` flag runs the full ReAct agent *on* the remote machine and
streams live events back to the local process.  The wire format is
**newline-delimited JSON (NDJSON)** carried over SSH stdout — no gRPC, no protobuf,
no extra ports.

> **Design decision (v1.5):** gRPC was the original plan (roadmap Q2). After
> shipping the NDJSON approach we found it simpler, auditable with plain `jq`, and
> just as capable for real-time event delivery.  gRPC stays on the v2.0 backlog for
> high-frequency streaming over unreliable links.

---

## How it works

```
local helm CLI
  └─ SSH subprocess: ssh <host> helm run "<goal>" --emit-events
       └─ remote helm binary
            └─ NdjsonSink → one JSON object per stdout line
  ← agent_remote::run_on_remote reads each line, parses AgentEvent, re-emits locally
```

1. The local CLI opens an SSH subprocess (via `std::process::Command`) running
   `helm run "<goal>" --emit-events` on the remote host.
2. The remote `helm` binary runs the full ReAct loop with `NdjsonSink` attached,
   writing one JSON object per line to stdout (`ndjson_sink.rs:191`).
3. The local `run_on_remote` function (`agent_remote.rs:56`) reads stdout
   line-by-line, deserialises each into a `WireLine`, maps it to an `AgentEvent`,
   and forwards it to the local `AgentEventSink` (TUI, CLI progress, hooks, audit).

---

## Wire format

Each line is a JSON object with a mandatory `"type"` field plus event-specific
fields.  Example sequence:

```json
{"type":"run_started","episode_id":"uuid","goal":"restart nginx"}
{"type":"provider_call_started","iteration":0,"provider":"anthropic","model":"claude-sonnet-4-6"}
{"type":"text_delta","chunk":"I'll restart"}
{"type":"tool_started","id":"t1","name":"shell"}
{"type":"tool_finished","id":"t1","name":"shell","success":true,"content":"OK"}
{"type":"provider_call_finished","iteration":1,"stop_reason":"end_turn","tokens_in":420,"tokens_out":35}
{"type":"run_finished","episode_id":"uuid","outcome":"success","turns":2}
```

All event types mirror `AgentEvent` variants; the full list is in
`crates/helm-agent/src/react.rs` and the parser in `agent_remote.rs:130`.

---

## Framing and backpressure

- **Framing:** line-delimited (newline `\n` terminates each object).  No length
  prefix or envelope needed; JSON objects never contain unescaped newlines.
- **Backpressure:** SSH stdout is a kernel pipe.  If the local consumer stalls,
  SSH apply flow-control to the remote writer automatically — no additional
  buffering needed.
- **Stderr:** the remote's stderr is forwarded verbatim to the local stderr so
  `tracing` logs remain visible.

---

## Version compatibility

The bootstrap sequence (`bootstrap.rs`) probes the
remote binary:

```bash
ssh <host> helm --version
```

The version string is parsed and checked during bootstrap. A minor version
mismatch produces a warning; a major version mismatch aborts with an error.
That keeps freshly-bootstrapped targets from silently drifting into an
incompatible wire format.

---

## Supported event types (v1.5)

| `type` field | Maps to |
|---|---|
| `run_started` | `AgentEvent::RunStarted` |
| `run_finished` | `AgentEvent::RunFinished` |
| `run_failed` | `AgentEvent::RunFailed` |
| `provider_call_started` | `AgentEvent::ProviderCallStarted` |
| `provider_call_finished` | `AgentEvent::ProviderCallFinished` |
| `plan_started` | `AgentEvent::PlanStarted` |
| `plan_finished` | `AgentEvent::PlanFinished` |
| `text_delta` | `AgentEvent::TextDelta` |
| `assistant_text` | `AgentEvent::AssistantText` |
| `tool_parsed` | `AgentEvent::ToolCallParsed` |
| `tool_validated` | `AgentEvent::ToolCallValidated` |
| `tool_started` | `AgentEvent::ToolCallStarted` |
| `tool_finished` | `AgentEvent::ToolCallFinished` |
| `tool_denied` | `AgentEvent::ToolCallDenied` |
| `permission_requested` | `AgentEvent::PermissionRequested` |
| `permission_denied` | `AgentEvent::PermissionDenied` |
| `budget_warning` | `AgentEvent::BudgetWarning` |
| `budget_exceeded` | `AgentEvent::BudgetExceeded` |
| `format_recovery` | `AgentEvent::FormatRecoveryUsed` |
| `correction_used` | `AgentEvent::CorrectionUsed` |
| `plan_cache_hit` | `AgentEvent::PlanCacheHit` |
| `skill_suggested` | `AgentEvent::SkillSuggested` |
| `provider_failover` | `AgentEvent::ProviderFailover` |
| `postcondition_warning` | `AgentEvent::PostconditionWarning` |
| `validation_failed` | `AgentEvent::ValidationFailed` |
| `breakpoint_hit` | `AgentEvent::BreakpointHit` |
| `prompt_cache_hit` | `AgentEvent::PromptCacheHit` |

Unknown `type` values are silently dropped so older local clients remain
compatible with newer remote binaries.

---

## Security

- HELM never stores SSH keys.  It reads `~/.ssh/config` and delegates to the
  user's `ssh-agent`.
- The remote binary runs with the same capabilities as an interactive SSH session
  — no privilege elevation.
- Audit events are written on the *remote* host into the HELM audit shard
  directory under its XDG data path, typically
  `~/.local/share/helm/audit/<local-host>.db`
  (per-host sharding, roadmap Q6) so a compromised transport does not lose the
  remote audit trail.
