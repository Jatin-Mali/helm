---
phase: 03-m3-fleet-multi-host
plan: 01
subsystem: helm-cli/remote
tags: [credential, ssh, fleet-foundation, m3-slice-3.1]
dependencies:
  requires: []
  provides:
    - Credential enum supporting SshAgent and KeyFile variants
    - RemoteEntry.credential field with serde(default) for backward compatibility
    - ssh_argv() extension injecting -i flag for KeyFile credentials
  affects:
    - helm-cli/src/remote.rs (primary change set)
    - helm-cli/src/bootstrap.rs (updated RemoteEntry construction)
    - helm-cli/src/main.rs (updated RemoteEntry construction in add command)
tech_stack:
  added:
    - serde with #[default] attribute for enum variants
    - PathBuf for SSH key file paths
  patterns:
    - Enum-based credential abstraction layer
    - TOML-serializable configuration with sensible defaults
key_files:
  created: []
  modified:
    - helm-cli/src/remote.rs (Credential enum, RemoteEntry field, ssh_argv extension, unit tests)
    - helm-cli/src/bootstrap.rs (to_remote_entry method)
    - helm-cli/src/main.rs (RemoteCommand::Add handler)
completed_date: 2026-05-18
duration_minutes: 15
completed_tasks: 4
---

# Phase 03 Plan 01: Credential Abstraction in RemoteEntry — SUMMARY

**Objective:** Add credential abstraction to RemoteEntry, enabling per-host SSH key file configuration as foundation for Slice 3.1 of M3 (Fleet Multi-Host).

**One-liner:** Credential enum with SshAgent/KeyFile variants stored in RemoteEntry; ssh_argv() injects -i flag for KeyFile; unit tests validate serialization and argv construction; backward compatible via serde(default).

---

## What Was Built

### Credential Enum
- **Location:** `helm-cli/src/remote.rs:20-24`
- **Variants:** `SshAgent` (unit), `KeyFile(PathBuf)` (file path)
- **Derives:** Debug, Clone, Default, Serialize, Deserialize
- **Default:** `SshAgent` (backward compatible)

### RemoteEntry Extension
- **Location:** `helm-cli/src/remote.rs:33-46`
- **New field:** `pub credential: Credential` with `#[serde(default)]`
- **Effect:** Existing remotes.toml entries without credential field automatically default to SshAgent

### ssh_argv() Enhancement
- **Location:** `helm-cli/src/remote.rs:111-135`
- **Logic:** If `Credential::KeyFile(path)`, inserts `-i <path>` into argv before the hostname
- **Ordering:** `ssh [opts] [-i /path/to/key] -u user -o BatchMode=yes -o ConnectTimeout=10 hostname`
- **Backward compatible:** SshAgent path unchanged

### Unit Tests (3 new, 2 updated)
- **Location:** `helm-cli/src/remote.rs:223-287`

**New tests:**
1. `test_ssh_argv_with_keyfile_injects_identity_flag()` — Verifies -i flag and path in argv, correct ordering
2. `test_credential_default_is_ssh_agent()` — Confirms Credential::default() == SshAgent
3. `test_credential_serialization_roundtrip()` — TOML round-trip preserves KeyFile path

**Updated tests:**
1. `registry_upsert_replaces_existing_entry()` — Added credential field
2. `remote_entry_ssh_argv_includes_port_user_and_opts()` — Added credential field

---

## Verification Results

| Check | Result | Details |
|-------|--------|---------|
| Compilation | PASS | `cargo build` completes clean |
| Unit Tests | PASS | 320 tests passed (6 test suites, 1.75s) |
| Format Check | PASS | `cargo fmt --check` no diffs |
| Clippy Lint | PASS | `cargo clippy -- -D warnings` zero errors |
| Backward Compatibility | PASS | Existing remotes.toml entries auto-default to SshAgent |

---

## Deviations from Plan

None — plan executed exactly as written.

---

## Key Changes Summary

| File | Change | Lines |
|------|--------|-------|
| `helm-cli/src/remote.rs` | Credential enum + RemoteEntry field + ssh_argv extension + 3 new tests | +110 |
| `helm-cli/src/bootstrap.rs` | to_remote_entry() updated with credential: Default::default() | +1 |
| `helm-cli/src/main.rs` | RemoteCommand::Add handler updated with credential: Default::default() | +1 |

---

## Threat Surface Assessment

**New threat surface introduced by Credential enum:**
- SSH key file path now specified per-host in remotes.toml (was globally in ssh_opts before)
- Path stored as plaintext in TOML file (same risk as ssh_opts previously)

**Mitigations in place:**
- No debug output of key paths (PathBuf serialized only for SSH command construction)
- File permissions: remotes.toml stored in config dir with 0700 parent, 0600 file mode (per security policy)
- SSH args properly split via Vec<String> — no shell injection possible
- Credential enum prevents password storage in M3 (Password variant locked out by D-M3-1)

**No new threat flags:** Uses existing trust boundary (file system, local SSH integration).

---

## Acceptance Criteria Met

- [x] Credential enum exists with SshAgent and KeyFile(PathBuf) variants
- [x] RemoteEntry struct has credential field with #[serde(default)] attribute
- [x] Default for Credential returns SshAgent
- [x] Crate compiles without errors
- [x] ssh_argv() compiles without errors
- [x] When credential is KeyFile(path), argv includes -i and the path
- [x] When credential is SshAgent, argv does not include -i
- [x] Unit test ssh_argv_with_keyfile_injects_identity_flag passes
- [x] All three unit tests exist and pass (serialization, default, ssh_argv integration)
- [x] Test names clearly indicate what is being verified
- [x] Tests cover both SshAgent and KeyFile paths
- [x] cargo fmt passes (no formatting issues)
- [x] cargo clippy reports 0 warnings/errors
- [x] cargo test --workspace passes (all tests green)
- [x] Backward compatible: existing remotes.toml entries default to SshAgent

---

## Next Steps (M3 Slices 3.2–3.5)

- **Slice 3.2:** Parallel SSH JoinSet + findings collection via `helm monitor --format json`
- **Slice 3.3:** DashboardData fleet integration + background refresh
- **Slice 3.4:** Fleet tab in TUI with per-host summary table
- **Slice 3.5:** DB migration 0011_host_id_snapshots.sql for multi-host snapshot tracking

---

## Commit Metadata

| Field | Value |
|-------|-------|
| Commit Hash | 39f3b18 |
| Message | feat(03-m3-fleet-multi-host): add Credential enum and RemoteEntry field for SSH key file support |
| Author | Claude Sonnet 4.6 |
| Timestamp | 2026-05-18T04:30:00Z (est.) |
| Files Modified | 3 (remote.rs, bootstrap.rs, main.rs) |
