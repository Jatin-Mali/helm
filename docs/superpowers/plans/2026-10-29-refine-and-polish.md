# Refine & Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every user-facing feature of HELM (web console first, then CLI/TUI) works the way an operator expects, and the web console gets a deliberate visual/UX pass.

**Architecture:** Three phases. (A) Fix proven-broken web flows at their root (server state, not UI patches). (B) A feature-by-feature verification sweep across web/CLI/TUI that produces a defect ledger; each defect becomes a RED→GREEN task appended to this plan. (C) Web UI redesign of the single embedded `dashboard.html` with frontend-design + design-taste-frontend + impeccable, after behavior is correct so the redesign doesn't hide bugs.

**Tech Stack:** Rust (axum, tokio, rusqlite), vanilla JS/HTML/CSS embedded via `include_str!` (`helm-cli/src/web/dashboard.html`), integration tests spawning `CARGO_BIN_EXE_helm` (pattern: `helm-cli/tests/web_security.rs`).

**Spec:** User request 2026-10-29 ("every feature works as intended; improve the web UI with frontend/impeccable/taste skills"). Prior defect history: `HELM_CODEBASE_AUDIT.md`, `docs/superpowers/plans/2026-10-21-repair-progress.md`.

## Global Constraints

- CLAUDE.md security invariants hold: capability gate before every tool call; `Tainted<T>` never stripped; audit log append-only; HMAC chain verifies.
- No audit-log schema changes. No new features beyond making existing ones work.
- Dashboard stays a single self-contained HTML file: no CDN, no build step, no JS framework (loopback console, CSP/hardening in `web_auth::harden`).
- Gate before every commit: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --all-targets`.
- Every bug fix ships with one test that fails without it.
- Do not touch `codex/` or root `src/`.

## Evidence gathered (live run, 2026-10-29, release binary, isolated XDG dirs, no daemon)

| # | Symptom | Root cause | Where |
|---|---|---|---|
| E1 | **Generate plan → "Finding not found. Run a collection first."** for every finding shown | Without the daemon, `collect_live_snapshot` runs `run_monitor_cycle` in memory and never persists; `api_generate_plan` looks findings up only in `SnapshotStore` via `find_finding_by_id` | `web.rs:836-843`, `web.rs:483`, `main.rs:5854` |
| E2 | After a successful "loading" response the plan never appears | Client writes the placeholder into `finding.fixPlan`, never polls; the next WS snapshot rebuilds findings from `finding.fix_plan` (deterministic), erasing it | `dashboard.html:2176-2205`, `web.rs:1069` |
| E3 | Failed generation (timeout/auth/rate-limit) shows "Generating…" forever | `api_generate_plan` only serves `ready` records; any other persisted status re-queues and answers `loading` | `web.rs:488-533` |
| E4 | No provider: toast says "Plan metadata loaded", `plan_note` dropped | Client ignores `plan_note` / `disabled` | `dashboard.html:2198-2204` |
| E5 | Header says AI `api` / provider `groq` with no key configured | `provider_context()` ignores `llm_provider_usable` | `web.rs:1348-1368` |
| E6 | Daemon panel shows empty status, `refreshIntervalSecs: 0` when daemon is off | `daemon_summary()` returns blank defaults instead of `stopped` | `web.rs` `daemon_summary` |
| E7 | Every browser tab/WS reconnect triggers a full monitor cycle | `handle_ws` calls `collect_live_snapshot` per connection instead of reusing the last broadcast | `web.rs:441-447` |
| E8 | Button gives no feedback; double clicks queue duplicate requests; non-JSON 401 throws an uncaught error | No disabled/busy state; `response.json()` unguarded | `dashboard.html` `requestPlan`, `reviewedAction`, `refreshSnapshot` |

## Review Focus

- Finding id from a live (non-daemon) snapshot must resolve in `/api/plan`, `/api/service-action` reviews, and any id-keyed endpoint — tested in Task 1.
- Plan state must survive WS snapshot ticks and page reload — tested in Task 2.
- Failed generation must surface the error and allow Retry, never spin forever — tested in Task 2.
- Session expiry mid-action (401) must show the pairing dialog, not a JS exception — tested in Task 3.
- Narrow (≤720px) viewport and keyboard-only use must keep every action reachable — checked in Task 6.

---

## Phase A — fix proven-broken web flows

### Task 1: Live findings resolvable by the plan endpoint (E1)

**Files:** Modify `helm-cli/src/web.rs` (`WebState`, `collect_live_snapshot`, `api_generate_plan`). Test: `helm-cli/tests/web_plan.rs` (new; copy `Server` + pairing helper from `web_security.rs`).

**Interfaces:**
- Produces: `WebState.live_findings: Mutex<HashMap<String, Finding>>` (replaced wholesale on every `collect_live_snapshot`); `fn lookup_finding(state: &WebState, conn: &rusqlite::Connection, id: &str) -> Option<Finding>` — live map first, then `crate::find_finding_by_id`.

- [x] Step 1: Write `plan_endpoint_resolves_live_snapshot_finding`: start `helm web` with isolated XDG dirs and no daemon, pair, GET `/api/snapshot`, POST `/api/plan` with `findings[0].id`; assert response has no `error` key and `plan_status ∈ {"disabled","loading","ready"}`.
- [x] Step 2: Run `cargo test -p helm-cli --test web_plan` → FAIL with "Finding not found".
- [x] Step 3: Add `live_findings`, populate it in `collect_live_snapshot` for both daemon and live branches, use `lookup_finding` in `api_generate_plan`.
- [x] Step 4: Re-run → PASS. Gate. Commit `fix(web): plan endpoint resolves findings from live snapshots`.

### Task 2: Plan lifecycle visible and retryable (E2, E3, E4)

**Files:** Modify `helm-cli/src/web.rs` (`api_generate_plan`, finding view struct at `web.rs:219` area, snapshot builder `web.rs:1069`), `helm-cli/src/web/dashboard.html` (`requestPlan`, `incidentDetailCard`). Test: `helm-cli/tests/web_plan.rs`.

**Interfaces:**
- Consumes: `lookup_finding` (Task 1); `TroubleshootingPlanStore::latest_for_fingerprint`; `persist_llm_plan_status` statuses (`loading|ready|timeout|auth|malformed|rate_limited|…` as stored in `dashboard_plan_status`).
- Produces: `/api/plan` JSON adds `plan_error: Option<String>`; `plan_status` is one of `ready | loading | failed | disabled`. `failed` re-queues only when the request body carries `"retry": true` (`PlanRequest.retry: bool`, `#[serde(default)]`). Snapshot finding view adds `planStatus: Option<String>` and `aiPlan: Option<String>` (from the latest plan record for that fingerprint, via `web_plan_summary`).

