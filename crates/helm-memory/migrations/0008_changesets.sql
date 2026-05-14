CREATE TABLE IF NOT EXISTS troubleshooting_plans (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    snapshot_id TEXT NOT NULL,
    hypotheses_json TEXT NOT NULL DEFAULT '[]',
    read_only_steps_json TEXT NOT NULL DEFAULT '[]',
    proposed_fix_steps_json TEXT NOT NULL DEFAULT '[]',
    approval_required INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    verdict_summary TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS change_sets (
    id TEXT PRIMARY KEY,
    plan_id TEXT NOT NULL,
    plan_title TEXT NOT NULL DEFAULT '',
    snapshot_id TEXT NOT NULL,
    before_snapshot_id TEXT NOT NULL,
    after_snapshot_id TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at INTEGER NOT NULL,
    approved_at INTEGER,
    rejected_at INTEGER,
    completed_at INTEGER,
    rolled_back_at INTEGER,
    rollback_snapshot_id TEXT,
    summary TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS change_set_steps (
    id TEXT PRIMARY KEY,
    change_set_id TEXT NOT NULL,
    plan_step_title TEXT NOT NULL,
    tool TEXT NOT NULL,
    input_json TEXT NOT NULL DEFAULT '{}',
    command_text TEXT,
    expected_effect TEXT NOT NULL DEFAULT '',
    risk TEXT NOT NULL DEFAULT 'none',
    status TEXT NOT NULL DEFAULT 'pending',
    output_text TEXT NOT NULL DEFAULT '',
    error_text TEXT NOT NULL DEFAULT '',
    verification_result TEXT NOT NULL DEFAULT '',
    started_at INTEGER,
    completed_at INTEGER,
    FOREIGN KEY (change_set_id) REFERENCES change_sets(id)
);

CREATE TABLE IF NOT EXISTS change_set_backups (
    id TEXT PRIMARY KEY,
    change_set_id TEXT NOT NULL,
    step_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    checksum_before TEXT NOT NULL,
    backup_content TEXT NOT NULL,
    restored INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (change_set_id) REFERENCES change_sets(id)
);

CREATE INDEX IF NOT EXISTS idx_change_set_steps_cs ON change_set_steps(change_set_id);
CREATE INDEX IF NOT EXISTS idx_change_set_backups_cs ON change_set_backups(change_set_id);
