CREATE TABLE IF NOT EXISTS finding_states (
    fingerprint TEXT PRIMARY KEY,
    status TEXT NOT NULL DEFAULT 'open',
    suppression_reason TEXT NOT NULL DEFAULT '',
    note TEXT NOT NULL DEFAULT '',
    updated_at INTEGER NOT NULL,
    snapshot_id TEXT NOT NULL DEFAULT '',
    finding_id TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_finding_states_status
    ON finding_states(status);
CREATE INDEX IF NOT EXISTS idx_finding_states_updated_at
    ON finding_states(updated_at DESC);
