CREATE TABLE IF NOT EXISTS snapshots (
    id TEXT PRIMARY KEY,
    host_hostname TEXT NOT NULL DEFAULT 'unknown',
    host_kernel_name TEXT NOT NULL DEFAULT 'unknown',
    host_kernel_release TEXT NOT NULL DEFAULT 'unknown',
    host_machine TEXT NOT NULL DEFAULT 'unknown',
    host_os_pretty_name TEXT,
    host_os_id TEXT,
    host_os_version_id TEXT,
    host_uptime_seconds INTEGER NOT NULL DEFAULT 0,
    collected_at INTEGER NOT NULL,
    profile TEXT NOT NULL DEFAULT 'standard',
    domains_json TEXT NOT NULL DEFAULT '{}',
    collector_errors_json TEXT NOT NULL DEFAULT '[]',
    redaction_version TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_snapshots_collected_at ON snapshots(collected_at DESC);
CREATE INDEX IF NOT EXISTS idx_snapshots_profile ON snapshots(profile);