- [x] Step 1: Tests: `failed_plan_reports_error_and_retry_requeues` (seed a plan row with status `timeout` for the finding's fingerprint in the server's DB before start, assert `plan_status=="failed"` and `plan_error` non-empty; with `retry:true` assert `loading`); `snapshot_carries_ready_ai_plan` (seed `ready` row, assert snapshot finding has `aiPlan`).
- [x] Step 2: Run → FAIL.
- [x] Step 3: Server: map non-ready/non-loading statuses to `failed`; add `planStatus`/`aiPlan` to snapshot. Client: button enters busy state (`disabled`, label "generating…"); while `loading`, poll `/api/plan` every 3 s, stop at 120 s with a "still generating — check back" toast; render "AI plan" as its own block (separate from deterministic "Fix plan") from `aiPlan`; `failed` shows `plan_error` + a Retry button sending `retry:true`; `disabled` shows `plan_note` inline with the `helm config` hint.
- [x] Step 4: Re-run → PASS. Manual: no-provider and Ollama runs in browser. Gate. Commit.

### Task 3: Truthful header state and robust client actions (E5, E6, E8)

**Files:** `helm-cli/src/web.rs` (`provider_context`, `daemon_summary`), `dashboard.html` (`authenticatedFetch`, `reviewedAction`, `refreshSnapshot`). Test: `web_plan.rs`.

- [x] Step 1: Test `snapshot_reports_unconfigured_ai_and_stopped_daemon`: isolated config with no key → `aiState=="off"`; no daemon → `daemon.status=="stopped"` and `refreshIntervalSecs` equals the configured web refresh.
- [x] Step 2: FAIL → implement (`provider_context` takes `&SecretsStore` and uses `llm_provider_usable`) → PASS.
- [x] Step 3: Client: one `jsonOrError(response)` helper used by every fetch — on 401 show login and return `{error:'Session expired — enter a new pairing code'}`; on non-JSON return `{error: \`HTTP ${status}\`}`. Every action button disables while in flight.
- [x] Step 4: Gate. Commit.

### Task 4: One collection per interval, not per tab (E7)

**Files:** `helm-cli/src/web.rs` (`WebState.last_snapshot: RwLock<Option<String>>`, `handle_ws`, background loop, `api_snapshot`).

