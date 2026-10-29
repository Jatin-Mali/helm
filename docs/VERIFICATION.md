# Independent verification of the 14-task repair

Review date: 2026-10-26. Starting revision: `589ead8`, branch
`repair/verified-project-completion`. This review preserves the user's existing
`AGENTS.md`, `.codex/`, graph, audit and browser artifacts. No release was published.

## Assessment

Claude's statement that all 14 tasks were complete was too broad. The implementation
and substantial regression coverage exist, but the first independent workspace run
failed, and Task 14 never demonstrated the required fresh supported VM. Its checkbox
and ledger heading have been corrected. Passing containers and mock service actions
does not complete fresh-machine acceptance.

This review follows the repair specification, plan, ledger, source paths, and actual
test results. It is a verification against those contracts, not a proof that every
deployment or every line of code is defect-free. The usage/presentation companion is
[PROJECT_GUIDE.md](PROJECT_GUIDE.md).

## Defects corrected during review

| Problem | Correction and evidence |
|---|---|
| Redacting serialized JSON as ordinary text could corrupt escaping inside log messages and prevent durable change preparation | `crates/helm-core/src/secret.rs` now redacts decoded JSON strings and re-encodes valid JSON. The first full run failed in `earlier_tui_denial_survives_final_approval`; a small nested-credential regression independently failed before the fix and passed after it. String and numeric credential fields remain redacted. |
| Read-write mounts with `errors=remount-ro` were classified as read-only | `fs_readonly.rs` now checks an exact comma-delimited `ro` option. Regression reproduced one false finding for `rw,relatime,errors=remount-ro` before the fix and also checks genuine `ro`. |
| Top-level help, nested help with descriptions, and version output could exit with failure | Preserve Clap's structured error and its exit code instead of guessing from the first line of output. A process-level regression checks both binaries and also rejects invalid arguments. |
| Ollama cloud models and remote endpoints could be labeled local inference | Shared model/endpoint classification is used by trust-report, TUI and web. Tests cover `gemma4:31b-cloud`, `glm-5.1:cloud`, a local model and a remote endpoint. Classification cannot detect an operator's custom forwarding proxy. |
| The real-agent demo driver always returned success and could lose an already-reaped child's status | Propagate exit status and return 124 on timeout. Regression tests reproduced both failures. New exact-match rules also required stripping the approval heading, normalizing terminal line endings and accepting relative fixture paths; regressions reproduced those mismatches. All five driver tests now pass, including rejection of other files and prior-output authorization attempts. |
| Demo assertions could overstate coverage | Require real baseline audit rows instead of `or True`; compare actual local/remote collection identities instead of hostnames; restrict positive-demo approvals to the intended file write and exact test command; reject invalid soak durations. |
| A release asset job could run without a successful advisory scan | Its dependencies now include both `test` and `advisories`; driver regressions also run in CI. Workflow configuration was inspected locally; no hosted release job was triggered. |
| Setup and completion documentation overstated behavior | Correct required troubleshoot argument, integer watch interval, Chromium backend, shell authority, cloud inference, retention estimates and sandbox help. Reopen fresh-VM acceptance and distinguish CLI-only outcome tests from separate TUI/web tests. |

## Task-by-task evidence map

Paths below are relative to the repository root. Test names and files are the durable
reproduction steps; historical ledger statements alone are not fresh evidence.

