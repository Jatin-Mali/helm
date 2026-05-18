---
phase: 3
slug: m3-fleet-multi-host
status: draft
tool: ratatui
created: 2026-05-18
---

# Phase 3 — M3: Fleet (Multi-Host) UI Design Contract

> Ratatui TUI design contract for multi-host fleet dashboard. Specifies layout, colors, widgets, key bindings, and terminal interaction patterns.

---

## Design System

| Property | Value |
|----------|-------|
| Tool | Ratatui 0.27+ |
| Terminal | ANSI 256-color, 80×24 minimum |
| Font | Monospace (user's terminal) |
| Theme | Dark-on-light ANSI palette |
| Layout Engine | Ratatui Flex + Rect constraint system |

---

## Color Palette (Ratatui ANSI)

**Source:** Locked in M1 ([linked-purring-hearth.md](../../REQUIREMENTS.md#color-semantics), D2–D3).

| Role | Ratatui Color | Hex Equivalent | Usage |
|------|----------------|----------------|-------|
| Dominant (60%) | `Color::Reset` / `Black` | #1e1e1e | Terminal background, status bar background |
| Secondary (30%) | `Color::DarkGray` | #808080 | Pane dividers, status bar border, disabled text |
| Accent (10%) | `Color::Cyan` | #00d4ff | Selection highlight, active tab underline, "UP" status |
| Critical | `Color::Red` | #ff0000 | CRIT badge, error text, "DOWN" status |
| Warning | `Color::Yellow` | #ffff00 | WARN badge, warning text |
| Info | `Color::Blue` | #0000ff | INFO badge, informational text |
| Success | `Color::Green` | #00ff00 | "UP" status (primary), healthy indicators |

**Accent Reserved For:**
- Selection highlight (row background in Fleet tab)
- Active tab underline (Alerts, Services, Fleet, etc.)
- Host reachability "UP" status in Fleet table
- Link text in footer (e.g., `[a]apply`, `[q]quit`)

---

## Layout Contract (Terminal 80×24 minimum, 120×40 tested)

### Overall Structure

```
┌─────────────────────────────────────┐
│ fleet: 12/12 up | [a]apply [q]quit │  ← Status bar (1 line, height=1)
├────────────┬──────────────────────┤
│   LEFT     │      RIGHT           │  ← Tabs: Alerts | Services | Fleet | ...
│  PANE      │      PANE            │     (7–8 tabs, no tab bar UI yet)
│ (Severity  │  (Fleet table:       │
│  Grouped   │   Name|Status|CRIT   │
│  Findings) │   |WARN|INFO|Last)   │  ← Content area (height=available, width=split)
│            │                      │
│            │                      │
└────────────┴──────────────────────┘
```

**Constraints:**
- **Status bar:** Always 1 line at top
- **Left pane (Findings queue):** Flex(1) width = 30% of available
- **Right pane (Tab content):** Flex(2) width = 70% of available
- **Minimum terminal:** 80 chars wide × 24 lines tall
- **Tested:** 120×40 (preferred for development, defined in snapshot tests)

**Layout Logic:**
```rust
// Pseudo-code for ratatui constraint allocation
let total_height = area.height;
let status_bar_height = 1;
let content_height = total_height.saturating_sub(status_bar_height);

// Horizontal split
let left_width = Constraint::Percentage(30).apply(area.width);
let right_width = area.width.saturating_sub(left_width);
```

---

## Widgets

### Fleet Tab (Slice 3.4 – Complete)

**Widget Type:** Ratatui `Table`

**Columns:**
| Column | Constraint | Header | Example Values |
|--------|-----------|--------|-----------------|
| Name | Percentage(30) | "Name" | "prod-01", "staging-web-2" |
| Status | Percentage(10) | "Status" | "UP" (green), "DOWN" (red) |
| CRIT | Percentage(12) | "CRIT" | "5", "0" |
| WARN | Percentage(12) | "WARN" | "12", "2" |
| INFO | Percentage(12) | "INFO" | "18", "8" |
| Last | Percentage(24) | "Last" | "5s ago", "2m ago", "1h ago", "N/A" |

**Cell Colors:**
- Header: `Style::default().fg(Color::Cyan)` (accent color)
- Status "UP": `Style::default().fg(Color::Green)`
- Status "DOWN": `Style::default().fg(Color::Red)`
- Selected row: `Style::default().bg(Color::DarkGray)` (secondary color as background)
- Number cells: Default `Style`
- Last refresh: Default `Style`

**Row Selection:**
- Keyboard: Arrow Up/Down navigate rows
- Visual: Selected row receives background color (DarkGray)
- Max visible rows: Constrained by terminal height (auto-scroll if fleet > visible)

**Behavior:**
- Selecting a row + pressing Enter: Switch `active_remote` to that host UUID, trigger dashboard refresh
- No multi-select; single cursor per tab session
- Cursor resets to row 0 if fleet count changes between refreshes

---

### Status Bar (Top, 1 line)

**Content (left to right):**
```
fleet: N/N up | [a]apply [q]quit
```

**Components:**
1. Fleet count: `"fleet: " + reachable_count + "/" + total_count + " up"`
   - Color: Default text
   - Visibility: Always shown when Fleet tab exists; updated every refresh
   - Example: "fleet: 11/12 up" (one host down)

2. Divider: `" | "` (default color)

3. Keybinds (footer):
   - `[a]apply` — Cyan (accent) for `a`, default for brackets and text
   - `[q]quit` — Cyan (accent) for `q`, default for brackets and text
   - Pattern: `[key]label` where key is accent, label is default

**Position:** Top of screen (always visible, never scrolls)

---

## Key Bindings (Fleet Tab Context)

| Key | Action | Scope | Note |
|-----|--------|-------|------|
| `Up` arrow | Move cursor up 1 row | Fleet tab only | Wraps to row 0 if at top |
| `Down` arrow | Move cursor down 1 row | Fleet tab only | Clamps to last row if at bottom |
| `Enter` | Switch active_remote to selected host, refresh dashboard | Fleet tab + Alerts tab | Triggers full refresh, selection sticky |
| `[a]` | Apply selected plan (if available) | Global (all tabs) | Capability-gated elsewhere |
| `[q]` | Quit HELMOPS | Global | Exits cleanly, no confirmation needed |
| `Tab` / `Shift+Tab` | Cycle tabs (Alerts → Services → Fleet → Logs → ...) | Global | Tab order: Alerts, Services, Processes, Logs, Network, Disk, Changes, Fleet |

**Notes:**
- **No mouse support** in M3 (TUI tradition, add in future if needed)
- **Tab navigation:** Cycles through all OpsTab variants; Fleet is variant #8
- **Selection pinning:** If active_remote switches away from a Fleet row, row selection resets to None until user re-selects

---

## Copy & Messaging

### Fleet Tab Table

| Context | Copy |
|---------|------|
| Empty state (no remotes) | *Table renders empty; "No hosts configured" implied by 0 rows* |
| Column headers | "Name", "Status", "CRIT", "WARN", "INFO", "Last" |
| Status UP | "UP" (green text) |
| Status DOWN | "DOWN" (red text) |
| Last refresh | "Xs ago", "Nm ago", "Nh ago", or "N/A" (no refresh yet) |
| Host unreachable | Row reachable=false, Status="DOWN", last_refresh="N/A" |

### Status Bar

| Context | Copy |
|---------|------|
| Fleet count | "fleet: 12/12 up" (all reachable) |
| Fleet degraded | "fleet: 11/12 up" (one or more down) |
| Keybind labels | "[a]apply", "[q]quit" |

### Data Structures (Internal)

**FleetHostStatus struct** (populated by background refresh, Slice 3.3):
```rust
pub struct FleetHostStatus {
    pub host_id: Uuid,           // Stable host identifier
    pub name: String,            // Display name (e.g., "prod-01")
    pub reachable: bool,         // SSH connection success
    pub crit: usize,             // Count of CRIT findings on this host
    pub warn: usize,             // Count of WARN findings
    pub info: usize,             // Count of INFO findings
    pub last_refresh: Option<Instant>,  // Last successful collection time
    pub error: Option<String>,   // SSH error message if unreachable
}
```

---

## Database Schema (Snapshot Store)

**New Column (Migration 0011):**

| Column | Type | Default | Purpose |
|--------|------|---------|---------|
| host_id | TEXT | '' | UUID of the remote host (matches RemoteEntry.host_id) |
| — | — | — | — |

**Constraint:** Add index on `host_id` for multi-host lookups:
```sql
CREATE INDEX IF NOT EXISTS idx_snapshots_host_id ON snapshots(host_id);
```

**Backward Compat:** Existing snapshots without host_id will have empty string (no crash, filtering works).

---

## Remote Entry Configuration (remotes.toml)

**Format:** TOML with credential abstraction (Slice 3.1, complete).

```toml
[[remotes]]
host_id = "550e8400-e29b-41d4-a716-446655440000"
name = "prod-01"
host = "prod-01.example.com"
port = 22
user = "ops"
ssh_opts = "-o BatchMode=yes -o ConnectTimeout=10"
credential = "SshAgent"  # or "KeyFile /path/to/key"

[[remotes]]
host_id = "550e8400-e29b-41d4-a716-446655440001"
name = "staging-web"
host = "staging.example.com"
port = 22
user = "ops"
ssh_opts = "-o BatchMode=yes"
credential = { KeyFile = "/home/ops/.ssh/staging_key" }
```

**Serialization (Slice 3.1, complete):**
- `Credential::SshAgent` serializes as string `"SshAgent"`
- `Credential::KeyFile(PathBuf)` serializes as inline struct `{ KeyFile = "/path" }`
- Default: `SshAgent` (via `#[serde(default)]` on RemoteEntry field)

---

## CLI Integration (Slice 3.2)

### `helm monitor --format json`

**Flag:** Already exists in MonitorArgs struct (line 658, main.rs).

**Output Format:** JSON array of finding summaries.

**Example:**
```json
[
  {
    "id": "finding-uuid-1",
    "severity": "CRIT",
    "title": "High CPU load",
    "host_id": "550e8400-e29b-41d4-a716-446655440000"
  },
  {
    "id": "finding-uuid-2",
    "severity": "WARN",
    "title": "Disk 80% full",
    "host_id": "550e8400-e29b-41d4-a716-446655440000"
  }
]
```

---

## Interaction Flows

### Flow 1: View Fleet Status (Slice 3.4)

1. User presses Tab key to cycle to Fleet tab
2. Fleet tab renders: table with N host rows
3. Status bar shows "fleet: N/M up"
4. User presses Up/Down to navigate rows
5. User presses Enter on a row
6. **Effect:** active_remote switches to that host UUID → dashboard refreshes → Alerts tab shows that host's findings

### Flow 2: Parallel SSH Collection (Slice 3.2)

1. Background refresh tick fires (every 60s, configurable in thresholds.toml)
2. Fleet registry spawns JoinSet: 20 concurrent SSH tasks
3. Each task:
   - SSH to remote host
   - Exec `helm monitor --format json`
   - Parse JSON array
   - Tag each finding with host_id
   - Return `(Uuid, Result<Vec<FleetFinding>>)`
4. Results collected, stored in `DashboardData.fleet_hosts`
5. TUI re-renders Fleet tab with updated counts
6. Status bar updates "fleet: X/Y up"

**Timeout:** 30s per host (prevent hang on slow/dead hosts)

**Concurrency cap:** 20 tasks max (benchmark target)

**Performance target:** 20-host refresh ≤2s p99

---

## Selection Stability (M1 Invariant)

**Applies to Fleet tab:**
- User selects row N (e.g., host "prod-01")
- Background refresh ticks, fleet_hosts vec rebuilds
- **Selection remains on row N** (positional pinning)
  - If row N still exists: cursor stays there
  - If row N deleted (host removed from config): cursor resets to None
- **Active host changes only on Enter key** (not on refresh)

---

## Snapshot Test Specification

**Test name:** `test_render_fleet_tab_snapshot`

**Setup:**
- Create 3 mock FleetHostStatus entries:
  - host1: name="prod-01", reachable=true, crit=2, warn=5, info=10, last_refresh=5s ago
  - host2: name="staging-web", reachable=true, crit=0, warn=1, info=3, last_refresh=2m ago
  - host3: name="backup-srv", reachable=false, crit=0, warn=0, info=0, last_refresh=None

- Render at 120×40 terminal size
- Selected row = Some(0)

**Assertions:**
- Headers present: "Name", "Status", "CRIT", "WARN", "INFO", "Last"
- Host names appear: "prod-01", "staging-web", "backup-srv"
- Status values: "UP" (green), "UP" (green), "DOWN" (red)
- Finding counts: "2", "5", "10", "0", "1", "3" all present
- Timestamps: "5s ago", "2m ago", "N/A"
- Selected row has background highlight

**Pass criterion:** All strings and colors render without panic; visual output matches inline snapshot.

---

## Accessibility & Terminal Compatibility

| Requirement | Implementation |
|-------------|-----------------|
| Monochrome mode | Use bold/dim instead of color (e.g., bold for UP, dim for DOWN) |
| High contrast | Cyan (accent) + Red/Green + Black/Gray provide 4:1+ contrast |
| Screen reader | Not applicable (TUI only; no ARIA roles) |
| Keyboard only | All navigation via arrow keys + Enter; no mouse required |
| 80×24 minimum | Layout tested; columns may wrap or truncate on narrow terminals |

---

## Deferred & Out of Scope

**Deferred to post-M3:**
- Per-host sparklines in Fleet tab (requires ring buffer per host; post-M5)
- Cross-host finding correlation (post-M4, requires K8s/infra context)
- Tab collapse from 7 → 5 variants (M1 Slice 1.3 deferred; Fleet is 8th variant for now)
- Password credential type (security hardening post-M3)
- Secrets.toml integration for credential storage

**Never in scope:**
- Web frontend (TUI only)
- Mouse support (keyboard-driven)
- Custom fonts or emoji

---

## Pre-Population From Upstream

| Source | Section | Decision |
|--------|---------|----------|
| CONTEXT.md (M3 discuss) | D-M3-1 | Credential model: SshAgent \| KeyFile only |
| CONTEXT.md (M3 discuss) | D-M3-2 | Remote collection: SSH + `helm monitor --format json` |
| CONTEXT.md (M3 discuss) | D-M3-3 | Fleet tab position: variant #8 in OpsTab |
| CONTEXT.md (M3 discuss) | D-M3-4 | Fleet aggregation: per-host summary table |
| CONTEXT.md (M3 discuss) | D-M3-5 | Parallel refresh: JoinSet bounded to 20, non-blocking |
| PATTERNS.md | Color palette | ANSI colors locked in M1 (Red/Yellow/Green/Cyan) |
| 03-01-SUMMARY.md | Credential enum | Implemented ✓ (Slice 3.1 complete) |
| 03-04-SUMMARY.md | Fleet tab UI | Implemented ✓ (Slice 3.4 complete) |

---

## Verification Gates

### Per-Task Gate (Before Slice Merge)

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

### Slice 3.4 Gate (Fleet Tab)

```bash
cargo test test_render_fleet_tab_snapshot
```

### M3 RC Gate (Post-Slice 3.5)

```bash
# 20-host mock SSH benchmark
cargo test fleet_parallel_refresh_20_hosts -- --ignored

# HMAC chain verifies after multi-host episode
cargo test hmac_chain_verifies_after_fleet_episode

# Full gate
cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

---

## Checker Sign-Off

- [ ] Dimension 1 Copywriting: Fleet table headers, status values, keybinds, status bar text
- [ ] Dimension 2 Visuals: Layout constraints, Fleet tab table, row selection highlight, terminal compatibility
- [ ] Dimension 3 Color: Ratatui ANSI palette, accent usage (UP/selection), destructive (DOWN/CRIT)
- [ ] Dimension 4 Typography: Monospace only, single size (terminal cell size, no scaling)
- [ ] Dimension 5 Spacing: Ratatui Flex/Percentage constraints, column widths, 1-line status bar
- [ ] Dimension 6 Registry Safety: Not applicable (no shadcn; Ratatui only)

**Approval:** pending
