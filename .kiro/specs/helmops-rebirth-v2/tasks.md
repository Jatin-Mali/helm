# Implementation Plan: HELMOPS Rebirth v2

## Overview

This plan transforms the 6-crate HELM workspace into a focused 5-crate monitoring console by removing dead agent-era code, adding new subsystems (custom rules, service registry, correlation engine, pipeline monitoring), refining the TUI/web dashboards, and ensuring correctness via property-based tests for all 36 design properties.

## Tasks

- [x] 1. Crate Consolidation and Dead Code Removal
  - [x] 1.1 Remove helm-tools crate from workspace
    - Delete `crates/helm-tools/` directory entirely
    - Remove `helm-tools` from workspace members in root `Cargo.toml`
    - Remove all `helm-tools` dependency declarations from other crates' `Cargo.toml` files
    - Remove `helm-tools` from root `[dev-dependencies]`
    - Verify `cargo build --workspace` succeeds
    - _Requirements: 1.1_

  - [x] 1.2 Remove knowledge graph module from helm-memory
    - Delete `crates/helm-memory/src/graph.rs`
    - Remove all `EntityGraph` references from source files, Cargo.toml features, CLI subcommands, and tests
    - Remove `helm memory graph`, `helm memory export`, `helm memory import`, `helm memory gc` CLI commands
    - _Requirements: 1.2_

  - [x] 1.3 Remove skills system from helm-memory
    - Delete `crates/helm-memory/src/skills.rs` and `crates/helm-memory/src/skill_learner.rs`
    - Remove all `SkillsManager` and `SkillLearner` references from source files, CLI subcommands, and tests
    - Remove `helm skills` CLI commands
    - _Requirements: 1.3_

  - [x] 1.4 Remove sessions system from helm-memory
    - Delete `crates/helm-memory/src/sessions.rs`
    - Remove all `SessionStore` references from source files, CLI subcommands, and tests
    - Remove `helm sessions` CLI commands
    - _Requirements: 1.4_

  - [x] 1.5 Remove episode steps and RBAC from helm-memory
    - Remove `StepRecord` type, `log_step`, `get_steps` methods, and `episode_steps` table interactions
    - Retain `EpisodeRecord`, `CapabilityGrantRecord`, `AuditEventRecord`, HMAC chain append/verify, and capability grant methods
    - Delete `crates/helm-memory/src/user_profile.rs` — remove `UserProfileStore` and `Role` references
    - Delete `crates/helm-memory/src/merge_episodes.rs` — remove `MergeResult`, `MergeConflict`, `ConflictResolution` references
    - Remove `helm episodes merge`, `helm profile`, `helm stats` CLI commands
    - _Requirements: 1.5, 1.6, 1.7_

  - [x] 1.6 Verify clean build and binary size reduction
    - Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets`
    - Build release binary with `cargo build --release -p helm-cli`
    - Verify binary size is at least 30% smaller than pre-consolidation baseline
    - _Requirements: 1.8, 1.9, 19.4_

- [x] 2. Checkpoint - Verify consolidation
  - Ensure all tests pass, ask the user if questions arise.

- [x] 3. Core Infrastructure — Ring Buffer and Metrics
  - [x] 3.1 Implement RingBuffer generic data structure
    - Create `crates/helm-monitor/src/ring_buffer.rs`
    - Implement `RingBuffer<T>` with `new(capacity)`, `push`, `iter`, `len`, `is_empty`
    - Ensure O(1) push, fixed memory, circular overwrite semantics
    - _Requirements: 4.5, 14.5_

  - [x] 3.2 Write property test for RingBuffer capacity invariant
    - **Property 7: Ring Buffer Capacity Invariant**
    - Use proptest with random push sequences to verify buffer never exceeds capacity and retains last N values in order
    - **Validates: Requirements 4.5, 14.5**

  - [x] 3.3 Implement MetricHistory using RingBuffer
    - Create `MetricHistory` struct with `RingBuffer<f64>` fields for cpu, memory, disk, network, load, swap, io_pressure
    - Integrate into daemon loop to update on each snapshot cycle
    - Wire sparkline rendering to read from `MetricHistory`
    - _Requirements: 4.5, 4.6, 7.3_

- [x] 4. Core Infrastructure — Thresholds Config and Daemon Loop
  - [x] 4.1 Implement thresholds.toml loading and validation
    - Create config loading from `$XDG_CONFIG_HOME/helm/thresholds.toml`
    - Support per-detector threshold overrides, disabled detectors list, refresh_interval_secs, sparkline_history_depth
    - Implement fallback to defaults for invalid values with logged warnings
    - Handle missing config file gracefully (use defaults)
    - _Requirements: 15.1, 15.2, 15.3, 15.4, 15.5, 15.6, 15.7_

  - [x] 4.2 Write property tests for config validation
    - **Property 17: Detector Threshold Override**
    - **Property 18: Invalid Config Fallback**
    - Use proptest to verify overridden thresholds are used by detectors and invalid values fall back to defaults
    - **Validates: Requirements 10.4, 15.2, 15.7**

  - [x] 4.3 Implement daemon loop with configurable refresh
    - Formalize the daemon loop in `helm-cli/src/main.rs` with configurable interval from thresholds.toml
    - Implement collect → detect → correlate → alert → broadcast pipeline
    - Add per-collector timeout enforcement (5s Quick, 10s Standard, 30s Deep)
    - _Requirements: 4.1, 4.2, 14.6, 19.1_

  - [x] 4.4 Write property test for collector failure isolation
    - **Property 8: Collector Failure Isolation**
    - Use proptest with random collector failure combinations to verify non-failing collectors still produce valid data
    - **Validates: Requirements 4.7**

- [x] 5. Execution Engine Formalization
  - [x] 5.1 Verify and formalize execution engine independence
    - Audit `crates/helm-monitor/src/execute.rs` to confirm only `std::process::Command` is used
    - Remove any remaining agent-era dependencies from execution path
    - Ensure capability gate check before each step execution
    - Ensure taint level verification rejects External-tainted content without fresh approval
    - _Requirements: 2.1, 2.2, 2.3_

  - [x] 5.2 Write property test for capability gate enforcement
    - **Property 1: Capability Gate Enforcement**
    - Use proptest with random plans × random taint levels to verify External-tainted steps are rejected
    - **Validates: Requirements 2.2, 2.3, 12.4**

  - [x] 5.3 Implement PreChangeBackup with SHA-256 verification
    - Ensure every file modified during plan execution gets a backup with SHA-256 checksum
    - Verify backup integrity matches actual file content at time of backup
    - _Requirements: 2.4_

  - [x] 5.4 Write property test for PreChangeBackup integrity
    - **Property 2: PreChangeBackup Integrity**
    - Use proptest with random file contents to verify stored SHA-256 matches actual content
    - **Validates: Requirements 2.4**

  - [x] 5.5 Implement halt-on-failure behavior
    - Ensure that when step N fails, all subsequent steps remain Pending and are not executed
    - Offer rollback of completed steps on failure
    - _Requirements: 2.7_

  - [x] 5.6 Write property test for execution halt on step failure
    - **Property 4: Execution Halts on Step Failure**
    - Use proptest with random plans and injected failures to verify subsequent steps stay Pending
    - **Validates: Requirements 2.7**

  - [x] 5.7 Implement pre/post execution snapshots and HMAC audit logging
    - Collect pre-execution and post-execution SystemSnapshot for every Changeset
    - Append each executed step to HMAC chain with command text, exit code, stdout, stderr
    - _Requirements: 2.5, 2.6_

  - [x] 5.8 Write property test for HMAC audit chain integrity
    - **Property 3: HMAC Audit Chain Integrity**
    - Use proptest with random audit event sequences to verify chain integrity and tamper detection
    - **Validates: Requirements 2.6, 12.1, 12.2, 12.3**

  - [x] 5.9 Write property test for approval required before mutation
    - **Property 19: Approval Required Before Mutation**
    - Use proptest to verify mutating steps are never executed without explicit operator approval
    - **Validates: Requirements 13.3**

- [x] 6. Checkpoint - Core infrastructure verified
  - Ensure all tests pass, ask the user if questions arise.

- [x] 7. Custom Rules Engine
  - [x] 7.1 Define rule condition types and data model
    - Create `crates/helm-monitor/src/custom_rules.rs`
    - Implement `CustomRule`, `RuleCondition` (Simple, Compound, Rate), `CompareOp`, `LogicOp`, `ThresholdValue`
    - Implement `RuleValidationError` with line number and description
    - _Requirements: 23.1, 23.2, 23.3, 23.5_

  - [x] 7.2 Implement TOML parser with validation and error reporting
    - Parse `rules.toml` into `Vec<CustomRule>` with full validation
    - Report syntax errors with line numbers and human-readable suggestions
    - Handle invalid rules individually (valid rules still load)
    - _Requirements: 23.1, 23.6_

  - [x] 7.3 Write property test for rule validation error reporting
    - **Property 27: Rule Validation Error Reporting**
    - Use proptest with random invalid TOML to verify errors include line numbers and descriptions
    - **Validates: Requirements 23.6**

  - [x] 7.4 Implement rule evaluation against SystemSnapshot
    - Implement `CustomRulesEngine::evaluate(&self, snapshot: &SystemSnapshot) -> Vec<Finding>`
    - Support metric path traversal into snapshot fields
    - Implement Simple condition evaluation (gt, lt, eq, ne, contains, matches)
    - Implement Compound condition evaluation (AND/OR logic)
    - Implement Rate condition evaluation (count within time window)
    - _Requirements: 23.2, 23.3, 23.4, 23.5_

  - [x] 7.5 Write property tests for custom rule evaluation
    - **Property 24: Custom Rule Evaluation Correctness**
    - **Property 25: Compound Condition Logic**
    - **Property 26: Rate-Based Condition Detection**
    - Use proptest with random rules × random snapshots to verify correct triggering
    - **Validates: Requirements 23.2, 23.3, 23.5**

  - [x] 7.6 Implement hot-reload on daemon cycle
    - Implement `CustomRulesEngine::reload(&mut self, path: &Path)` called each daemon cycle
    - Detect file modification and reload without restart
    - Integrate custom rules evaluation into the detector pipeline
    - _Requirements: 23.7_

- [x] 8. Custom Service Registry
  - [x] 8.1 Define service types and registry data model
    - Create `crates/helm-monitor/src/custom_service.rs`
    - Implement `ServiceType` enum (DockerContainer, Database, Kubernetes, Pipeline, WebServer, TcpEndpoint, HttpEndpoint)
    - Implement `DbKind` enum (PostgreSQL, MySQL, Redis, MongoDB)
    - Implement `RegisteredService` struct with id, name, service_type, environment, team, consecutive_failures, last_healthy
    - Implement `ServiceRegistry` struct with services vec and config_path
    - _Requirements: 22.2, 22.6_

  - [x] 8.2 Implement health check logic for each service type
    - Docker: check container status via Docker API
    - Database: TCP connect to connection string
    - Kubernetes: kubectl cluster-info
    - Pipeline: HTTP GET expecting 2xx
    - WebServer: HTTP GET matching expected status code
    - TcpEndpoint: TCP connect
    - HttpEndpoint: HTTP GET expecting 2xx
    - _Requirements: 22.2_

  - [x] 8.3 Implement services.toml persistence with atomic write
    - Load from `$XDG_CONFIG_HOME/helm/services.toml`
    - Persist via atomic write (temp-file + rename) with file mode 0600
    - Support add, remove, modify operations without daemon restart
    - Validate connection strings (max 2048 chars), URLs, and configuration on registration
    - Enforce max 256 registered services
    - _Requirements: 22.3, 22.4, 22.5, 22.9_

  - [x] 8.4 Write property test for invalid service registration rejection
    - **Property 23: Invalid Service Registration Rejection**
    - Use proptest with random invalid connection strings and malformed configs to verify rejection with descriptive errors
    - **Validates: Requirements 22.4**

  - [x] 8.5 Implement health check escalation logic
    - After 3 consecutive failures → generate Warning-severity finding
    - After 5 consecutive failures → escalate to Critical severity
    - Reset consecutive_failures on successful health check
    - Update last_healthy timestamp on success
    - _Requirements: 22.7, 22.8_

  - [x] 8.6 Write property test for service health escalation
    - **Property 22: Service Health Escalation**
    - Use proptest with random failure sequences to verify 3→WARN, 5→CRIT escalation
    - **Validates: Requirements 22.7, 22.8**

  - [x] 8.7 Implement CLI/TUI service registration interface
    - Add `helmops service add` CLI command
    - Add `/service add` TUI slash command
    - Support interactive prompts for service type, connection details, environment, team
    - Preserve previously entered fields on validation failure
    - _Requirements: 22.1_

- [x] 9. Checkpoint - Custom rules and service registry complete
  - Ensure all tests pass, ask the user if questions arise.

- [x] 10. Correlation Engine
  - [x] 10.1 Implement correlation engine data structures
    - Create `crates/helm-monitor/src/correlation.rs`
    - Implement `CorrelationGroup`, `CausalLink`, `CausalRelationship` (ResourceDependency, TemporalPrecedence, SharedResource)
    - Implement `CorrelationEngine` with temporal history, dependency graph, and configurable temporal window
    - _Requirements: 20.1, 20.4, 20.5_

  - [x] 10.2 Implement temporal pattern detection
    - Track finding fingerprint timestamps in a sliding window
    - Detect when finding A consistently precedes finding B within the temporal window
    - Record temporal correlations as `CausalRelationship::TemporalPrecedence`
    - _Requirements: 20.4_

  - [x] 10.3 Write property test for temporal correlation detection
    - **Property 20: Temporal Correlation Detection**
    - Use proptest with random finding sequences to verify temporal patterns are detected
    - **Validates: Requirements 20.4**

  - [x] 10.4 Implement resource dependency graph and cascade detection
    - Maintain a dependency graph (resource → dependents)
    - When a resource fails, correlate dependent service findings with root cause
    - Support `add_dependency(resource, dependent)` for known relationships
    - _Requirements: 20.5_

  - [x] 10.5 Write property test for resource dependency cascade
    - **Property 21: Resource Dependency Cascade**
    - Use proptest with random dependency graphs and failures to verify cascade correlation
    - **Validates: Requirements 20.5**

  - [x] 10.6 Implement correlation group formation and integration
    - Group related findings under a single correlation ID with root cause as primary
    - Integrate correlation engine into daemon pipeline (after detection, before alert routing)
    - Wire correlated findings to dashboard display (collapsed groups with "× N" indicator)
    - _Requirements: 20.1, 20.2, 20.3, 5.5, 5.6_

  - [x] 10.7 Write property test for correlation group correctness
    - **Property 10: Correlation Group Correctness**
    - Use proptest with random finding sets sharing resources to verify correct grouping
    - **Validates: Requirements 5.5, 20.1**

- [x] 11. Plan Cache Formalization
  - [x] 11.1 Implement plan cache with TTL and background generation
    - Formalize `PlanCache` with `DashMap<String, PlanState>` and configurable TTL (default 24h)
    - Implement `get`, `ensure_plan` (spawn background generation), `evict_expired`
    - Integrate into daemon loop for new fingerprints
    - _Requirements: 6.1, 6.2, 6.3, 6.4_

  - [x] 11.2 Write property test for plan cache TTL behavior
    - **Property 11: Plan Cache TTL Behavior**
    - Use proptest with random fingerprints × timestamps to verify cache hit within TTL and miss after expiry
    - **Validates: Requirements 6.2**

  - [x] 11.3 Implement plan step completeness validation
    - Ensure every generated plan step includes: command text, expected effect, risk level, blast radius, rollback instructions, required Capability
    - Validate plan structure before caching
    - _Requirements: 6.5, 6.6, 13.2_

  - [x] 11.4 Write property test for plan step completeness
    - **Property 12: Plan Step Completeness**
    - Use proptest to verify all required fields are present in generated plan steps
    - **Validates: Requirements 6.5, 6.6, 13.2**

- [x] 12. Checkpoint - Correlation and plan cache complete
  - Ensure all tests pass, ask the user if questions arise.

- [x] 13. Finding Model Enhancements
  - [x] 13.1 Implement finding severity ordering and lifecycle
    - Ensure findings are always sorted by severity descending, then by timestamp descending within same severity
    - Implement `FindingLifecycle` transitions: Open → SelfResolved when finding no longer detected
    - _Requirements: 3.3, 10.5, 24.2_

  - [x] 13.2 Write property test for finding severity ordering
    - **Property 5: Finding Severity Ordering**
    - Use proptest with random finding sets to verify correct ordering
    - **Validates: Requirements 3.3, 24.2**

  - [x] 13.3 Write property test for finding lifecycle self-resolution
    - **Property 16: Finding Lifecycle Self-Resolution**
    - Use proptest with random finding lifecycle sequences to verify SelfResolved transition
    - **Validates: Requirements 10.5**

  - [x] 13.4 Implement finding field completeness enforcement
    - Ensure all findings (built-in and custom) have non-empty detector_id, affected_resource, valid severity, valid confidence, and at least one EvidenceRef
    - Add validation in detector registry before findings are emitted
    - _Requirements: 5.2, 5.3, 10.2_

  - [x] 13.5 Write property test for finding field completeness
    - **Property 9: Finding Field Completeness**
    - Use proptest with random snapshots through all detectors to verify field completeness
    - **Validates: Requirements 5.2, 5.3, 10.2, 23.4**

  - [x] 13.6 Implement finding title constraints
    - Ensure all finding titles are ≤80 characters, state affected resource and current state in plain language
    - Audit all 34+ existing detectors for title compliance
    - _Requirements: 24.3_

  - [x] 13.7 Write property test for finding title length and readability
    - **Property 28: Finding Title Length and Readability**
    - Use proptest with random snapshots through all detectors to verify title constraints
    - **Validates: Requirements 24.3**

- [x] 14. Alerting System Verification and Enhancement
  - [x] 14.1 Verify and enhance alert severity routing
    - Verify AlertRouter routes findings only when severity ≥ configured minimum
    - Ensure webhook, Slack, and PagerDuty sinks respect severity threshold independently
    - Load alerting config from `$XDG_CONFIG_HOME/helm/alerting.toml`
    - _Requirements: 11.1, 11.4, 11.5, 11.6_

  - [x] 14.2 Write property test for alert severity routing
    - **Property 13: Alert Severity Routing**
    - Use proptest with random findings × config to verify severity-based routing
    - **Validates: Requirements 11.1**

  - [x] 14.3 Implement alert deduplication window
    - Suppress duplicate alerts for same fingerprint within configurable dedup window (default 300s)
    - Track last-routed timestamp per fingerprint
    - _Requirements: 11.2_

  - [x] 14.4 Write property test for alert deduplication
    - **Property 14: Alert Deduplication**
    - Use proptest to verify same-fingerprint alerts within dedup window are suppressed
    - **Validates: Requirements 11.2**

  - [x] 14.5 Implement alert rate limiting
    - Enforce rate_limit_per_min per sink to prevent alert storms
    - Drop excess alerts beyond the configured limit within any 60-second window
    - _Requirements: 11.3_

  - [x] 14.6 Write property test for alert rate limiting
    - **Property 15: Alert Rate Limiting**
    - Use proptest with burst finding sequences to verify rate limit enforcement
    - **Validates: Requirements 11.3**

  - [x] 14.7 Implement PagerDuty resolution events
    - Send resolution event when finding lifecycle transitions to Resolved or SelfResolved
    - Use finding fingerprint as dedup_key
    - _Requirements: 11.7_

- [x] 15. Checkpoint - Alerting and findings complete
  - Ensure all tests pass, ask the user if questions arise.

- [x] 16. TUI Dashboard Refinement
  - [x] 16.1 Implement 6-tab layout
    - Update TUI to support 6 tabs: ALERTS (default), SERVICES, RESOURCES, LOGS, FLEET, CHANGES
    - Navigate via arrow keys or number keys 1-6
    - Implement RESOURCES sub-sections (CPU/Memory, Disk, Network, Processes) with keyboard toggles
    - _Requirements: 7.7, 7.8_

  - [x] 16.2 Implement single-line summary and header bar
    - Display header: hostname, kernel version, uptime, fleet status, current time, LLM provider status
    - Display single-line summary (≤120 chars) at top with critical issue count and brief descriptions
    - _Requirements: 7.1, 24.4_

  - [x] 16.3 Write property test for summary line length
    - **Property 29: Summary Line Length**
    - Use proptest with random finding sets to verify summary ≤120 chars and accurate critical count
    - **Validates: Requirements 24.4**

  - [x] 16.4 Implement fingerprint-based selection stability
    - Replace positional index selection with fingerprint-based pinning
    - Maintain selection across refresh cycles when fingerprint still exists
    - Fall back to clamped position if fingerprint disappears
    - _Requirements: 4.3, 4.4_

  - [x] 16.5 Write property test for fingerprint selection stability
    - **Property 6: Fingerprint Selection Stability**
    - Use proptest with random findings lists and selection to verify stability across refresh
    - **Validates: Requirements 4.3, 4.4**

  - [x] 16.6 Implement responsive collapse for narrow terminals
    - Collapse two-pane layout to single-pane when width < 120 columns
    - Show detail on Enter key press in single-pane mode
    - Display "larger terminal required" when width < 60 or height < 10
    - _Requirements: 7.9, 7.10_

  - [x] 16.7 Implement beginner mode tooltips and help
    - Add contextual tooltips explaining each section
    - Toggle via help mode hotkey
    - Display hotkey hints in bottom status bar with refresh countdown
    - _Requirements: 7.6, 13.6, 24.8_

- [x] 17. Web Console Enhancement
  - [x] 17.1 Implement mobile-responsive card-based layout
    - Update `helm-cli/src/web/dashboard.html` for card-based design
    - Minimum font size 14px, minimum 16px spacing between cards
    - No more than 2 visual elements per card (severity indicator + title)
    - Render without horizontal scrolling on viewports ≥320px
    - Touch targets ≥44x44px for interactive elements
    - _Requirements: 8.2, 24.6, 24.7_

  - [x] 17.2 Implement WebSocket live updates
    - Establish WebSocket connection for real-time finding updates
    - Push new snapshot data to all connected clients on each daemon cycle
    - Handle disconnection with automatic reconnection
    - _Requirements: 8.3_

  - [x] 17.3 Implement keyboard shortcuts for web console
    - Support: 1-9 for tab switching, Enter for selection, Escape for clearing, Alt+G for plan guidance, F5 for refresh
    - Display operator cards: daemon status, update pressure, storage risk, runtime counts, collector health
    - _Requirements: 8.5, 8.6_

- [x] 18. Checkpoint - Dashboard refinement complete
  - Ensure all tests pass, ask the user if questions arise.

- [x] 19. Pipeline Monitoring
  - [x] 19.1 Implement pipeline collector
    - Create `crates/helm-monitor/src/pipeline.rs`
    - Implement `PipelineCollector` that monitors registered CI/CD endpoints
    - Surface: last build status, build duration trend, failure rate, currently running builds
    - Support webhook-based push notifications from GitHub Actions, GitLab CI, Jenkins
    - _Requirements: 25.1, 25.3_

  - [x] 19.2 Implement config drift detector
    - Create config drift detection comparing current system state against declared baseline
    - Generate findings with specific configuration key, expected value, and actual value
    - _Requirements: 25.4, 25.5_

  - [x] 19.3 Write property test for config drift detection
    - **Property 32: Config Drift Detection**
    - Use proptest to verify deviations generate findings with key, expected, and actual values
    - **Validates: Requirements 25.4, 25.5**

  - [x] 19.4 Implement pipeline failure finding generation
    - Generate Critical finding when pipeline build fails with pipeline name, failure stage, and log reference
    - Track deployment frequency and change failure rate as metrics in RESOURCES tab
    - _Requirements: 25.2, 25.6_

  - [x] 19.5 Write property test for pipeline failure finding generation
    - **Property 33: Pipeline Failure Finding Generation**
    - Use proptest to verify failed builds generate Critical findings with required fields
    - **Validates: Requirements 25.2**

- [x] 20. Logs Tab Enhancement
  - [x] 20.1 Implement log anomaly detection and spike promotion
    - Display recent journal errors grouped by unit with occurrence count and last-seen timestamp
    - Highlight new error patterns not present in previous snapshot cycle
    - Promote patterns exceeding spike threshold to Warning/Critical findings
    - Support filtering by unit name, severity level, and time range
    - _Requirements: 17.1, 17.2, 17.3, 17.4, 17.5_

  - [x] 20.2 Write property test for log spike promotion
    - **Property 30: Log Spike Promotion**
    - Use proptest to verify patterns exceeding threshold are promoted to findings
    - **Validates: Requirements 17.3**

  - [x] 20.3 Write property test for log filter correctness
    - **Property 31: Log Filter Correctness**
    - Use proptest with random filter combinations to verify only matching entries are displayed
    - **Validates: Requirements 17.4**

  - [x] 20.4 Write property test for new log pattern highlighting
    - **Property 36: New Log Pattern Highlighting**
    - Use proptest to verify new patterns are highlighted when absent from previous cycle
    - **Validates: Requirements 17.2**

- [x] 21. Fleet and Security Verification
  - [x] 21.1 Verify parallel SSH collection and credential abstraction
    - Verify `RemoteRegistry::parallel_collect_reports` uses tokio::JoinSet for concurrent collection
    - Verify 20-host collection completes within 2s timeout
    - Verify credential abstraction (SshAgent, KeyFile, Password) with no passwords in registry file
    - Extend remote findings with source host_id (UUID) for traceability
    - _Requirements: 9.1, 9.2, 9.5, 9.7_

  - [x] 21.2 Write property test for remote finding host traceability
    - **Property 34: Remote Finding Host Traceability**
    - Use proptest to verify all remote findings include source host_id UUID
    - **Validates: Requirements 9.7**

  - [x] 21.3 Implement unreachable host handling
    - Mark unreachable hosts as down with staleness indicator
    - Retain last-known findings for unreachable hosts
    - Display fleet summary in header: "fleet: N/M up"
    - _Requirements: 9.3, 9.6_

  - [x] 21.4 Write property test for unreachable host staleness
    - **Property 35: Unreachable Host Staleness**
    - Use proptest to verify unreachable hosts are marked down with staleness indicator
    - **Validates: Requirements 9.6**

  - [x] 21.5 Verify HMAC chain and audit trail completeness
    - Verify `helmops audit verify` checks entire chain and reports tampering/gaps
    - Verify secrets store permissions (0600 file, 0700 parent)
    - Verify API key resolution order: CLI flag → secrets store → env var
    - _Requirements: 12.3, 12.5, 12.6, 12.7_

- [x] 22. Checkpoint - Fleet and security verified
  - Ensure all tests pass, ask the user if questions arise.

- [x] 23. Services Tab and Changes Tab
  - [x] 23.1 Implement Services tab
    - Display all loaded systemd units with active state, sub-state, and description
    - Sort failed units to top with Critical severity highlighting
    - Display systemd timers with last-trigger, next-trigger, and overdue status
    - Show recent journal output for selected service
    - Provide quick-action hotkeys (restart, stop, view journal) — all capability-gated
    - _Requirements: 18.1, 18.2, 18.3, 18.4, 18.5_

  - [x] 23.2 Implement Changes tab
    - Display chronological list of Changesets with timestamp, plan title, status, affected resources
    - Show step-by-step execution log with stdout/stderr for selected Changeset
    - Display pre/post execution snapshot diffs
    - Support rollback initiation using stored PreChangeBackups
    - Correlate Changesets with findings (show which finding triggered each change)
    - _Requirements: 16.1, 16.2, 16.3, 16.4, 16.5_

- [x] 24. Doctor and Self-Diagnostics
  - [x] 24.1 Implement helmops doctor command
    - Verify: database accessibility, secrets store permissions, provider reachability, required binaries (systemctl, journalctl, df, ss), collector prerequisites
    - Report each check as PASS/WARN/FAIL with human-readable explanation
    - Support --json output for automation
    - Suggest packages for missing binaries based on detected distribution
    - Verify HMAC audit chain integrity
    - _Requirements: 21.1, 21.2, 21.3, 21.4, 21.5_

- [x] 25. Workspace Finalization and Binary Entry Points
  - [x] 25.1 Finalize binary entry points and CLI structure
    - Ensure single binary `helmops` from helm-cli crate
    - Maintain `helm` as compatibility alias
    - Default command launches web console; `helmops tui` launches TUI
    - Implement `helmops completion` for bash, zsh, fish
    - Verify workspace members: helm-core, helm-providers, helm-memory, helm-monitor, helm-cli
    - _Requirements: 19.1, 19.2, 19.3, 19.5_

  - [x] 25.2 Implement zero-config instant dashboard launch
    - Render first frame within 500ms of invocation (async first-frame, defer collection)
    - Begin first collection cycle within 100ms of first frame
    - Display finding or healthy-state indicator within 5s of start
    - Operate without config file, API key, or prior setup for local monitoring
    - Show inline hint when no LLM provider configured
    - _Requirements: 3.1, 3.2, 3.4, 3.5, 3.6, 3.7, 3.8_

- [x] 26. Checkpoint - Feature implementation complete
  - Ensure all tests pass, ask the user if questions arise.

- [x] 27. Performance Benchmarks
  - [x] 27.1 Implement criterion benchmarks
    - Create benchmarks for: dashboard render (≤16ms at 60 findings + 5 sparklines), sparkline render (≤4ms at 60 points × 8 metrics), ring buffer push (O(1)), fleet collect (≤2s for 20 hosts), finding sort (200 findings within render budget)
    - Add `proptest.toml` to workspace root with `cases = 100`, `max_shrink_iters = 1000`
    - _Requirements: 14.1, 14.2, 14.3, 14.4_

- [x] 28. Integration Tests
  - [x] 28.1 Write integration tests for full daemon cycle
    - Test collect → detect → correlate → alert pipeline with mock sinks
    - Test fleet SSH with mock SSH server
    - Test web console HTTP/WebSocket round-trip with real Axum server
    - Test plan generation with mock LLM provider
    - Test database persistence round-trip for snapshots, findings, changesets
    - _Requirements: 4.1, 4.2, 8.3, 9.1_

- [x] 29. CI Gate Verification
  - [x] 29.1 Verify release gate passes
    - Run full CI gate: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets`, `cargo build --release -p helm-cli`
    - Verify property tests pass with 100 iterations
    - Verify benchmarks do not regress beyond 10%
    - _Requirements: 1.9, 19.4_

