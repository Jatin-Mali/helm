---
phase: 03-m3-fleet-multi-host
plan: 05
subsystem: helm-memory
tags:
  - database
  - migration
  - snapshots
  - multi-host
dependency_graph:
  requires:
    - "03-01"
  provides:
    - "host_id column in snapshots table"
    - "SnapshotRecord.host_id field"
    - "host_id extraction and storage in insert()"
  affects:
    - snapshot keying in multi-host scenarios
tech_stack:
  added:
    - SQLite ALTER TABLE migration
  patterns:
    - Row mapping with column indices
    - JSON extraction (val["host"]["id"])
    - Unit tests with in-memory SQLite
key_files:
  created:
    - crates/helm-memory/migrations/0011_host_id_snapshots.sql
  modified:
    - crates/helm-memory/src/snapshots.rs
decisions:
  - Host ID extracted from JSON at val["host"]["id"] (trusted source)
  - Default empty string for backward compatibility
  - Index on host_id for efficient queries
  - Tests cover roundtrip and multi-host scenarios
metrics:
  duration: 15 minutes
  tasks_completed: 6/6
  test_count: 73 (helm-memory)
  files_modified: 2
  commits: 1 (d2f6557)
completion_date: 2026-05-18T00:00:00Z

---

# Phase 03 Plan 05: Add host_id Column to Snapshots — SUMMARY

DB migration 0011_host_id_snapshots.sql: adds host_id TEXT column to snapshots table for multi-host snapshot keying.

## Completed Tasks

| # | Task | Status | Commit |
|---|------|--------|--------|
| 1 | Create migration 0011_host_id_snapshots.sql | ✓ | d2f6557 |
| 2 | Add host_id field to SnapshotRecord | ✓ | d2f6557 |
| 3 | Update insert() to extract/store host_id | ✓ | d2f6557 |
| 4 | Update query functions (latest, get, list, latest_except) | ✓ | d2f6557 |
| 5 | Add unit tests for host_id roundtrip | ✓ | d2f6557 |
| 6 | Full suite verification (fmt, clippy, test) | ✓ | d2f6557 |

## Implementation Details

### Migration 0011_host_id_snapshots.sql
- Adds `host_id TEXT DEFAULT ''` column to snapshots table
- Creates `idx_snapshots_host_id` index for efficient lookups
- Idempotent: `CREATE INDEX IF NOT EXISTS` prevents errors on re-run
- Backward compatible: existing rows receive empty string default

### SnapshotRecord Struct
- New field: `pub host_id: String` inserted after `host_hostname`
- Preserves existing field order for other row.get(N) mappings

### insert() Function
- Extracts `host_id` from JSON: `val["host"]["id"].as_str().unwrap_or("")`
- Updates INSERT statement to include host_id column (param ?3)
- Updates params![] list to include host_id in correct position

### Query Functions Updated
All 4 functions updated to include host_id in SELECT and row mapping:
- `latest()`: SELECT includes host_id, row.get(2) for host_id
- `get()`: SELECT includes host_id, row.get(2) for host_id
- `list()`: SELECT includes host_id, row.get(2) for host_id
- `latest_except()`: SELECT includes host_id, row.get(2) for host_id

Column indices adjusted: host_id at position 2, all subsequent columns shifted right.

### Unit Tests
Two test functions added to #[cfg(test)] module:

1. **test_host_id_roundtrip()**
   - Creates in-memory SQLite DB with schema
   - Inserts snapshot with host_id="uuid-12345"
   - Retrieves via get() and asserts host_id matches

2. **test_multiple_hosts_snapshots()**
   - Inserts two snapshots with host_id="uuid-1" and host_id="uuid-2"
   - Retrieves both by ID, validates correct host_id for each
   - Calls list() to verify ordering and host_id population

## Verification Results

```
cargo fmt --check      → OK (no formatting issues)
cargo clippy ...       → OK (no warnings/errors)
cargo test -p helm-memory --lib → PASSED (73 tests, including 2 new)
```

## Test Coverage

- Roundtrip test validates insert→retrieve consistency
- Multi-host test validates independent host_id isolation
- List query test validates correct ordering with new column
- All existing tests continue to pass (no regressions)

## Deviations from Plan

None — plan executed exactly as written. All acceptance criteria met.

## Threat Flags

No new threat surface introduced:
- host_id extracted from trusted JSON input (same trust model as existing fields)
- DB is local SQLite with standard file permissions
- No user input flows into host_id

## Known Stubs

None — plan complete with no pending functionality.

## Self-Check: PASSED

- ✓ Migration file 0011_host_id_snapshots.sql exists
- ✓ Contains ALTER TABLE ADD COLUMN host_id TEXT DEFAULT ''
- ✓ Contains CREATE INDEX IF NOT EXISTS idx_snapshots_host_id
- ✓ SnapshotRecord struct includes pub host_id: String field
- ✓ insert() function extracts and stores host_id
- ✓ All query functions (latest, get, list, latest_except) updated with host_id
- ✓ Unit tests test_host_id_roundtrip and test_multiple_hosts_snapshots exist
- ✓ All helm-memory tests pass (73 total)
- ✓ Commit d2f6557 records all changes
