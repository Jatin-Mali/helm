# HELMOPS Web Dashboard — Implementation Plan
> Canonical revision: 2026-05-21
> Based on: Feature Inventory, Architecture Design, UX/UI Spec
> Total phases: 5 | Total tasks: ~50 | Estimated: 4-6 focused sessions

---

## Phase 0: Web Server Foundation (Infrastructure)

**Goal:** Working web server with live WebSocket data, serving real HTML dashboard shell.

### Task 0.1: Add axum dependencies
- Add `axum = { version = "0.8", features = ["ws"] }` to workspace Cargo.toml
- Add to helm-cli/Cargo.toml
- `cargo check -p helm-cli` — compiles
- **Verify:** `cargo check` passes

### Task 0.2: Create web server module skeleton
- Create `helm-cli/src/web.rs`
- `pub async fn serve(db_path: PathBuf, port: u16) -> Result<()>`
- Background collector: tokio::spawn loop running MonitorProfile::Quick every 5s
- Broadcast channel for WebSocket fan-out
- Axum router with routes: `/`, `/ws`, `/api/snapshot`
- **Verify:** `cargo check -p helm-cli` passes

### Task 0.3: Wire `helm web` CLI command
- Add `Web(WebArgs)` to Command enum in main.rs
- Add `WebArgs { port: u16 }` struct with clap
- Add dispatch in run()
- Add "web" to is_known_command()
- **Verify:** `cargo check -p helm-cli` passes, `helm --help` shows web command

### Task 0.4: Build LiveSnapshot JSON serialization
- Define LiveSnapshot struct with all fields from schema
- Implement `collect_live_snapshot()` — calls existing monitor cycle
- Serialize to JSON using serde
- **Verify:** `cargo test -p helm-cli` passes

### Task 0.5: Create real dashboard HTML shell
- Create `helm-cli/src/web/dashboard.html` — NOT the static mock
- Dark theme CSS matching design spec
- HTML structure: header, status strip, tab bar, queue sidebar, main content, footer
- Embed static assets via `include_str!()` in web.rs
- **Verify:** Browser shows styled layout at localhost:9120, no JavaScript yet

### Task 0.6: WebSocket live data push
- Implement ws_handler: upgrade, send initial snapshot, stream updates
- Background collector sends to broadcast channel
- Test: connect via wscat or browser console, receive JSON
- **Verify:** `wscat -c ws://localhost:9120/ws` shows live snapshot JSON

---

## Phase 1: Core Dashboard (Data → DOM)

**Goal:** Live dashboard with real data, finding queue, and detail panel.

### Task 1.1: JavaScript WebSocket client
- Connect to /ws on page load
- Parse incoming LiveSnapshot JSON
- Update DOM elements: header metrics, status strip badges, finding queue
- Handle reconnection with exponential backoff
- **Verify:** Dashboard updates every 5 seconds without page refresh

### Task 1.2: Header bar rendering
- Hostname, provider/model label, LIVE dot, clock, load/mem/cpu metrics
- Color-coded thresholds (green → yellow → red)
- Clock updates every second
- **Verify:** Header shows real data matching `helm monitor` output

### Task 1.3: Status strip rendering
- Severity badges: CRIT N, WARN N, INFO N, MUTED N
- Service failure chip: "svc N fail"
- Container chip: "ctr N/N"
- **Verify:** Badges update on each WebSocket message

### Task 1.4: Finding queue sidebar
- Render findings list grouped by severity
- Each item: severity icon, title, affected resource, domain
- Selected state: highlighted row
- Click handler: select finding, render detail
- Keyboard: ↑/↓ navigation
- **Verify:** Click a finding, detail panel appears