- [x] 30. Final checkpoint - All tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- Each task references specific requirements for traceability
- Checkpoints ensure incremental validation after each major phase
- Property tests validate universal correctness properties from the design document (36 total)
- Unit tests validate specific examples and edge cases
- The project uses Rust with the `proptest` crate for property-based testing and `criterion` for benchmarks
- All property tests should use minimum 100 iterations per the design's testing strategy
- The workspace uses `tokio` for async runtime, `ratatui` for TUI, `axum` for web server

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1", "1.2", "1.3", "1.4"] },
    { "id": 1, "tasks": ["1.5"] },
    { "id": 2, "tasks": ["1.6"] },
    { "id": 3, "tasks": ["3.1", "4.1", "5.1"] },
    { "id": 4, "tasks": ["3.2", "3.3", "4.2", "4.3", "5.2", "5.3"] },
    { "id": 5, "tasks": ["4.4", "5.4", "5.5", "5.7"] },
    { "id": 6, "tasks": ["5.6", "5.8", "5.9"] },
    { "id": 7, "tasks": ["7.1", "8.1", "10.1", "11.1", "13.1"] },
    { "id": 8, "tasks": ["7.2", "8.2", "10.2", "11.2", "13.2", "13.3"] },
    { "id": 9, "tasks": ["7.3", "7.4", "8.3", "10.3", "10.4", "11.3", "13.4"] },
    { "id": 10, "tasks": ["7.5", "7.6", "8.4", "8.5", "10.5", "10.6", "11.4", "13.5", "13.6"] },
    { "id": 11, "tasks": ["8.6", "8.7", "10.7", "13.7"] },
    { "id": 12, "tasks": ["14.1", "16.1", "19.1", "20.1"] },
    { "id": 13, "tasks": ["14.2", "14.3", "16.2", "16.4", "19.2", "19.4", "20.2", "20.3", "20.4"] },
    { "id": 14, "tasks": ["14.4", "14.5", "14.7", "16.3", "16.5", "16.6", "16.7", "19.3", "19.5"] },
    { "id": 15, "tasks": ["14.6", "17.1", "17.2", "17.3"] },
    { "id": 16, "tasks": ["21.1", "21.3", "21.5", "23.1", "23.2", "24.1"] },
    { "id": 17, "tasks": ["21.2", "21.4"] },
    { "id": 18, "tasks": ["25.1", "25.2"] },
    { "id": 19, "tasks": ["27.1", "28.1"] },
    { "id": 20, "tasks": ["29.1"] }
  ]
}
```
