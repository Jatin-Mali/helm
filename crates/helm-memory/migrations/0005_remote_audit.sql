ALTER TABLE audit_events ADD COLUMN target TEXT;
CREATE INDEX IF NOT EXISTS idx_audit_events_target ON audit_events(target, id);
