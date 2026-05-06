-- v4: capability grants and hash-chained audit events.
CREATE TABLE IF NOT EXISTS capability_grants (
    id TEXT PRIMARY KEY,
    capability TEXT NOT NULL,
    scope TEXT NOT NULL,
    granted_at INTEGER NOT NULL,
    expires_at INTEGER,
    revoked_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_capability_grants_capability
    ON capability_grants(capability, revoked_at, expires_at);

CREATE TABLE IF NOT EXISTS audit_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    episode_id TEXT,
    timestamp INTEGER NOT NULL,
    tool_name TEXT NOT NULL,
    input_hash TEXT NOT NULL,
    output_hash TEXT NOT NULL,
    capability TEXT NOT NULL,
    taint TEXT NOT NULL,
    cwd TEXT NOT NULL,
    decision TEXT NOT NULL,
    previous_hash TEXT NOT NULL,
    event_hash TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_events_episode
    ON audit_events(episode_id, id);
