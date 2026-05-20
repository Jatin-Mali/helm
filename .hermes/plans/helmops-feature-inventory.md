# HELMOPS — Complete Feature Inventory & Product Specification
> Canonical revision: 2026-05-21 | Status: Pre-build planning
> Based on: PRD, TRD, ROADMAP, Rebirth Roadmap, competitor research, user feedback

---

## Competitive Landscape Summary

| Tool | Strength | Weakness (HELMOPS opportunity) |
|------|----------|-------------------------------|
| **Cockpit** | Web server mgmt, systemd, storage, networking GUI | No troubleshooting guidance, no AI, no fleet view |
| **Netdata** | Real-time metrics, ML anomaly detection, auto-dashboards | Metrics-only, no troubleshooting plans, no command previews |
| **Grafana** | Flexible dashboards, query language, ecosystem | Requires Prometheus setup, no built-in collectors, steep learning curve |
| **Datadog** | Enterprise APM, logs, traces, alerts | SaaS-only, expensive, no local-first, no command execution |
| **Beszel** | Lightweight, simple, agent-based | Too minimal — no detectors, no plans, no troubleshooting |
| **htop/btop** | Fast, terminal-native, zero config | Single-host, no persistence, no fleet, no web |
| **Portainer** | Container management GUI | Containers only, no system-level monitoring |
| **OpenCode** | Clean terminal UI, agent-friendly | AI coding tool, not a monitoring tool |

**HELMOPS moats (what no other tool does):**
1. **Evidence-backed findings** — every alert cites exact snapshot fields, not just thresholds
2. **Guided troubleshooting plans** — structured hypothesis trees with verification steps
3. **Command previews with blast radius** — shows exactly what a fix command will do before execution
4. **Read-only-first architecture** — safe to run anywhere, mutation requires explicit approval
5. **Local-first with zero dependencies** — no SaaS, no Prometheus, no Grafana required
6. **Typed system snapshot** — structured, queryable state of entire system at a point in time
7. **Audit trail for every action** — append-only, HMAC-chained, verifiable
8. **Works without AI, enhanced with AI** — detector engine runs locally, LLM adds narrative summaries

---

## Complete Feature Inventory (1-100)

### SECTION A: Core Monitoring Engine (1-20) — ALL DONE in current codebase

| # | Feature | Status |
|---|---------|--------|
| 1 | Host identity collection (hostname, OS, kernel, uptime) | DONE |
| 2 | CPU load collection (1m, 5m, 15m, cores) | DONE |
| 3 | Memory collection (total, used, available, swap) | DONE |
| 4 | Disk filesystem collection (mounts, usage, type) | DONE |
| 5 | Service collection (systemd units, states) | DONE |
| 6 | Container collection (Docker/Podman, status, health) | DONE |
| 7 | Port collection (listeners, protocols, processes) | DONE |
| 8 | Log collection (journal errors, kernel, auth failures) | DONE |
| 9 | Backup detection (restic, borg, tar, timers) | DONE |
| 10 | Package collection (outdated, security updates) | DONE |
| 11 | Timer collection (systemd timers, cron) | DONE |
| 12 | Network collection (interfaces, routes, DNS) | DONE |
| 13 | Firewall collection (iptables/nftables rules) | DONE |
| 14 | Certificate collection (TLS certs from listeners) | DONE |
| 15 | Database health probes (PostgreSQL, MySQL) | DONE |
| 16 | Web server config collection (nginx, Caddy) | DONE |
| 17 | Monitor profiles (Quick/Standard/Deep) | DONE |
| 18 | Per-collector health tracking | DONE |
| 19 | Partial-failure tolerance per collector | DONE |
| 20 | Configurable timeouts per collector | DONE |

### SECTION B: Detector Engine (21-45) — MOSTLY DONE

| # | Feature | Status |
|---|---------|--------|
| 21 | Disk usage detector (>80%, >90%, >95% thresholds) | DONE |
| 22 | Inode pressure detector | MISSING |
| 23 | SMART disk health detector | PARTIAL |
| 24 | Service failure detector (failed units) | DONE |
| 25 | Service restart loop detector | DONE |
| 26 | Container health detector (unhealthy, exited) | DONE |
| 27 | Container restart count detector | DONE |
| 28 | Open port exposure detector (non-loopback listeners) | DONE |
| 29 | SSH exposure detector (PasswordAuth, RootLogin) | DONE |
| 30 | Log error rate spike detector | DONE |
| 31 | OOM kill detector (kernel log traces) | DONE |
| 32 | Backup staleness detector (>24h, >7d old) | DONE |
| 33 | Backup exclusion detector (critical paths missed) | MISSING |
| 34 | Certificate expiry detector (<7d, <30d, <60d) | DONE |
| 35 | Self-signed certificate detector | DONE |
| 36 | Package update detector (security updates available) | MISSING |
| 37 | Database reachability detector | DONE |
| 38 | Web server TLS detector (no HTTPS on sites) | DONE |
| 39 | Config file drift detector (hash comparison) | DONE |
| 40 | DNS resolver health detector | DONE |
| 41 | Monitoring stack health detector (Grafana, Prometheus) | DONE |
| 42 | CPU pressure stall detector | MISSING |
| 43 | Memory pressure stall detector | MISSING |
| 44 | IO pressure stall detector | MISSING |
| 45 | Firewall mismatch detector (ports vs rules) | MISSING |

