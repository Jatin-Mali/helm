# HELMOPS Web Dashboard — UX/UI Design Specification
> Based on HTML mock at docs/helmops_tui_dashboard.html

## Design System

### Colors (GitHub-dark inspired)
```
--bg-primary:    #0d1117   (main background)
--bg-secondary:  #161b22   (header, tab bar, footer)
--bg-tertiary:   #21262d   (borders, separators)
--border:        #30363d   (panel borders)
--text-primary:  #c9d1d9   (main text)
--text-secondary:#8b949e   (labels, descriptions)
--text-muted:    #6e7681   (dim text, separators)
--accent-blue:   #58a6ff   (HELMOPS brand, links, info)
--accent-green:  #3fb950   (success, healthy, LIVE dot)
--accent-yellow: #d29922   (warnings, medium severity)
--accent-red:    #f85149   (critical, errors, high severity)
--accent-orange: #f0883e   (elevated, attention)
```

### Typography
```
font-family: 'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace
font-size: 11.5px (body), 10.5px (footer/labels)
line-height: 1.4
letter-spacing: 0.06em (section headers)
```

### Spacing
```
gap-xs:   4px
gap-sm:   8px
gap-md:   14px
gap-lg:   24px
border-radius: 3px (buttons/badges), 6px (panels)
```

## Screen Layout

### Screen 1: Dashboard (default view)

```
┌──────────────────────────────────────────────────────────────┐
│ HELMOPS | HOSTNAME | provider/model | ● LIVE | TIME | METRICS│  ← Header (40px)
├──────────────────────────────────────────────────────────────┤
│ CRIT 2  WARN 14  INFO 38  MUTED 5    svc 2 fail  ctr 4/5    │  ← Status Strip (28px)
├──────────────────────────────────────────────────────────────┤
│ ALERTS | SERVICES | PROCESSES | LOGS | NETWORK | DISK | ...  │  ← Tab Bar (32px)
├────────────┬─────────────────────────────────────────────────┤
│ FINDING    │  ┌─────────────────────────────────────────┐   │
│ QUEUE      │  │  WHAT BROKE                              │   │
│            │  │  Nginx service failed                    │   │
│ ● CRITICAL │  │  Affected: nginx.service                 │   │
│   Nginx    │  │  Since: 1d ago | Occurrences: 3          │   │
│   down     │  ├─────────────────────────────────────────┤   │
│            │  │  IMPACT                                   │   │
│ ● WARNING  │  │  Service outage on production            │   │
│   Disk     │  ├─────────────────────────────────────────┤   │
│   /var 88% │  │  EVIDENCE                                 │   │
│            │  │  systemctl is-active nginx: failed        │   │
│ ● INFO     │  │  journalctl -u nginx: connection refused  │   │
│   Cert     │  ├─────────────────────────────────────────┤   │
│   exp 15d  │  │  WHAT CHANGED                             │   │
│            │  │  nginx.conf modified 2h before failure    │   │
│ (scroll)   │  ├─────────────────────────────────────────┤   │
│            │  │  HOW TO REPRODUCE                         │   │
│            │  │  systemctl status nginx                   │   │
│            │  │  nginx -t                                  │   │
│            │  ├─────────────────────────────────────────┤   │
│            │  │  RECOMMENDED FIX                          │   │
│            │  │  nginx -t && systemctl restart nginx      │   │
│            │  │  Risk: MEDIUM | Rollback: systemctl stop   │   │
│            │  ├─────────────────────────────────────────┤   │
│            │  │  ROLLBACK                                 │   │
│            │  │  systemctl stop nginx                     │   │
│            │  ├─────────────────────────────────────────┤   │
│            │  │  EXECUTION STATUS                         │   │
│            │  │  ⏳ Pending approval                      │   │
│            │  └─────────────────────────────────────────┘   │
├────────────┴─────────────────────────────────────────────────┤
│ Tab focus  F5 refresh  Enter select  Alt+G plan  S mute  ... │  ← Footer (24px)
└──────────────────────────────────────────────────────────────┘
```

### Screen 2: Services Tab

```
┌────────────┬─────────────────────────────────────────────────┐
│ FINDING    │  SERVICES (32 total, 2 failed)                  │
│ QUEUE      │  ┌─────────────────────────────────────────┐   │
│ (same)     │  │ NAME          STATE      DESCRIPTION     │   │
│            │  │ nginx.service FAILED     Nginx web server│   │
│            │  │ docker.service RUNNING   Docker daemon   │   │
│            │  │ sshd.service  RUNNING    OpenSSH server  │   │
│            │  │ postgresql    RUNNING    PostgreSQL DB   │   │
│            │  │ ...32 rows...                             │   │
│            │  └─────────────────────────────────────────┘   │
│            │  ┌─ nginx.service Detail ──────────────────┐   │
│            │  │ Journal (last 20 lines):                 │   │
│            │  │ May 21 14:30:15 nginx[1234]: emerg       │   │
│            │  │ bind() to 0.0.0.0:80 failed (98:        │   │
│            │  │ Address already in use)                  │   │
│            │  │ May 21 14:30:15 nginx[1234]: emerg       │   │
│            │  │ bind() to [::]:80 failed (98: Address    │   │
│            │  │ already in use)                           │   │
│            │  └─────────────────────────────────────────┘   │
└────────────┴─────────────────────────────────────────────────┘
```

### Screen 3: Containers Tab
Similar layout — container list + detail with logs, ports, mounts.

### Screen 4: Fleet Tab
```
┌────────────┬─────────────────────────────────────────────────┐
│ FINDING    │  FLEET (4 hosts)                                │
│ QUEUE      │  ┌─────────────────────────────────────────┐   │
│ (same)     │  │ HOST          STATUS    FINDINGS  LAST    │   │
│            │  │ phantom       ● online   2 crit   14:30  │   │
│            │  │ web-01        ● online   0        14:28  │   │
│            │  │ db-01         ◐ warning  1 warn   14:25  │   │
│            │  │ cache-01      ○ offline  —        12:00  │   │
│            │  └─────────────────────────────────────────┘   │
│            │  Click host → loads that host's dashboard       │
└────────────┴─────────────────────────────────────────────────┘
```

## Interaction States

### Loading State
- Skeleton screens: gray pulsing rectangles where content will appear
- Spinner in header: "Collecting system snapshot..."
- First load: shows within 500ms (cached snapshot from DB)

### Empty State
- "No findings detected — your system looks healthy"
- Green checkmark icon
- "Last checked: 14:30:00 UTC"

### Error State
- Red banner: "Collection failed: connection refused"
- Retry button with countdown
- "Try: helm daemon start" suggestion

### Success State
- Green toast: "Plan generated successfully"
- Auto-dismiss after 3 seconds
- Plan appears in detail panel

### Keyboard Navigation
```
1-8      → Switch tabs
↑/↓      → Navigate finding queue
Enter    → Select finding / open detail
Escape   → Close detail / go back to queue
F5       → Force refresh
Alt+G    → Generate troubleshooting plan
Alt+A    → Open apply flow
S        → Suppress finding
R        → Resolve finding
/        → Focus search
Ctrl+K   → Command palette
?        → Show keyboard shortcuts
```

## Responsive Breakpoints
```
Desktop:  ≥ 1024px  (full layout with sidebar)
Tablet:   768-1023px (collapsible sidebar)
Mobile:   < 768px   (stacked layout, hidden queue)
```
