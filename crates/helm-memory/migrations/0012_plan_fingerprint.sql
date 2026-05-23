ALTER TABLE troubleshooting_plans ADD COLUMN finding_fingerprint TEXT NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS idx_troubleshooting_plans_fingerprint_updated
    ON troubleshooting_plans(finding_fingerprint, updated_at DESC, created_at DESC);
