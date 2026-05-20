# HELMOPS TUI & Web Bug Fix Plan
> **Goal:** Fix all TUI rendering bugs, remove broken features, make the dashboard production-ready.

## Problems Found (Phase 0 inspection)

### Bug 1: DUPLICATE FOOTER
- `render_dashboard()` at line 6572 calls `render_ops_footer` at `vert[4]`
- Main render function at line 6425 ALSO calls `render_ops_footer` at `vertical[1]`
- Result: TWO footer lines rendered on screen
- **Fix:** Remove footer from `render_dashboard()`. Footer should only render once in main render function.

### Bug 2: BROKEN AUTO-REFRESH
- Tick handler spawns a tokio task that runs `run_monitor_cycle` but DISCARDS results
- No DB persistence, no dashboard data update
- `DashboardRefresh` event is sent but doesn't actually refresh data
- **Fix:** Remove broken auto-refresh code. The daemon handles periodic collection. TUI refresh should just read latest from DB.

### Bug 3: BROKEN PLAN GENERATION  
- `spawn_background_plan()` creates a oneshot channel, sends `Loading`, returns rx
- But rx is NEVER read — the code at line 2201-2208 was supposed to poll it but doesn't
- `generate_dashboard_plan()` calls LLM — blocking call, never actually wired
- **Fix:** Remove broken plan generation from TUI (move to web later). Keep Alt+G as a "not implemented yet" toast.

### Bug 4: WEB COMMAND IS STATIC HTML MOCK
- `helm web` serves the exact HTML file via `include_str!()`
- WebSocket pushes live JSON but the HTML has NO JavaScript to receive it
- This is NOT a real web dashboard — it's a static mock
- **Fix:** Remove `helm web` command entirely. Keep the the HTML as a design reference for future implementation.

### Bug 5: DAEMON TOGGLE UNTESTED
- `toggle_daemon()` calls `daemon_start_core()` / `daemon_stop_core()` 
- These spawn child processes and write state files
- No verification that they actually work from within the TUI event loop
- **Fix:** Remove Ctrl+D daemon toggle. Keep daemon control via CLI (`helm daemon start/stop`).

---

## Tasks (execute in order, one at a time)

### Task 1: Remove duplicate footer from render_dashboard
**File:** `helm-cli/src/tui.rs`
**Change:** Remove line 6572 (`render_ops_footer(app, vert[4], buf)`) and remove vert[4] constraint
**Verify:** `cargo test -p helm-cli` — all dashboard tests pass

### Task 2: Remove broken auto-refresh from Tick handler  
**File:** `helm-cli/src/tui.rs`
**Change:** Remove lines 2112-2134 (the whole auto-refresh block in UiEvent::Tick)
**Verify:** `cargo check -p helm-cli` — compiles, `cargo test -p helm-cli` — passes

### Task 3: Remove broken plan generation from TUI
**File:** `helm-cli/src/tui.rs`
**Change:** 
- Remove `spawn_background_plan()` function (lines 5459-5466)
- Remove `needs_plan_generation` polling (lines 2201-2208) 
- Change Alt+G to show toast "Plan generation will be available in helm web"
**Verify:** `cargo test -p helm-cli` — passes

### Task 4: Remove helm web command
**Files:**
- `helm-cli/src/web.rs` — delete file
- `helm-cli/src/main.rs` — remove `mod web`, `Command::Web`, `WebArgs`, dispatch, is_known_command entry
- `helm-cli/Cargo.toml` — remove axum, tower-http, tower deps
- `Cargo.toml` (workspace) — remove axum, tower-http, tower deps
**Verify:** `cargo check -p helm-cli` — compiles, `cargo test --workspace` — 658 tests pass

### Task 5: Remove Ctrl+D daemon toggle from TUI
**File:** `helm-cli/src/tui.rs`
**Change:** Remove lines 2434-2438 (Dashboard mode Ctrl+D check), remove `toggle_daemon()` function
**Verify:** `cargo test -p helm-cli` — passes

### Task 6: Final verification
- `cargo test --workspace` — all 658 pass
- `cargo build --release -p helm-cli` — zero warnings
- `cargo fmt --check` — clean
- `./target/release/helm --help` — verify commands
- Summary to user
