# Requirements Document

## Introduction

HELMOPS Rebirth v2 is a complete reimagining of the HelmOps tool as a daily-driver monitoring and troubleshooting console for Linux DevOps/SysAdmin professionals. The system transforms from a monitoring tool with agent-era leftovers into a focused, zero-config, real-time operations console that identifies problems within 5-10 seconds of opening, provides complete issue context, and offers pre-built fix plans before the operator asks. The rebirth strips all unused agent-era infrastructure (knowledge graph, skills, sessions, episodes, standalone tools crate) and consolidates the architecture around the monitoring-first, read-only-first safety model.

## Glossary

- **Dashboard**: The primary TUI interface that displays system health, findings, and fix plans in a single view
- **Finding**: A detected issue with severity, confidence, evidence, impact assessment, and fix plan
- **Fingerprint**: A stable hash-based identity for a finding that persists across refresh cycles
- **Collector**: A module that gathers raw system data from a specific domain (disks, services, containers, etc.)
- **Detector**: A module that analyzes collected data and produces findings when anomalies are detected
- **Fix_Plan**: A pre-generated, capability-gated remediation plan attached to a finding
- **Severity**: The urgency classification of a finding (Critical, Warning, Info)
- **Fleet**: A set of remote Linux hosts monitored in parallel via SSH
- **Changeset**: An audited record of an applied fix plan with pre/post snapshots and rollback capability
- **Ring_Buffer**: A fixed-size circular buffer storing metric history for sparkline rendering
- **Sparkline**: A compact inline trend graph showing metric history over time
- **Progressive_Disclosure**: A UI pattern where simple information is shown first with deeper detail available on demand
- **Execution_Engine**: The component that applies fix plan steps with capability gates, taint checks, file backups, and verification
- **HMAC_Chain**: The hash-based message authentication code chain ensuring audit log integrity
- **Capability_Gate**: A permission check that must pass before any mutating operation executes
- **Taint_Level**: A classification (User/Tool/External) that tracks the trust level of data flowing through the system
- **Web_Console**: The Axum-based web dashboard accessible via browser with WebSocket live updates
- **Daemon**: The background process that continuously collects snapshots and generates findings
- **Thresholds_Config**: The TOML configuration file controlling detector sensitivity and refresh cadence

## Requirements

### Requirement 1: Crate Consolidation and Dead Code Removal

**User Story:** As a maintainer, I want all agent-era dead code removed and the workspace consolidated, so that the binary is smaller, the codebase is focused, and new contributors can understand the architecture immediately.

#### Acceptance Criteria