| Task | Implementation/evidence reviewed | Qualification |
|---|---|---|
| 1. Baseline and build | Workspace fmt, strict Clippy, feature compilation, release build; telemetry and release workflow | Builds establish compilation, not delivery to a telemetry collector |
| 2. Canonical plans and approvals | `tests/approval_integrity.rs`, `tests/v20_controlled_execution.rs`; TUI approval tests | Exact plan revision/step/cwd/host decisions; denial and tamper regressions |
| 3. Process and policy boundary | `tests/execution_policy.rs`, `helm-cli/tests/execution_modes.rs`, process-runner tests | Cancellation/output/deadlines and no-effect policy cases; sandbox is refused |
| 4. Durable changes and restoration | `tests/change_recovery.rs`, CLI execution-journal tests, operations fixture | File backups, changed-file refusal, missing/corrupt backup and interrupted-state cases; arbitrary shell remains manual recovery |
| 5. Honest monitoring | Reporter/collector/detector tests, `tests/v18_monitor.rs`, `helm-cli/tests/service_findings.rs` | Unavailable or incomplete coverage is not healthy; new mount regression fixes a false positive |
| 6. Paths, fleet, retention | `helm-cli/tests/runtime_paths.rs`, `helm-cli/tests/fleet.rs`, snapshot prune tests, Docker SSH scenario | Two SSH containers are not two VMs; referenced snapshots can exceed the retention target |
| 7. Privacy/configuration/audit | `tests/privacy_boundaries.rs`, core redaction/private-filesystem tests, config/secrets/audit tests, upgrade scenario | Pattern redaction has limits; an unkeyed chain needs off-host anchoring against same-user rewriting |
| 8. Web and webhook boundaries | `helm-cli/tests/web_security.rs`, web review tests, webhook signature/replay tests, real Chromium pairing | UI pairing, authenticated access, CSRF and live rendering checked; real service mutation is mocked |
| 9. Planning and responsive TUI | PlanCache single-owner/retry tests, TUI pending-provider/cancellation tests and persisted-plan tests | Automated event/state tests; no exhaustive manual terminal-keyboard audit |
| 10. Agent runtime | `crates/helm-agent/tests/agent_runtime.rs` | Tool turns, malformed calls, retry/step/context/budget limits, cancellation and evidence |
| 11. Host tools and permissions | `crates/helm-agent/tests/agent_security.rs` plus browser-required run | Denial, tamper, replay, links, protected paths, destinations, real static browser; approved shell has broad OS authority |
| 12. Sessions and workflows | `helm-cli/tests/agent_workflows.rs`, TUI agent tests, real-provider project demonstration | Approve/deny/resume/SIGINT and both binaries; remote general-agent tasks remain unsupported |
| 13. Distribution and features | `scripts/test-install.sh`, alert mock tests, daemon lifecycle, optional builds, advisory scan, release YAML | Local checks do not prove a published GitHub asset or delivery to real Slack/PagerDuty accounts |
| 14. Fresh-machine demonstrations | `scripts/verify-demo.sh`, operations/upgrade/fleet/agent/soak drivers | **Partial:** isolated state and Docker scenarios exist; required fresh VM and real disposable systemd service remain unverified |

The file named `cross_frontend.rs` invokes the CLI. Its name does not prove a full
CLI/TUI/web Cartesian outcome matrix; the other frontends have their own review and
policy tests. The original matrix wording has been corrected accordingly.

## Fresh command results

| Check | Fresh result |
|---|---|
| `cargo fmt --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --workspace --all-targets` | PASS: 924 passed, 2 normally ignored |
| Explicit `test_fleet_parallel_refresh_20_hosts -- --ignored --nocapture` | PASS: both binary targets (2 additional executions); reserved `.invalid` hosts, not 20 real hosts |
| `cargo check --workspace --all-targets --all-features` | PASS |
| `cargo check --workspace --all-targets --no-default-features` | PASS |
| `cargo build --release -p helm-cli` | PASS: both binary names |
| `cargo audit --deny warnings` | PASS for the resolved dependency set |
| Python acceptance-driver regression tests | PASS: 5 tests |
| Invalid/missing soak-duration checks; shell syntax; local doc links; diff whitespace | PASS |
| Final strict integration harness against rebuilt release and saved real baseline | PASS: all 11 categories, zero FAIL/UNVERIFIED; final run used `--strict --soak-minutes 1` |
| Separate 10-minute daemon soak at 5-second interval | PASS: 102 snapshots; clean stop |

The initial full suite failed at durable TUI preparation. Each later source change
was followed by fresh checks; the final 924-test result includes numeric credential
masking, the exact mount-option detector and all frontend privacy labels.

The first long strict attempt failed during old-binary fixture creation and the
real-provider scenario; its overall result was FAIL. A fresh upgrade run then passed,
retaining 2 baseline snapshots, 3 legacy audit events and the baseline change set.
The old binary's own audit verification still fails on its historical bare `read`
capability; the new binary verifies it. The first fixture failure was not reproduced;
the driver now includes failure stderr for future diagnosis.

Cloud testing initially returned DNS/HTTP 502; resolution recovered. Restrictive
demo approvals then exposed the heading/line-ending and relative-path mismatches
described above. A later run actually fixed the file and passed its tests but
remained `failed` because unrelated listing commands had been denied. The runtime's
conservative outcome rule was retained. The positive fixture now states its two
filenames and allowed tools explicitly; it passes reproduce-failure → inspect →
approved edit → passing check → completed session. This is a prepared reproducible
scenario, not evidence that every free-form model task succeeds.

Additional real-provider checks passed with Ollama `gemma4:31b-cloud`: exact approved
`uname -s && uptime` diagnostic with saved tool evidence; an untrusted README read
without executing its injected write; all requested writes denied with no target
file created and a failed outcome; and SIGINT cancellation with a saved cancelled
session. Direct `/proc/` shell reads were correctly refused before approval.

