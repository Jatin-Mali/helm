# Refine-and-polish verification matrix

2026-10-29. Starting revision `a712ac8`; existing uncommitted work preserved. Claude had completed Phase A (Tasks 1–4) plus its three important review fixes. Tasks 5–7 had not been completed. This continuation executes inline.

PASS means the stated scope was exercised. OUT OF SCOPE identifies an external acceptance prerequisite, not a successful test. Fixture evidence proves the local contract; it does not certify a real remote deployment. No live host service was restarted or stopped.

## Web console

| Feature | Frontend | Expected | Observed | Result | Evidence |
|---|---|---|---|---|---|
| Pairing and replay | Web | Single-use code, protected APIs/WS | Paired isolated browser; replay and anonymous access rejected | PASS | Browser; `web_security`, `web_auth::tests` |
| Session expiry / non-JSON failure | Web | Pair again on 401; useful error instead of exception | Request helper and authentication regressions; browser HTTP 503 gives a useful error | PASS | `web_security`; dashboard JS tests; browser action check |
| Overview | Web | Incident evidence, checks, deterministic and AI plans | Empty and populated fixtures render; real incident selected | PASS | Chromium 1440/390 captures; fixture sweep |
| Services | Web | Units, journal, timers, sites, pipelines | Empty and populated data render; pipeline delivery visible | PASS | Browser fixture sweep and signed ingress |
| Containers | Web | Container/compose/process details | Empty and populated data render | PASS | Browser fixture sweep; desktop/mobile captures |
| Kubernetes | Web | Pods and VMs, including no integration | Empty and populated data render | PASS | Browser fixture sweep; unsupported collectors remain labeled |
| Data | Web | Database, package, backup and deployment information | Empty and populated data render | PASS | Browser fixture sweep |
| Logs | Web | Filterable patterns with drill-down | Pattern context renders; typing `fixture.service` retains focus | PASS | Browser filter and populated fixture checks |
| Network | Web | Ports, interfaces, security information | Empty and populated data render | PASS | Browser fixture sweep |
| Storage | Web | Filesystem pressure and SMART details | Empty and populated data render; native usage meter | PASS | Browser fixture sweep |
| Fleet | Web | Zero-host state and separate observed/unavailable hosts | Empty state and two-host fixture render | PASS | Browser fixture sweep; `helm-cli/tests/fleet.rs` |
| Changes | Web | History and exact rollback review | Empty/populated history renders; missing rollback is a clear error | PASS | Browser; `web::tests`, `cross_frontend`, operations demo |
| Incident search | Web | Filter without losing selection identity | Stable fingerprint selection preserved; filtered queue renders | PASS | Phase A browser evidence; JS/browser rendering |
| Keyboard | Web | j/k, Enter, Esc, 1–0, Alt+G, F5; native controls remain usable | Enter on Services now opens Services; log input retains focus; modal shortcuts gated | PASS | Dashboard regression tests; Chromium keyboard walkthrough |
| Service review/journal/restart/stop | Web | Exact single-use review before mutation | Stub stop executes only after valid review; altered/replayed request refused; journal capability gate tested | PASS | `web_security`, `web::tests`; no production service targeted |
| Rollback supported/unsupported | Web/CLI | Restore authenticated backup; reject unsupported/newer state | Restores original bytes; refuses newer edit; nonexistent change gives clear error | PASS | Operations demo, `change_recovery`, browser action check |
| Plan lifecycle | Web | Ready/failed/disabled/loading visible and retryable | State survives reload, stale rows fail, retry queues explicitly | PASS | 12 `web_plan` integration tests; prior Phase A browser lifecycle |
| Connection failure | Web/TUI | Explain endpoint/connection problem | Refused TCP connection says “Cannot reach the provider” | PASS | RED→GREEN `unreachable_provider_reports_connection_failure`; browser |
| Shared collection | Web | Readers reuse current collection | Cached readers and concurrent refresh guards pass | PASS | `web_plan` integration tests |
| Pending actions | Web | Duplicate clicks and snapshot renders cannot repeat a request | Request owner lives outside the replaced DOM | PASS | RED→GREEN dashboard JS regression |
| Signed pipeline webhook | Web | Authenticate, ingest, display result | Signed GitHub fixture accepted (200); failed deployment appears in Services | PASS | Isolated loopback ingress + browser; webhook replay/auth unit tests |
| Responsive layout | Web | All actions reachable at 1440/720/390 | All 10 tabs have no page overflow at 1440/390; final queue rows fit at all 3 widths | PASS | Before/after captures, final content-fit measurement |
| Light/dark/reduced motion | Web | System theme, readable states, visible focus | Both palettes rendered; reduced-motion mode exercised | PASS | Chromium emulation, CSS review, `web-ui-audit.md` |
| Physical touch / screen reader | Web | Assistive-device compatibility | No physical device or screen reader available | OUT OF SCOPE | Keyboard and viewport checks are not physical-device certification |

## CLI and TUI