### SECTION C: Findings & Evidence System (46-55) — DONE

| # | Feature | Status |
|---|---------|--------|
| 46 | Typed Finding with severity, confidence, category | DONE |
| 47 | Evidence references pointing to snapshot fields | DONE |
| 48 | Finding lifecycle (New, Recurring, Suppressed, Resolved) | DONE |
| 49 | Finding fingerprinting for cross-snapshot tracking | DONE |
| 50 | Correlated findings (linking related issues) | DONE |
| 51 | Confidence scoring per finding | DONE |
| 52 | Impact assessment per finding | DONE |
| 53 | Missing data enumeration per finding | DONE |
| 54 | Read-only verification checks per finding | DONE |
| 55 | Assumptions documentation per finding | DONE |

### SECTION D: Troubleshooting & Plans (56-65) — PARTIAL

| # | Feature | Status |
|---|---------|--------|
| 56 | Hypothesis generation from findings | DONE |
| 57 | Structured troubleshooting plans | DONE |
| 58 | LLM-narrated plan summaries | DONE |
| 59 | Command previews with expected effects | MISSING (web) |
| 60 | Risk assessment per fix step | MISSING (web) |
| 61 | Blast radius calculation | MISSING |
| 62 | Rollback status per command | MISSING |
| 63 | Post-fix verification steps | MISSING |
| 64 | Plan approval gate before execution | DONE |
| 65 | Plan cache for repeated issues | DONE |

### SECTION E: Web Dashboard — Core Screens (66-80) — TO BUILD

| # | Feature | Status |
|---|---------|--------|
| 66 | Real-time system overview (CPU, mem, load, disks) | TO BUILD |
| 67 | Finding/alerts queue with severity grouping | TO BUILD |
| 68 | Finding detail panel (WHAT BROKE → IMPACT → EVIDENCE → FIX) | TO BUILD |
| 69 | Services tab (all units, states, journal preview) | TO BUILD |
| 70 | Containers tab (status, health, ports, logs) | TO BUILD |
| 71 | Processes tab (top CPU/mem, tree view) | TO BUILD |
| 72 | Network tab (listeners, connections, firewall) | TO BUILD |
| 73 | Disk tab (filesystems, I/O, SMART) | TO BUILD |
| 74 | Logs tab (journal, errors, auth, search) | TO BUILD |
| 75 | Fleet tab (multi-host overview, status badges) | TO BUILD |
| 76 | Changes tab (config drift, recent modifications) | TO BUILD |
| 77 | Troubleshooting workspace (10-section incident view) | TO BUILD |
| 78 | Plan generation with hypothesis tree | TO BUILD |
| 79 | Apply plan with approval gates per step | TO BUILD |
| 80 | Runbook export (Markdown, PDF) | TO BUILD |

### SECTION F: Web Dashboard — UX/UI Polish (81-90) — TO BUILD

| # | Feature | Status |
|---|---------|--------|
| 81 | Dark theme (matching HTML mock: #0d1117 bg, #c9d1d9 fg) | TO BUILD |
| 82 | Responsive layout (desktop-first, tablet minimum) | TO BUILD |
| 83 | Keyboard shortcuts throughout (Tab, 1-8, Enter, Escape) | TO BUILD |
| 84 | Live data updates via WebSocket (configurable interval) | TO BUILD |
| 85 | Toast notifications for actions (plan generated, fix applied) | TO BUILD |
| 86 | Loading states with skeleton screens | TO BUILD |
| 87 | Error states with clear recovery actions | TO BUILD |
| 88 | Empty states with helpful guidance | TO BUILD |
| 89 | Breadcrumb navigation for incident drill-down | TO BUILD |
| 90 | Search/filter across all findings, services, containers | TO BUILD |

### SECTION G: Advanced Features — Moats (91-100) — TO BUILD

| # | Feature | Status |
|---|---------|--------|
| 91 | AI chat panel — ask questions about system state, findings | TO BUILD |
| 92 | Natural language troubleshooting ("why is nginx slow?") | TO BUILD |
| 93 | Automated runbook generation from system state | TO BUILD |
| 94 | Fleet-wide dashboard (all hosts at a glance) | TO BUILD |
| 95 | Fleet-wide alert correlation (same issue across hosts) | TO BUILD |
| 96 | Historical trend graphs (CPU/mem/disk over time) | TO BUILD |
| 97 | Configurable alert thresholds per host/domain | TO BUILD |
| 98 | Webhook/email/Slack alert notifications | TO BUILD |
| 99 | Role-based access (Admin/Operator/Viewer) | TO BUILD |
| 100 | One-click daemon install on remote hosts (bootstrap) | TO BUILD |