Ten-minute soak measurements: single standard collection 0.75 s; daemon RSS
29.0 → 34.7 MiB; database/WAL/SHM footprint 5.4 → 27.0 MB; approximately 0.223 MB per
additional snapshot (about 223 MB per 1000 similar snapshots). This is an estimate,
not a disk cap: protected references, audit, backups and sessions add retention.
The daemon stopped cleanly. These measurements came from the earlier long-run
attempt; the final rebuilt binary separately passed the strict harness with a
one-minute soak. Neither run proves multi-day stability.

The current advisory scan fetched RustSec data and checked 472 resolved dependencies:
no advisories or warnings were reported. This result applies to the local resolved
dependency set. `Cargo.lock` is currently ignored by the repository, so another
resolution can differ. The usual Cargo warning about `helm` and `helmops` sharing
`main.rs` is present; it is not a Clippy lint failure.

Real-browser checks on temporary read-only state: anonymous pairing form visible;
wrong code rejected; correct code removes the form and opens a live WebSocket;
refresh renders host observations, findings and coverage; `/api/session` returns
200 after pairing; POST `/api/refresh` without CSRF returns 403; pairing input is
`type=password` and cleared after success. Expected 401/403 requests appear in the
browser console. The temporary browser and server are closed after the check.

`graphify update .` regenerated the source graph with AST extraction only. SQL
relationships remain partial because `tree_sitter_sql` is not installed; no semantic
API rebuild was performed. The fork-identity scan found only the mandated scan
examples in project instruction/spec files, not incorrect installer/repository URLs.

## Reproduce

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo check --workspace --all-targets --all-features
cargo check --workspace --all-targets --no-default-features
cargo build --release -p helm-cli
cargo audit --deny warnings
python3 -m unittest discover -s tests/demo -p test_drivers.py
HELM_DEMO_PROVIDER=ollama HELM_DEMO_MODEL=gemma4:31b-cloud \
  bash scripts/verify-demo.sh --strict --soak-minutes 10
```

The strict harness requires a usable real provider, Chrome/Chromium, Docker and SSH
tooling; it must fail when a required integration is unavailable. Tests and demos
use temporary state. Real integrations may require running outside a restricted
network/process sandbox. No key values belong in the report or captured evidence.

## Remaining acceptance limits

- A fresh supported VM and a disposable real systemd unit have not been demonstrated.
- Container SSH tests do not establish VM lifecycle behavior or host isolation.
- Live coverage of other providers, real external alert accounts, Kubernetes,
  libvirt and database deployments has not been established by fixture tests.
- Short soak measurements do not establish multi-day reliability or a hard disk cap.
- Real power loss during a mutation was not tested; interrupted/corrupt-state paths
  are covered by failure injection and recovery tests.
- Browser support is a static reader; JavaScript applications, remote general-agent
  execution and sandbox confinement remain unsupported.

These are explicit limits on the completion claim. They are not silently converted
to PASS and do not justify moving a release tag without the release gate.

## Refine-and-polish pass (2026-10-29)

Plan: [2026-10-29-refine-and-polish](superpowers/plans/2026-10-29-refine-and-polish.md). Feature matrix: [FEATURE_MATRIX.md](FEATURE_MATRIX.md). UI audit: [web-ui-audit.md](web-ui-audit.md).

Gate at `865df7b`: `cargo fmt --check` PASS, `cargo clippy --workspace --all-targets -- -D warnings` PASS, `cargo test --workspace --all-targets` 946 passed / 0 failed, dashboard JS regression tests 7/7.

`scripts/verify-demo.sh --soak-minutes 1` (non-strict):

| Check | Result |
|---|---|
| Dashboard interaction / acceptance driver regression tests | PASS |
| Agent workflows (scripted provider), agent security | PASS |
| Cross-frontend outcomes | PASS |
| Dashboard authentication, origin and CSRF | PASS |
| Installer | PASS |
| Operations demonstration (finding → explain → approved change → rollback) | PASS |
| Fleet over two disposable SSH hosts | PASS |
| Prolonged monitoring run (1 min) | PASS |
| Database upgrade, backup and recovery | FAIL in the run, then PASS (18/18) after the fix below |
| General agent with a real provider | UNVERIFIED (no `HELM_DEMO_PROVIDER`/`HELM_DEMO_MODEL` set) |

The upgrade failure was in the **baseline** (`fbbbec3`) debug build: its journal context offset overflows an `i8` on hosts with more than 128 journal entries and the debug overflow trap aborts it before it writes state. Current code computes the offset in `isize` (regression `journal_context_crosses_signed_byte_boundary`). The script now builds the baseline in release mode, as that version shipped; `tests/demo/upgrade.py` against it passes every check.

Web behaviour was additionally verified in Chromium (Webcmd) at 1440×900 and 390×844, light and dark: pairing, plan generation with polling, reload during generation, provider failure with Retry, selection across refreshes, no horizontal overflow.

Not verified in this pass: a real LLM provider end to end, physical touch devices and screen readers, a fresh VM.
