# Phase 6: M6 — AI Excellence — PLAN

**Goal:** Operator selects a finding; a fix plan is already waiting.

**Depends on:** Phase 1 (TUI), Phase 5 (Alerting lifecycle)

---

## Slices

### Slice 6.1 — Fingerprint-keyed plan cache + auto-trigger

**Files:** `helm-cli/src/tui.rs`

**Tasks:**
- T6.1.1: Add `plan_cache: HashMap<String, PlanStatus>` to `DashboardState`. Initialize to empty in `DashboardState::new()`.
- T6.1.2: Add `fn auto_generate_plan_for_fingerprint(&mut self, fp: &str)` — checks `plan_cache` for a hit; if hit, restores `active_plan` from cache; if miss and LLM configured, calls `generate_dashboard_plan()` path without requiring manual Alt+G. Cache is keyed by fingerprint.
- T6.1.3: Hook into selection change: after `selected_fingerprint` is set (in `set_dashboard_selection_from_visible_index`), call `auto_generate_plan_for_fingerprint`.
- T6.1.4: After `pending_plan_rx` resolves (the poll in `tick()`), store result in `plan_cache` keyed by the active plan's fingerprint.

**Verify:** Unit test — populate cache with Ready status for fingerprint "abc"; call auto_generate_plan; assert active_plan is restored from cache without spawning a new task.

---

### Slice 6.2 — Cache-hit renders instantly; spinner on miss

**Files:** `helm-cli/src/tui.rs`

**Tasks:**
- T6.2.1: In the Overview detail pane FIX PLAN section render (near `DashboardView::Overview` detail draw), check `plan_cache.get(selected_fingerprint)`:
  - `Some(PlanStatus::Ready { narrative, fix_steps })` → render narrative + steps inline
  - `Some(PlanStatus::Loading { .. })` → render "⏳ Generating fix plan…" line
  - `None` → render "  [Alt+G to generate plan]" hint
- T6.2.2: In `DashboardView::TroubleshootPlan` render path (existing), do the same cache lookup to stay in sync with the cache.

**Verify:** Snapshot test: `render_fix_plan_section` with cache=Loading produces "⏳" line; cache=Ready produces step count line.

---

### Slice 6.3 — `[a]` hotkey (no modifier) in Dashboard mode

**Files:** `helm-cli/src/tui.rs`

**Tasks:**
- T6.3.1: In the Dashboard mode `Char` key handler (near the `Alt+A` handler at line ~1787), add:
  ```
  KeyCode::Char('a') if self.mode == AgentMode::Dashboard
      && key.modifiers.is_empty()
      && self.dashboard.active_plan.as_ref()
             .map_or(false, |p| matches!(p.status, PlanStatus::Ready { .. })) => {
      self.apply_dashboard_plan().await?;
      return Ok(false);
  }
  ```
- T6.3.2: Update the status-bar hint in `render_dashboard_statusbar` to show `a:apply` when a Ready plan exists.

**Verify:** Integration smoke — select a finding, cache inject a Ready plan, press 'a', verify `apply_dashboard_plan` is called.

---

### Slice 6.4 — Plan quality snapshot test

**Files:** `crates/helm-monitor/src/troubleshoot.rs` (new `#[cfg(test)]` block)

**Tasks:**
- T6.4.1: Add `#[test] fn plan_quality_structure()` that constructs a `TroubleshootingPlan` from a synthetic `Finding` (use `plan_from_finding`) and asserts:
  - At least one `PlanStep` is produced.
  - All `PlanStep::command` values pass `CommandValidator::is_safe` (no `rm -rf /`, no `dd if=/dev/zero`, no unconfirmed destructive ops).
  - Plan source is `PlanSource::Finding`.
- T6.4.2: Add `#[test] fn parse_llm_response_numbered_steps()` that feeds a fixture LLM response (3 numbered steps with backtick commands) to `parse_llm_response` and asserts:
  - 3 steps returned.
  - Each step has non-empty `command` and `purpose`.
  - No step has `RiskLevel::Critical` without a rollback.

**Verify:** `cargo test plan_quality_structure parse_llm_response_numbered_steps` both pass.

---

## Gate (per slice)

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

## Milestone RC Gate

```bash
cargo test deterministic_100_run -- --ignored
```

## Acceptance Criteria (from ROADMAP.md)

1. ✓ On fingerprint change, background tokio task spawns LLM call; result cached by fingerprint in plan_cache
2. ✓ Cache hit renders plan instantly; cache miss shows "⏳ Generating…"; ≥80% on second open
3. ✓ Plans capability-gated and taint-checked (existing apply path enforces this)
4. ✓ `[a]` hotkey in Dashboard → apply_dashboard_plan → capability gate + taint check + audit append
5. ✓ Plan quality snapshot tests pass
