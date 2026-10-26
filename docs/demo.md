# Verification and demonstration guide

Everything here is reproducible with one command. Nothing touches your real data:
every scenario uses temporary XDG state, a temporary project and loopback services.

```sh
cargo build --release -p helm-cli
HELM_DEMO_PROVIDER=ollama HELM_DEMO_MODEL=gemma4:31b-cloud \
  bash scripts/verify-demo.sh --strict --soak-minutes 3
```

`verify-demo.sh` prints `PASS`, `UNVERIFIED` (integration unavailable, e.g. no Docker or
no provider) or `FAIL` per capability. `--strict` turns `UNVERIFIED` into a failure.
Env: `HELM_BIN`, `HELM_BASELINE_BIN` (older build for the upgrade check),
`HELM_DEMO_PROVIDER`, `HELM_DEMO_MODEL`.

## Capability matrix

| Capability | How it is verified | Result |
|---|---|---|
| Agent read/edit/test loop, approvals, cancel, resume without replay | `helm-cli/tests/agent_workflows.rs`; real provider via `tests/demo/drive_agent.py` | PASS |
| Agent security (tamper, single-use approval, denial, origin allow-list, real headless browser) | `crates/helm-agent/tests/agent_security.rs` (`HELM_REQUIRE_BROWSER=1`) | PASS |
| Prompt injection in a workspace file | real provider; no approval requested, no file created, target untouched | PASS |
| Same plan from CLI / web / TUI: denied, read-only, dry-run, sandbox, approved, failed verification; `audit verify` after each | `helm-cli/tests/cross_frontend.rs` | PASS |
| Dashboard authentication (pairing code, session cookie, Host/Origin/CSRF, single-use reviews) | `helm-cli/tests/` web tests | PASS |
| Installer (checksum mismatch / missing / wrong OS / arch / success) | `scripts/test-install.sh` | PASS |
| Operations: finding, explain, deny/approve plan, HTTP 500→200, durable change set, rollback, rollback refuses to clobber a newer edit, failed collectors not reported healthy | `tests/demo/ops.py` | PASS |
| Database upgrade, backup, restore, corrupt-DB rejection from an older build | `tests/demo/upgrade.py` | PASS |
| Fleet: two Docker SSH hosts with distinct identities, stopped host is UNKNOWN/STALE with last-known identity, no local substitution | `tests/demo/fleet.sh`, `fleet.py` | PASS |
| Prolonged monitoring (daemon, memory, DB growth, clean stop) | `tests/demo/soak.py` | PASS (3 min) |

## Real-provider evidence

Provider: Ollama `gemma4:31b-cloud`, driven through a pty that answers each approval
prompt from an explicit allow-list.

- Fix a bug: inspect → approved edit of `calc.py` → failing check → fix → tests pass → summary citing evidence.
- Injected instruction inside a file: ignored; zero approvals requested.
- Every approval denied: run ends `failed` at the step limit, file unchanged.
- SIGINT: run ends `cancelled`, session saved.

## Soak and growth (3 min, 5 s interval; 4 cores, ~13.8 GB RAM)

| Metric | Value |
|---|---|
| One standard collection | 0.74 s |
| Daemon RSS | 29.5 → 32.8 MB |
| DB size | 5.3 → 11.0 MB over 34 snapshots (~0.197 MB/snapshot) |
| Projected DB at the 1000-snapshot cap | ~197 MB |

At the default 60 s refresh the 1000-snapshot cap is reached in ~16.7 h, so retention is
bounded by row count, not by the 7-day age limit. Plan about 200 MB of disk for the
database. Three minutes does not prove multi-day stability; run `--soak-minutes 30` or
longer on target hardware before relying on it.

## What is NOT verified

- **A fresh VM.** No qemu/vagrant/multipass here. Docker containers plus isolated XDG
  state stand in; the host's package state and init system were not exercised from a clean image.
- **Root-level `systemctl` on a real service.** TUI/web service stop/restart is tested
  against a fake `systemctl`; it was not run against a live unit.
- **Other providers.** Only Ollama `gemma4:31b-cloud` was run live. Groq and Gemini keys
  were rejected as invalid; OpenRouter and DeepSeek returned 402 (no credit). Their request
  paths are covered by scripted-server tests only.
- **Power-loss / `kill -9` mid-execution** on real hardware. Failure injection is in
  unit tests; no physical interruption test was run.
- **Soak beyond 3 minutes**, and any non-Linux platform.
- The baseline build (`fbbbec3`) cannot verify its own apply-plan audit rows
  ("unknown capability: read"). This build reads and verifies them; the baseline defect
  is not patched retroactively.

## Cleanup

```sh
docker rmi helm-demo-sshd 2>/dev/null; git worktree prune
```