### Task 1.5: Finding detail panel — 10 sections
- WHAT BROKE: title, resource, timeline
- IMPACT: impact text
- WHEN IT HAPPENED: timestamps
- WHY IT HAPPENED: assumptions, missing data
- WHAT CHANGED BEFORE: correlated findings, drift
- EVIDENCE: evidence references with values
- HOW TO REPRODUCE: read-only checks
- RECOMMENDED FIX PLAN: fix steps with risk
- ROLLBACK: rollback instructions
- EXECUTION STATUS: lifecycle state
- **Verify:** Selecting a finding shows all 10 sections with real data

### Task 1.6: Tab bar rendering
- 8 tabs: ALERTS, SERVICES, PROCESSES, LOGS, NETWORK, DISK, CHANGES, FLEET
- Active tab highlighted with underline
- Click to switch tabs
- Keyboard: 1-8 switches tabs
- **Verify:** Clicking a tab switches the main content area

---

## Phase 2: Tab Content (All Monitoring Domains)

**Goal:** Every tab shows useful domain-specific data from the live snapshot.

### Task 2.1: Services tab
- Table: name, state (color-coded), description
- Filter: All | Failed | Running
- Click row: expand journal preview (last 20 lines)
- **Verify:** Matches `systemctl list-units` output

### Task 2.2: Containers tab
- Table: name, image, status, ports, health
- Filter: All | Running | Stopped | Unhealthy
- Click row: expand details (mounts, restart count, logs)
- **Verify:** Matches `docker ps` output

### Task 2.3: Processes tab
- Table: PID, name, CPU%, MEM%, state
- Sortable columns
- Auto-refresh shows changes
- **Verify:** Matches `ps aux --sort=-%cpu | head -20`

### Task 2.4: Network tab
- Listening ports table: port, protocol, process
- Active connections count
- DNS servers list
- Routes summary
- **Verify:** Matches `ss -tlnp` output

### Task 2.5: Disk tab
- Filesystems table: mount, usage bar, total, used, available
- Color-coded usage bars
- I/O stats if available
- **Verify:** Matches `df -h` output

### Task 2.6: Logs tab
- Journal error count
- Recent errors list (scrollable)
- Auth failures
- Kernel errors
- Search/filter input
- **Verify:** Matches `journalctl -p err -n 50` output

### Task 2.7: Changes tab
- Recent config modifications
- Files changed, timestamps, diff preview
- **Verify:** Shows files modified since last snapshot

### Task 2.8: Fleet tab
- Host list with status indicators
- Finding counts per host
- Click host → loads that host's findings
- **Verify:** Shows remote hosts from remotes.toml

---

## Phase 3: Advanced Features (Moat Features)

**Goal:** AI-powered troubleshooting, plan generation, command execution.

### Task 3.1: Plan generation — backend
- POST /api/plan endpoint
- Accepts finding_id
- Calls existing build_narrative_prompt + LLM
- Returns structured plan JSON
- **Verify:** curl POST returns plan with hypotheses

### Task 3.2: Plan generation — frontend
- Alt+G or "Generate Plan" button
- Loading state: spinner with "Generating troubleshooting plan..."
- Render hypothesis tree in detail panel
- Show command previews with risk badges
- **Verify:** Pressing Alt+G generates and displays a plan

### Task 3.3: Command preview with risk
- Each fix step shows: command text, expected effect, risk level, blast radius, rollback
- Color-coded risk badges (low=green, medium=yellow, high=red)
- **Verify:** Plan includes command previews matching the design

### Task 3.4: Apply plan — backend
- POST /api/apply endpoint
- Accepts step_id, requires explicit approval
- Executes via existing tool infrastructure
- Returns verification results
- Records audit entry
- **Verify:** Applying a step executes the command and returns output

### Task 3.5: Apply plan — frontend
- "Apply" button on each step
- Confirmation dialog: shows command, risk, blast radius
- "Approve" / "Deny" buttons
- Shows execution output in real-time
- **Verify:** Applying a step shows confirmation, executes, shows output

### Task 3.6: AI chat panel
- Collapsible chat panel (bottom or right side)
- Ask questions about system state
- Uses existing LLM infrastructure
- Context: current snapshot + selected finding
- **Verify:** Can ask "why is nginx failing?" and get contextual answer