| Feature | Frontend | Expected | Observed | Result | Evidence |
|---|---|---|---|---|---|
| All CLI help routes and version | CLI | Both binaries exit successfully for supported help | Recursive sweep: 70 help routes pass; invalid input still fails | PASS | Release binary sweep; `cli_help` regression |
| Config round trip | CLI/TUI | Valid typed metadata persists; invalid edit preserves file | Provider kind/model set/get/validate succeeds; bad structure rejected | PASS | Isolated CLI; `config_set_rejects_values_the_config_cannot_load` |
| Secrets | CLI/TUI | Only dedicated secrets store persists keys | List works; env keys are not copied on startup; masking and redaction tests pass | PASS | Secrets/TUI/privacy tests; isolated CLI |
| Permissions | CLI/TUI | List/grant/revoke exact capabilities | List works; permission consumption and approvals verified | PASS | Isolated CLI; permission and approval tests |
| Alternate database path | CLI | `--db-path` used without touching default DB | Regression passes | PASS | `runtime_paths` |
| Snapshot | CLI | Real observation, valid JSON | `snapshot --profile quick --json` emits valid JSON | PASS | Isolated release CLI |
| Monitor | CLI | Domain filtering, valid JSON | Quick services monitor emits valid JSON | PASS | Isolated release CLI; collectors/detectors tests |
| Empty/two-host fleet monitor | CLI | JSON array even when no hosts; no local substitution | Empty case fixed RED→GREEN; observed/down/stale-host fixture passes | PASS | `fleet` integration tests |
| Explain/troubleshoot/incident plan/apply-plan | CLI | Evidence-backed plans and explicit approved mutation | Operations demo exercises finding, explain, plan/apply and durable result | PASS | `tests/demo/ops.py`, execution tests |
| Audit / trust-report | CLI | Valid audit chain and truthful inference boundary | Verification/trust report work with isolated DB | PASS | Audit/privacy tests; release CLI |
| Run agent / resume / sessions | CLI | Bounded execution and exact approvals | Approve/deny/resume/cancel and sessions tested | PASS | `agent_workflows`; isolated `run --sessions` |
| Daemon start/stop/status | CLI | Isolated state and truthful stale/stopped status | Starts, stops, restarts; stale heartbeat refused as live | PASS | `daemon_lifecycle`; stopped status in browser |
| Service registry | CLI/TUI | Registered health affects findings | Repeated failed endpoint becomes a finding | PASS | `service_findings`; custom-service tests |
| Change sets / backup / runbook / alerting | CLI | Durable list/report/output contracts | Change list, backup report, alerting JSON work; rollback demo passes | PASS | CLI sweep; operations demo; memory tests |
| Remote registration / attach selection | CLI/TUI | Quoted remote command and correct target | Stub SSH and remote-target tests pass | PASS | `fleet`; TUI remote selection tests |
| Completion | CLI | Shell completion output | Bash completion emits successfully | PASS | Release CLI sweep |
| TUI keybindings | TUI | Commands, help, panels, editor, cancel, quit | PTY smoke visits help/palette/sidebar/history/clear/verbose/editor/cancel/quit; exit 0, no panic | PASS | Isolated release PTY, keymap tests |
| TUI assistant approvals | TUI | Run, approve, deny, cancel | Exact approval and denied-file invariants hold | PASS | TUI assistant tests, `agent_workflows` PTY scenarios |
| TUI plan recovery | TUI | Restarted rate-limit countdown retries; orphan loading ends | Restored deadline and stale timeout regressions pass | PASS | RED→GREEN TUI persistence/recovery tests |
| Shared plans | Web/TUI | Both frontends read shared persisted state | Web snapshot integration and fresh TUI restoration read canonical plan records | PASS | `snapshot_carries_ready_ai_plan`, TUI persisted status tests; no claim of simultaneous manual generation in both screens |
| Provider inventory/doctor against real backend | CLI | Actual provider health and tool call support | No live provider selected for this run | OUT OF SCOPE | Provider fixtures pass; real-provider acceptance requires an explicit model/backend |
| Remote agent installation/bootstrap/trust target | CLI/TUI | Deployment to an actual SSH host | No disposable SSH deployment target supplied | OUT OF SCOPE | Help/quoting/selection tested; installation into a real host not performed |
| TUI remote monitoring session | TUI | Attach to a working SSH monitoring target | Selection/offline/stale contracts tested; no live SSH session | OUT OF SCOPE | Requires real target; local stub evidence above |

## Defect ledger

| ID | Defect | Resolution / regression |
|---|---|---|
| B1 / Task 8 | Connection refusal described as unparseable plan | Additive `provider_unavailable` persisted state; web integration test |
| B2 / Task 9 | Enter overrides native controls; pairing allows dashboard shortcuts | Scope shortcuts; dashboard JS tests + browser Enter check |
| B3 / Task 9 | Snapshot replacement re-enables pending action and permits duplicate requests | Pending-operation Set independent of DOM; JS duplicate-request test |
| B4 / Task 9 | Plan shows “not generated” before first reply | Immediate loading state; JS regression |
| B5 / Task 10 | Restored rate-limit countdown never starts | Reconstruct deadline using persisted update time; TUI regression |
| B6 / Task 10 | Orphan loading row spins forever in TUI | Timeout after 120 seconds; TUI regression |
| B7 / Task 11 | Empty fleet `--json` emits no JSON | Emit `[]`; CLI integration regression |
| B8 / Task 12 | Init loops on closed input | EOF handling and bounded process regression; final result below |
| C1 | Queue grid shrinks content into overlapping rows | Natural grid row sizing; final browser content-fit check at 3 widths |
| B9 | `config set provider ollama` reported success and wrote a config every later command rejected | Write refused unless the result loads; `config_set_rejects_values_the_config_cannot_load` |
| B10 | `config get` on a fresh install printed an OS error | Reports the key as unset (default applies); `config_get_without_a_config_file_reports_unset` |
| B11 | 27 of 28 subcommands had no help description | Descriptions added; `every_subcommand_has_a_description` |
| B12 | Web: a plan row's `pending` placeholder read as a failed generation | `pending` reads as loading; `freshly_inserted_pending_plan_is_loading_not_failed` |
| C2 | Live findings showed `open n/a · seen n/a`; evidence summary was a bare number; status chips looked like buttons; queue below the fold on phones | See `web-ui-audit.md` final pass; `live_findings_carry_last_seen` |

No release tag, push, deployment, or fresh-VM certification is part of this continuation. Final gate and acceptance results are in [VERIFICATION.md](VERIFICATION.md).
