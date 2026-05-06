PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS episodes (
    id TEXT PRIMARY KEY,
    goal TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    outcome TEXT,
    iterations INTEGER NOT NULL DEFAULT 0,
    tokens_in INTEGER NOT NULL DEFAULT 0,
    tokens_out INTEGER NOT NULL DEFAULT 0,
    final_message TEXT,
    error TEXT
);

CREATE TABLE IF NOT EXISTS episode_steps (
    episode_id TEXT NOT NULL REFERENCES episodes(id) ON DELETE CASCADE,
    step_index INTEGER NOT NULL,
    role TEXT NOT NULL,
    content_json TEXT NOT NULL,
    tokens_in INTEGER NOT NULL DEFAULT 0,
    tokens_out INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (episode_id, step_index)
);

CREATE INDEX idx_episodes_started_at ON episodes(started_at DESC);
CREATE INDEX idx_episodes_outcome ON episodes(outcome);