1. THE Workspace SHALL remove the helm-tools crate from the workspace members list in the root Cargo.toml, delete the crates/helm-tools directory, and remove all helm-tools dependency declarations from other crates' Cargo.toml files
2. THE Workspace SHALL remove the knowledge graph module (helm-memory/src/graph.rs) and all references to EntityGraph in source files, Cargo.toml feature flags, CLI subcommands, and tests
3. THE Workspace SHALL remove the skills system (helm-memory/src/skills.rs, skill_learner.rs) and all references to SkillsManager and SkillLearner in source files, CLI subcommands, and tests
4. THE Workspace SHALL remove the sessions system (helm-memory/src/sessions.rs) and all references to SessionStore in source files, CLI subcommands, and tests
5. THE Workspace SHALL remove the StepRecord type, the log_step and get_steps methods, and the episode_steps table interactions from helm-memory, retaining the EpisodeRecord, CapabilityGrantRecord, AuditEventRecord types, the HMAC chain append/verify methods, and the capability grant methods
6. THE Workspace SHALL remove the user_profile/RBAC module (helm-memory/src/user_profile.rs) and all references to UserProfileStore and Role in source files, CLI subcommands, and tests
7. THE Workspace SHALL remove the merge_episodes module (helm-memory/src/merge_episodes.rs) and all references to MergeResult, MergeConflict, and ConflictResolution in source files, CLI subcommands, and tests
8. WHEN the workspace is built after consolidation using `cargo build --release -p helm-cli`, THE Build SHALL produce a binary whose file size is at least 30% smaller than the binary produced by the same command on the commit immediately preceding the consolidation changes
9. WHEN the workspace is built after consolidation, THE Build SHALL complete with exit code 0 for each of the following commands run in sequence: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace --all-targets`

### Requirement 2: Execution Engine Independence

**User Story:** As a DevOps engineer, I want the execution engine to operate without any agent dependencies, so that fix plans can be applied safely using only standard process execution.

#### Acceptance Criteria

1. THE Execution_Engine SHALL use only std::process::Command for running fix plan steps
2. THE Execution_Engine SHALL check the Capability_Gate before executing each step
3. THE Execution_Engine SHALL verify Taint_Level and reject execution of steps derived from External-tainted content without fresh user approval
4. THE Execution_Engine SHALL create a PreChangeBackup with SHA-256 checksum for every file modified during plan execution
5. THE Execution_Engine SHALL collect a pre-execution SystemSnapshot and a post-execution SystemSnapshot for every Changeset
6. THE Execution_Engine SHALL append each executed step to the HMAC_Chain audit log with full command text, exit code, stdout, and stderr
7. IF a step fails during execution, THEN THE Execution_Engine SHALL halt the remaining steps and offer rollback of completed steps

### Requirement 3: Zero-Config Instant Dashboard

**User Story:** As a sysadmin, I want to type a single command and immediately see what is wrong with my system, so that I can identify problems within 5-10 seconds of opening the tool.

#### Acceptance Criteria

1. WHEN the operator invokes `helmops` with no arguments, THE Dashboard SHALL render the first visible frame of the web console within 500 milliseconds of process start
2. WHEN the operator invokes `helmops tui`, THE Dashboard SHALL render the first visible frame of the terminal interface within 500 milliseconds of process start
3. WHEN the Dashboard launches and findings are available, THE Dashboard SHALL display the highest-severity finding at the top of the findings list without requiring scrolling, ordered by severity descending then by most-recent detection timestamp within the same severity level
4. WHEN the Dashboard launches, THE Dashboard SHALL begin the first Standard MonitorProfile collection cycle within 100 milliseconds of rendering the first frame
5. WHEN the first collection cycle completes, THE Dashboard SHALL display at least one finding (or a healthy-state indicator if no issues are detected) within 5 seconds of process start
6. THE Dashboard SHALL operate without requiring any configuration file, API key, or prior setup for local monitoring of the host machine
7. WHEN no LLM provider is configured, THE Dashboard SHALL display findings and evidence without fix plan generation, and SHALL display a persistent inline hint in the fix plan section indicating that an LLM provider must be configured to enable AI-generated plans
8. IF no findings are detected after the first collection cycle completes, THEN THE Dashboard SHALL display a healthy-state summary showing the number of collectors that ran successfully and the timestamp of the completed scan

### Requirement 4: Real-Time Continuous Monitoring

**User Story:** As a DevOps engineer, I want the system to continuously monitor and refresh data in real-time, so that I see problems as they develop rather than viewing stale snapshots.

#### Acceptance Criteria

1. THE Daemon SHALL collect SystemSnapshots at a configurable interval defaulting to 30 seconds
2. THE Daemon SHALL run all 17 collectors concurrently via tokio::join with per-collector timeout enforcement
3. WHILE the Dashboard is active, THE Dashboard SHALL refresh the findings list on every new snapshot without losing the current selection
4. THE Dashboard SHALL maintain fingerprint-based selection stability across refresh cycles
5. THE Dashboard SHALL update Ring_Buffer metric history on each refresh cycle for cpu, memory, disk, network, and load metrics
6. THE Dashboard SHALL render Sparkline trend graphs beneath each severity group in the findings pane
7. IF a collector fails or times out, THEN THE Dashboard SHALL display the collector error in a status indicator without blocking other collectors
8. THE Thresholds_Config SHALL allow operators to configure refresh interval, collector timeouts, and sparkline history depth

### Requirement 5: Finding Context and Progressive Disclosure

**User Story:** As a sysadmin, I want every finding to show complete context (when, where, what, why, impact, fix) in a structured layout, so that I can understand and resolve issues without switching to other tools.

#### Acceptance Criteria

1. WHEN a finding is selected, THE Dashboard SHALL display a detail pane with sections in this order: WHEN (timestamp and duration), WHERE (affected resource), WHAT (title and evidence), WHY (root cause analysis), IMPACT (blast radius and affected services), FIX (pre-generated plan steps)
2. THE Finding SHALL include at least one EvidenceRef linking to the specific snapshot field that triggered detection
3. THE Finding SHALL include an impact assessment describing the blast radius and downstream effects
4. THE Finding SHALL include read-only verification checks that the operator can run to confirm the issue
5. WHEN correlated findings exist for the same root cause, THE Dashboard SHALL collapse them into a single entry with a count indicator showing "× N"
6. WHEN the operator expands a collapsed correlation group, THE Dashboard SHALL show each individual finding with its specific evidence

### Requirement 6: Pre-Built Fix Plans

**User Story:** As a DevOps engineer, I want fix plans to be ready before I ask for them, so that I can resolve issues immediately without waiting for plan generation.

#### Acceptance Criteria

1. WHEN a new finding is detected, THE Daemon SHALL spawn a background task to generate a Fix_Plan via the configured LLM provider
2. THE Daemon SHALL cache generated Fix_Plans by finding fingerprint in an in-memory cache with configurable TTL defaulting to 24 hours
3. WHEN the operator selects a finding with a cached Fix_Plan, THE Dashboard SHALL render the plan instantly without a loading state
4. WHEN the operator selects a finding without a cached Fix_Plan, THE Dashboard SHALL display a "generating plan..." indicator
5. THE Fix_Plan SHALL include numbered steps with exact commands, expected effects, risk levels, blast radius, and rollback instructions
6. THE Fix_Plan SHALL be capability-gated so that each step declares the required Capability before execution
7. WHEN no LLM provider is configured, THE Dashboard SHALL display static remediation hints based on the detector type instead of LLM-generated plans

### Requirement 7: TUI Dashboard Layout

**User Story:** As a sysadmin, I want a clean, information-dense terminal dashboard that shows critical information prominently without visual clutter, so that I can scan system health at a glance.

#### Acceptance Criteria

1. THE Dashboard SHALL display a header bar containing: hostname, kernel version, uptime, fleet status (N/N up), current time, and LLM provider status
2. THE Dashboard SHALL display a left pane occupying approximately one-third of the terminal width, with findings grouped by severity (Critical block, Warning block, Info block) with count badges showing the number of findings in each group
3. THE Dashboard SHALL display Sparklines for cpu, memory, disk, network, and load beneath the severity groups in the left pane, each rendering the most recent 60 data points of history
4. WHEN a finding is selected in the left pane, THE Dashboard SHALL display a right detail pane showing the finding context with labeled sections: WHEN, WHERE, WHAT, WHY, IMPACT, and FIX
5. IF no finding is selected, THEN THE Dashboard SHALL display a placeholder message in the right detail pane indicating that no finding is selected
6. THE Dashboard SHALL display a bottom status bar with hotkey hints, a refresh countdown indicator showing seconds until next refresh, and a last-refresh timestamp in UTC
7. THE Dashboard SHALL provide 6 tabs: ALERTS (default), SERVICES, RESOURCES, LOGS, FLEET, CHANGES, navigable via left/right arrow keys or number keys 1-6
8. THE Dashboard SHALL render the RESOURCES tab with sub-sections for CPU/Memory, Disk, Network, and Processes, each accessible via a keyboard toggle key displayed in the sub-section header
9. WHILE the terminal width is less than 120 columns, THE Dashboard SHALL collapse the two-pane layout into a single-pane list with detail shown on Enter key press
10. IF the terminal width is less than 60 columns or height is less than 10 rows, THEN THE Dashboard SHALL display a message indicating that a larger terminal is required instead of rendering the dashboard layout

### Requirement 8: Web Console

**User Story:** As a DevOps engineer, I want a web-based dashboard accessible from any browser, so that I can monitor systems from machines where a terminal is not convenient.

#### Acceptance Criteria

1. WHEN the operator invokes `helmops` or `helmops web`, THE Web_Console SHALL start an HTTP server on port 9120 (configurable via --port flag)
2. THE Web_Console SHALL serve a single-file HTML frontend with no JavaScript framework dependency and no build step
3. THE Web_Console SHALL establish a WebSocket connection to the Daemon for real-time finding updates
4. THE Web_Console SHALL display the same finding data, severity grouping, and detail context as the TUI Dashboard
5. THE Web_Console SHALL support keyboard shortcuts: 1-9 for tab switching, Enter for selection, Escape for clearing, Alt+G for plan guidance, F5 for refresh
6. THE Web_Console SHALL display operator cards for: daemon status, update pressure, storage risk, runtime counts, and collector health

### Requirement 9: Fleet Multi-Host Monitoring

**User Story:** As a DevOps engineer managing multiple servers, I want a single pane of glass showing all hosts with parallel data collection, so that I can identify which host has problems without SSH-ing into each one.

#### Acceptance Criteria

1. THE Fleet SHALL collect findings from all registered remote hosts concurrently via parallel SSH using tokio JoinSet
2. THE Fleet SHALL complete a full refresh of 20 hosts within 2 seconds
3. THE Fleet SHALL display a summary in the header bar showing "fleet: N/M up" where N is reachable hosts and M is total registered hosts
4. THE Fleet tab SHALL display per-host rows with hostname, critical finding count, warning count, and last-seen timestamp
5. THE Fleet SHALL support credential abstraction with SshAgent, KeyFile, and Password variants, storing no passwords in the registry file
6. WHEN a remote host becomes unreachable, THE Fleet SHALL mark the host as down and retain the last-known findings with a staleness indicator
7. THE Fleet SHALL extend each remote finding with the source host_id (UUID) for traceability

### Requirement 10: Comprehensive Detector Coverage

**User Story:** As a sysadmin, I want the system to detect all common infrastructure problems automatically, so that I do not need to manually check each subsystem.

#### Acceptance Criteria

1. THE Detector_Registry SHALL include detectors for: disk usage, inode exhaustion, filesystem read-only, high load, memory pressure, swap exhaustion, OOM kills, failed services, inactive critical services, container restarts, unhealthy containers, restart loops, journal error spikes, certificate expiry, backup staleness, package security updates, DNS health, exposed ports, SSH exposure, web server errors, database health, Kubernetes pod restarts, PVC pressure, libvirt domain state, Compose health, configuration drift, and SMART disk health
2. WHEN a detector produces a finding, THE Finding SHALL include the detector_id, affected_resource, severity, confidence, and at least one EvidenceRef
3. THE Detector_Registry SHALL allow operators to disable specific detectors via Thresholds_Config
4. THE Detector_Registry SHALL allow operators to adjust severity thresholds per detector via Thresholds_Config
5. WHEN a finding that was previously open is no longer detected, THE Detector_Registry SHALL transition the finding lifecycle to SelfResolved

### Requirement 11: Alerting and Notification

**User Story:** As a DevOps engineer, I want critical findings to page me through my existing alerting infrastructure, so that I am notified of problems even when not watching the dashboard.

#### Acceptance Criteria

1. THE Alert_Router SHALL route findings to configured sinks based on severity threshold (Critical, Warning, Info gated independently)
2. THE Alert_Router SHALL deduplicate alerts by finding fingerprint within a configurable window defaulting to 300 seconds
3. THE Alert_Router SHALL enforce rate limiting per sink to prevent alert storms
4. WHEN a finding is routed, THE Webhook_Sink SHALL send an HTTP POST with JSON body containing finding details, with exponential backoff retry on failure
5. WHEN a finding is routed, THE Slack_Sink SHALL send a formatted message to the configured incoming webhook URL with severity color coding (red for Critical, yellow for Warning, blue for Info)
6. WHEN a finding is routed, THE PagerDuty_Sink SHALL send an Events API v2 event with dedup_key set to the finding fingerprint
7. WHEN a finding lifecycle transitions to Resolved or SelfResolved, THE PagerDuty_Sink SHALL send a resolution event

### Requirement 12: Audit Trail and Security Spine

**User Story:** As a sysadmin, I want every action logged in a tamper-proof audit trail, so that I can prove what happened during incident response and satisfy compliance requirements.

#### Acceptance Criteria

1. THE HMAC_Chain SHALL append an audit record for every fix plan execution, including the operator identity, timestamp, commands executed, and outcomes
2. THE HMAC_Chain SHALL include the hash of the previous record in each new record to form a verifiable chain
3. WHEN `helmops audit verify` is invoked, THE HMAC_Chain SHALL verify the entire chain and report any tampering or gaps
4. THE Capability_Gate SHALL block all write operations on External-tainted content without fresh operator approval
5. THE Secrets_Store SHALL store API keys in $XDG_CONFIG_HOME/helm/secrets.toml with file mode 0600 and parent directory mode 0700
6. THE Secrets_Store SHALL resolve API keys in order: CLI flag, secrets store, environment variable
7. THE Dashboard SHALL never silently import environment variable keys into the persistent secrets store

### Requirement 13: Guided Troubleshooting Flow

**User Story:** As a junior sysadmin, I want step-by-step troubleshooting guidance with explanations, so that I can resolve issues safely even without deep expertise.

#### Acceptance Criteria

1. WHEN the operator requests troubleshooting for a finding, THE Troubleshooter SHALL generate a plan with hypothesis, read-only verification steps, and proposed fix steps
2. THE Troubleshooter SHALL display each fix step with: exact command, expected effect, risk level, blast radius, and rollback instructions
3. THE Troubleshooter SHALL require explicit operator approval before executing each mutating step
4. WHEN a fix step is approved, THE Execution_Engine SHALL execute it, capture output, and verify the expected outcome
5. IF a verification step produces unexpected output, THEN THE Troubleshooter SHALL pause and present the unexpected result to the operator with guidance on how to interpret it
6. THE Troubleshooter SHALL provide a "beginner mode" that includes explanations of what each command does and why it is being run

### Requirement 14: Performance Budgets

**User Story:** As a sysadmin, I want the dashboard to remain responsive and fast regardless of the number of findings or hosts, so that the tool never becomes a bottleneck during incident response.

#### Acceptance Criteria

1. THE Dashboard SHALL complete a full render cycle within 16 milliseconds at 60 findings with 5 active sparklines
2. THE Sparkline renderer SHALL complete rendering within 4 milliseconds for 60 data points across 8 metrics
3. THE Fleet SHALL complete parallel SSH collection from 20 hosts within 2 seconds
4. THE Dashboard SHALL launch and display the first frame within 500 milliseconds of invocation
5. THE Ring_Buffer SHALL cap at a configurable depth defaulting to 60 points per metric, using constant memory regardless of uptime duration
6. THE Daemon SHALL enforce per-collector timeouts (5s Quick, 10s Standard, 30s Deep) to prevent any single collector from blocking the refresh cycle

### Requirement 15: Configuration and Customization

**User Story:** As an experienced sysadmin, I want to tune detection thresholds, disable irrelevant detectors, and configure refresh behavior, so that the tool adapts to my specific infrastructure without false positives.

#### Acceptance Criteria

1. THE Thresholds_Config SHALL be loaded from $XDG_CONFIG_HOME/helm/thresholds.toml
2. THE Thresholds_Config SHALL support per-detector threshold overrides (e.g., disk_usage_warning_percent, load_critical_multiplier)
3. THE Thresholds_Config SHALL support disabling specific detectors by name
4. THE Thresholds_Config SHALL support configuring the refresh interval in seconds
5. THE Thresholds_Config SHALL support configuring sparkline history depth
6. WHEN thresholds.toml does not exist, THE Dashboard SHALL operate with sensible defaults requiring no configuration
7. WHEN thresholds.toml contains invalid values, THE Dashboard SHALL log a warning and fall back to defaults for the invalid fields

### Requirement 16: Changes Tab and Incident Timeline

**User Story:** As a DevOps engineer, I want to see a timeline of all changes applied through the tool and correlate them with findings, so that I can understand cause-and-effect during incident review.

#### Acceptance Criteria

1. THE Changes tab SHALL display a chronological list of all Changesets with: timestamp, plan title, status (completed/failed/rolled-back), and affected resources
2. WHEN a Changeset is selected, THE Changes tab SHALL display the full step-by-step execution log with stdout/stderr for each step
3. THE Changes tab SHALL display the pre-execution and post-execution snapshot diffs for each Changeset
4. THE Changes tab SHALL allow the operator to initiate rollback of a completed Changeset using the stored PreChangeBackups
5. THE Changes tab SHALL correlate Changesets with findings by showing which finding triggered each change

### Requirement 17: Logs Tab with Anomaly Detection

**User Story:** As a sysadmin, I want to see log anomalies surfaced automatically with pattern detection, so that I do not need to manually grep through journal output to find problems.

#### Acceptance Criteria

1. THE Logs tab SHALL display recent journal errors grouped by unit with occurrence count and last-seen timestamp
2. THE Logs tab SHALL highlight new error patterns that were not present in the previous snapshot cycle
3. WHEN an error pattern exceeds a configurable spike threshold, THE Logs tab SHALL promote the pattern to a finding with Warning or Critical severity
4. THE Logs tab SHALL support filtering by unit name, severity level, and time range
5. THE Logs tab SHALL display the raw log lines for a selected error pattern with context lines above and below

### Requirement 18: Services Tab

**User Story:** As a sysadmin, I want a dedicated view of all systemd services and timers with their health status, so that I can quickly identify failed or degraded services.

#### Acceptance Criteria

1. THE Services tab SHALL display all loaded systemd units with their active state, sub-state, and description
2. THE Services tab SHALL sort failed units to the top with Critical severity highlighting
3. THE Services tab SHALL display systemd timers with last-trigger time, next-trigger time, and whether the timer is overdue
4. WHEN a service is selected, THE Services tab SHALL display recent journal output for that unit
5. THE Services tab SHALL provide quick-action hotkeys for: restart service, stop service, view full journal (all capability-gated)

### Requirement 19: Workspace Build and Binary Output

**User Story:** As a maintainer, I want the workspace to produce a single optimized binary with clear entry points, so that installation and deployment remain simple.

#### Acceptance Criteria

1. THE Workspace SHALL produce a single binary named `helmops` from the helm-cli crate
2. THE Workspace SHALL maintain `helm` as a compatibility alias or symlink
3. THE Workspace SHALL build with workspace members: helm-core, helm-providers, helm-memory, helm-monitor, helm-cli (5 crates total after tools removal)
4. THE Workspace SHALL pass the release gate before any tag: cargo fmt --check, cargo clippy --workspace --all-targets -- -D warnings, cargo test --workspace --all-targets, cargo build --release -p helm-cli
5. THE Binary SHALL support shell completion generation for bash, zsh, and fish via `helmops completion`

### Requirement 20: Correlation and Root Cause Analysis

**User Story:** As a DevOps engineer, I want the system to correlate related findings and trace symptoms to root causes, so that I fix the actual problem rather than chasing symptoms.

#### Acceptance Criteria

1. WHEN multiple findings share a common root cause (e.g., disk full causing service failures), THE Detector_Registry SHALL group them under a single correlation identifier
2. THE Dashboard SHALL display correlated findings as a collapsed group with the root cause finding as the primary entry
3. WHEN the operator expands a correlation group, THE Dashboard SHALL show the causal chain: root cause → intermediate effects → symptoms
4. THE Correlation engine SHALL detect temporal patterns where finding A consistently precedes finding B within a configurable window
5. THE Correlation engine SHALL detect resource dependency patterns where a resource failure cascades to dependent services

### Requirement 21: Doctor and Self-Diagnostics

**User Story:** As a sysadmin, I want the tool to verify its own health and dependencies on first run, so that I know immediately if something is misconfigured.

#### Acceptance Criteria

1. WHEN `helmops doctor` is invoked, THE Doctor SHALL verify: database accessibility, secrets store permissions, provider reachability (if configured), required system binaries (systemctl, journalctl, df, ss), and collector prerequisites
2. THE Doctor SHALL report each check as PASS, WARN, or FAIL with a human-readable explanation
3. THE Doctor SHALL support --json output for automation integration
4. WHEN a required binary is missing, THE Doctor SHALL suggest the package to install for the detected distribution (apt/dnf/pacman)
5. THE Doctor SHALL verify that the HMAC audit chain is intact

### Requirement 22: Custom Service Registration

**User Story:** As a DevOps engineer, I want to register my own services (Docker containers, databases, Kubernetes clusters, CI/CD pipelines, web servers, custom applications) for monitoring, so that the tool covers my entire infrastructure stack without requiring code changes.

#### Acceptance Criteria

1. THE Dashboard SHALL provide a service registration interface accessible from the TUI (via slash command `/service add`) and CLI (`helmops service add`) where operators can add monitored services
2. THE Service_Registry SHALL support registering services of types: Docker containers (by name or label, health check via Docker API container status), databases (PostgreSQL, MySQL, Redis, MongoDB by connection string of at most 2048 characters, health check via TCP connect), Kubernetes clusters (by kubeconfig path, health check via kubectl cluster-info), CI/CD pipelines (by webhook URL or API endpoint of at most 2048 characters, health check via HTTP GET expecting 2xx response), web servers (by URL and expected HTTP status code, health check via HTTP GET matching the specified status code), and custom TCP/HTTP endpoints (health check via TCP connect or HTTP GET expecting 2xx), up to a maximum of 256 registered services
3. WHEN a service is registered, THE Daemon SHALL begin monitoring the service within the next refresh cycle (default 30 seconds) without requiring a restart
4. IF a service registration request contains an invalid connection string, unreachable URL, or malformed configuration, THEN THE Service_Registry SHALL reject the registration and display an error message indicating the validation failure while preserving any previously entered fields
5. THE Service_Registry SHALL persist registered services in $XDG_CONFIG_HOME/helm/services.toml using atomic write (temp-file + rename) with file mode 0600
6. THE Service_Registry SHALL support grouping services by environment (production, staging, development, or a custom label up to 64 characters) and team ownership (a string label up to 64 characters identifying the owning team)
7. WHEN a registered service fails 3 consecutive health checks, THE Dashboard SHALL display a WARN-severity finding with the service name, health status, failure reason, and last-healthy timestamp
8. IF a registered service fails 5 consecutive health checks, THEN THE Dashboard SHALL escalate the finding to CRIT severity
9. THE Service_Registry SHALL support removal and modification of registered services without restarting the daemon, with changes taking effect on the next refresh cycle

### Requirement 23: Custom Detection Conditions

**User Story:** As an experienced engineer, I want to define my own detection conditions and alert rules, so that I can monitor business-specific metrics and thresholds that the built-in detectors do not cover.

#### Acceptance Criteria

1. THE Custom_Detector system SHALL allow operators to define detection rules in $XDG_CONFIG_HOME/helm/rules.toml using a declarative syntax
2. THE Custom_Detector rule format SHALL support: metric source (collector field path), comparison operator (gt, lt, eq, ne, contains, matches), threshold value, severity level, and human-readable title
3. THE Custom_Detector system SHALL support compound conditions combining multiple metrics with AND/OR logic
4. WHEN a custom rule triggers, THE Dashboard SHALL display the finding with the same structure as built-in findings (severity, evidence, impact, fix suggestion)
5. THE Custom_Detector system SHALL support rate-based conditions (e.g., "more than N occurrences in M seconds")
6. THE Custom_Detector system SHALL validate rule definitions on load and report syntax errors with line numbers and suggestions
7. WHEN rules.toml is modified, THE Daemon SHALL reload rules on the next refresh cycle without requiring a restart

### Requirement 24: Minimalistic UI with 5-Second Problem Identification

**User Story:** As any engineer from beginner to expert, I want the UI to be minimalistic and immediately understandable, so that I can identify what is critically wrong within 5 seconds of opening the tool regardless of my experience level.

#### Acceptance Criteria

1. THE Dashboard SHALL use a maximum of 3 colors for severity indication: red for Critical, amber/yellow for Warning, and blue/gray for Info
2. THE Dashboard SHALL display Critical findings with bold text, a red severity indicator, and positioned above all Warning and Info findings so that Critical items appear first in the list without scrolling
3. THE Dashboard SHALL use finding titles of no more than 80 characters that state the affected resource and its current state without abbreviations or internal metric names (e.g., "Disk /var is 94% full" not "disk_usage threshold exceeded on mount /var")
4. THE Dashboard SHALL display a single-line summary of no more than 120 characters at the top of the view, visible without scrolling, stating the count of critical issues and a brief description of each (e.g., "2 critical issues: disk nearly full, nginx returning 5xx errors")
5. THE Dashboard SHALL display by default only the summary line, the list of findings (title and severity indicator), and a count of total findings, with all other elements (configuration options, historical data, metadata, and navigation) hidden until the user explicitly selects a finding or activates a menu
6. THE Web_Console SHALL render a card-based layout with a minimum font size of 14px for body text, a minimum of 16px spacing between cards, and no more than 2 visual element types per card (severity indicator and finding title)
7. THE Web_Console SHALL render all findings and the summary line without horizontal scrolling on viewports as narrow as 320px, with touch targets of at least 44x44px for interactive elements
8. THE Dashboard SHALL provide contextual tooltips for beginners explaining what each section means, togglable via a help mode

### Requirement 25: Infrastructure Pipeline Monitoring

**User Story:** As a deployment engineer, I want to monitor CI/CD pipelines, deployment status, and infrastructure-as-code drift, so that I can catch deployment failures and configuration drift from the same console.

#### Acceptance Criteria

1. THE Pipeline_Collector SHALL monitor registered CI/CD pipeline endpoints and surface: last build status, build duration trend, failure rate, and currently running builds
2. WHEN a pipeline build fails, THE Dashboard SHALL display a Critical finding with the pipeline name, failure stage, and link to logs
3. THE Pipeline_Collector SHALL support webhook-based push notifications from CI/CD systems (GitHub Actions, GitLab CI, Jenkins) via a configurable HTTP endpoint
4. THE Config_Drift_Detector SHALL compare current system state against a declared baseline and surface deviations as findings
5. WHEN infrastructure drift is detected, THE Finding SHALL include the specific configuration key, expected value, and actual value
6. THE Pipeline_Collector SHALL track deployment frequency and change failure rate as metrics visible in the RESOURCES tab

