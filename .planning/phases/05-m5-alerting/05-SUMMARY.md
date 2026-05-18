# Phase 5: M5 — Alerting — COMPLETE

**Completed:** 2026-05-18  
**Commits:** e60437b → a34f9e1 (3 commits)

## What Was Built

### Wave 1 — Foundation (05-01)
- `FindingLifecycle` enum added to `findings.rs` (`Open`, `Resolved`, `SelfResolved`, `Suppressed`)
- `lifecycle` field on `Finding` struct (default `Open`); used by PD resolution path
- `AlertPayload` — derived from `Finding`, carries fingerprint/severity/title/description/resource/detector/category/timestamp/lifecycle
- `AlertConfig` — deserializable from `~/.config/helm/thresholds.toml` [alerting]: min_severity, dedup_window_secs (300), rate_limit_per_min (60)
- `AlertRouter` — severity gate → per-fingerprint dedup window → 60s rate-limit window → fan-out to sinks
- `AlertSink` trait with `SendFuture<'a>` type alias (avoids clippy complex-type lint)

### Wave 2 — Sinks (05-02/03/04)
- **WebhookSink**: HTTP POST with exponential backoff (3 attempts; 4xx → immediate fail, 5xx/network → retry)
- **SlackSink**: Slack incoming webhook; severity colors Critical=#d32f2f, Warning=#f9a825, Info=#1565c0; channel override
- **PagerDutySink**: Events API v2 at `https://events.pagerduty.com/v2/enqueue`; lifecycle→action (Open→trigger, Resolved/SelfResolved→resolve); dedup_key=fingerprint; 202 Accepted treated as success
- All three sinks use `TcpListener` port-0 mock HTTP servers in tests (no extra deps)

### Wave 3 — Engine Integration (05-05)
- `AlertingEngine` in `engine.rs`: wraps `DetectorRegistry` + `AlertRouter`, exposes `run_once(profile)` → collect → detect → route
- `with_detectors()` override for test injection
- Integration test: `AlwaysCritDetector` + `CaptureSink` verifies full collect→detect→route chain

## Test Coverage
- 634 tests passing (was 617 before Phase 5; +17 new tests)
- All gate checks clean: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --workspace`

## Files Added/Modified
- `crates/helm-monitor/src/findings.rs` — `FindingLifecycle` enum + `lifecycle` field
- `crates/helm-monitor/src/lib.rs` — `pub mod alerting`, `AlertingEngine` export, `FindingLifecycle` export
- `crates/helm-monitor/src/alerting/mod.rs` — `AlertSink`, `AlertRouter`, `SendFuture<'a>`
- `crates/helm-monitor/src/alerting/payload.rs` — `AlertPayload`, `From<&Finding>`
- `crates/helm-monitor/src/alerting/config.rs` — `AlertConfig`, `load_config()`
- `crates/helm-monitor/src/alerting/webhook.rs` — `WebhookSink`
- `crates/helm-monitor/src/alerting/slack.rs` — `SlackSink`
- `crates/helm-monitor/src/alerting/pagerduty.rs` — `PagerDutySink`
- `crates/helm-monitor/src/engine.rs` — `AlertingEngine`
- `crates/helm-monitor/Cargo.toml` — added `reqwest.workspace`, `toml.workspace`