### Task 3.7: Historical trend graphs
- Sparklines in header for CPU/mem/disk
- Toggle to full-size graphs
- Data from SQLite history
- Simple canvas or SVG rendering (no chart library)
- **Verify:** Shows 60-point history sparkline that updates

### Task 3.8: Alert notifications
- Webhook configuration UI
- Email alert settings
- Slack/Discord integration placeholder
- **Verify:** Configurable in UI, test notification works

---

## Phase 4: Polish & Production Readiness

**Goal:** Error handling, responsive design, accessibility, performance.

### Task 4.1: Loading states
- Skeleton screens for all panels
- Spinner for data-dependent operations
- Progressive rendering: header first, then queue, then detail
- **Verify:** Page loads with skeleton screens, resolves within 500ms

### Task 4.2: Error states
- Collection failure: red banner + retry button
- WebSocket disconnect: yellow banner + auto-reconnect
- LLM timeout: warning with retry option
- API error: specific error message, not raw JSON
- **Verify:** Kill the daemon, see error banner. Restart, see recovery.

### Task 4.3: Empty states
- No findings: "System healthy" with green check
- No services: "No systemd units found"
- No containers: "Docker/Podman not detected"
- No fleet hosts: "Add hosts in remotes.toml"
- **Verify:** Fresh system shows helpful empty states, not broken UI

### Task 4.4: Responsive design
- Desktop (≥1024px): Full layout
- Tablet (768-1023px): Collapsible sidebar
- Below 768px: Stacked layout, hamburger menu
- **Verify:** Resize browser window, layout adapts without breaking

### Task 4.5: Keyboard shortcuts
- Implement all shortcuts from UX spec
- `?` key shows shortcut overlay
- `Ctrl+K` opens command palette
- Focus management: Tab cycles through interactive elements
- **Verify:** Every shortcut works, documented in help overlay

### Task 4.6: Performance optimization
- DOM diffing: only update changed elements
- Virtual scrolling for large lists (>100 findings)
- Debounced WebSocket updates (max 1 render per 200ms)
- Lazy load tab content (only active tab)
- **Verify:** Dashboard stays responsive with 100+ findings

### Task 4.7: Accessibility
- ARIA labels on interactive elements
- Focus indicators visible
- Screen reader announcements for state changes
- Color contrast meets WCAG AA
- **Verify:** Lighthouse accessibility score > 90

### Task 4.8: Browser compatibility
- Chrome, Firefox, Edge: full support
- Safari: tested, any issues documented
- Mobile browsers: basic functionality
- **Verify:** Tested in at least Chrome + Firefox

---

## Phase 5: Ship

**Goal:** Tests, documentation, release packaging.

### Task 5.1: Integration tests
- Test web server starts and responds
- Test WebSocket connection and data flow
- Test API endpoints return valid JSON
- Test dashboard loads without JS errors
- **Verify:** `cargo test -p helm-cli` — new tests pass

### Task 5.2: End-to-end smoke test
- Start daemon: `helm daemon start`
- Start web: `helm web`
- Open browser, verify dashboard shows data
- Select finding, verify detail panel
- Generate plan, verify hypothesis tree
- **Verify:** Full user flow works end-to-end

### Task 5.3: Documentation
- README: web dashboard setup and usage
- docs/web-dashboard.md: complete user guide
- Keyboard shortcuts reference in help overlay
- API documentation for /api/* endpoints
- **Verify:** New user can follow docs and get dashboard running

### Task 5.4: Release build
- `cargo build --release -p helm-cli`
- Verify binary size, no warnings
- Test on clean system (no prior config)
- **Verify:** `./target/release/helm web` works on fresh install

### Task 5.5: Install script update
- install.sh includes web dashboard instructions
- First-run experience: `helm web` after `helm init`
- **Verify:** install.sh downloads and sets up correctly