- [x] Step 1: Test `second_websocket_does_not_trigger_collection`: count `snapshot_source`/`collectedAt` — two `/api/snapshot` calls inside one interval return the same `collectedAt`.
- [x] Step 2: FAIL → background loop stores into `last_snapshot`; `handle_ws`/`api_snapshot` serve it (collect only if empty); `/api/refresh` still forces a collection → PASS. Gate. Commit.

## Phase B — verification sweep (produces new tasks)

### Task 5: Feature-by-feature defect ledger

**Files:** Create `docs/FEATURE_MATRIX.md`; append tasks to this plan.

Drive every feature as a user would and record `feature | frontend | expected | observed | PASS/FAIL | evidence`. Use isolated XDG dirs and the release binary; web via `webcmd-browser` (Playwright) with screenshots; TUI via the existing pty harness pattern in `tests/demo/drive_agent.py`.

Checklist (minimum):
- [x] Web: pairing/expiry, each of the 10 tabs with empty and populated data, search, keyboard (j/k/Enter/Esc/1–0/Alt+G/F5), service journal/restart/stop review flow, change rollback (supported + unsupported), fleet tab with 0 and 2 hosts, pipeline webhook ingest shown in UI, log pattern drill-down.
- [x] CLI: every subcommand in `helm --help` with `--json` where offered; `--db` honored (audit F22); `explain`, `plan`, `apply-plan`, `audit verify`, `run`, `daemon start/stop/status`, `config` round-trip.
- [x] TUI: every keybinding in `keybindings.rs`, plan generation and its rate-limit retry (audit F20 noted the countdown never retries — verify), assistant run/approve/deny/cancel, remote attach.
- [x] Cross-frontend: a plan generated in TUI appears in web and vice versa.

Each FAIL becomes `Task N` here in the Task 1 shape (failing test → root-cause fix → gate → commit). Order: data loss/security > broken action > misleading state > cosmetic.

## Phase C — web UI redesign

### Task 6: Design pass on `dashboard.html`

**Skills (in order):** `impeccable` (critique/audit current UI first) → `design-taste-frontend` (direction) → `frontend-design` (implementation) → `web-design-guidelines` (final compliance review).

**Files:** `helm-cli/src/web/dashboard.html` only.

- [x] Step 1: Screenshot every tab at 1440px and 390px (before). Run impeccable audit; write findings to `docs/web-ui-audit.md`.
- [x] Step 2: Pick direction: dense operator console (incident queue + detail), not a marketing dashboard. Keep token system on `:root`; reduce the 39 inline `style=""` uses to classes; one type scale, one spacing scale, severity colors that pass WCAG AA on the bg.
- [x] Step 3: Implement: clear hierarchy for the incident detail (what broke → evidence → verify → fix/AI plan with explicit state chip), plan button moved into the incident detail where its context lives (header copy keeps Alt+G), proper empty/loading/error states for every tab, visible focus rings, `prefers-reduced-motion`, light theme via `prefers-color-scheme`.
- [x] Step 4: Re-screenshot (after), keyboard-only walkthrough, 720px/390px check, `web-design-guidelines` review; fix findings.
- [x] Step 5: Re-run all `web_*` tests + gate. Commit `feat(web): console redesign`.

### Task 7: Final verification

- [x] `scripts/verify-demo.sh` and full gate; update `docs/VERIFICATION.md` with the matrix results; every FEATURE_MATRIX row PASS or explicitly documented as out of scope.


## Continuation defects found in Task 5

### Task 8: Provider connection failures remain distinct from unsafe/unparseable plans

- [x] RED: `unreachable_provider_reports_connection_failure` reports an unparseable answer for a refused TCP connection.
- [x] Add `provider_unavailable` without schema changes; surface actionable connection/endpoint guidance in web and TUI and preserve it across reloads.
- [x] Final workspace gate and verification matrix.

### Task 9: Browser input and action ownership

- [x] RED: `tests/demo/dashboard.test.cjs` reproduces Enter overriding native controls, shortcuts escaping pairing, duplicate reviewed actions, and missing immediate plan state.
- [x] Gate keyboard shortcuts to the dashboard, preserve focus on render, keep pending request ownership outside replaced DOM nodes, show immediate loading and contextual plan controls.
- [x] Confirm actual browser keyboard/filter/re-render behavior; full gate.

### Task 10: Restored TUI plan recovery

- [x] RED: persisted rate-limit plan has no retry deadline; orphaned loading row never becomes retryable.
- [x] Reconstruct retry deadline from persisted update time; surface loading older than 120 seconds as timeout.
- [x] Final regression suite and cross-frontend state checks.
