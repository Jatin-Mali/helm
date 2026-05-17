-- Add host_id column to snapshots table for multi-host keying
ALTER TABLE snapshots ADD COLUMN host_id TEXT DEFAULT '';
CREATE INDEX IF NOT EXISTS idx_snapshots_host_id ON snapshots(host_id);
