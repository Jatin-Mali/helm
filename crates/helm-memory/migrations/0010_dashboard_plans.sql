ALTER TABLE troubleshooting_plans ADD COLUMN finding_id TEXT NOT NULL DEFAULT '';
ALTER TABLE troubleshooting_plans ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE troubleshooting_plans ADD COLUMN dashboard_plan_status TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE troubleshooting_plans ADD COLUMN generation_error TEXT NOT NULL DEFAULT '';
ALTER TABLE troubleshooting_plans ADD COLUMN narrative_summary TEXT NOT NULL DEFAULT '';
ALTER TABLE troubleshooting_plans ADD COLUMN verification_steps_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE troubleshooting_plans ADD COLUMN reproduction_steps_json TEXT NOT NULL DEFAULT '[]';

CREATE INDEX IF NOT EXISTS idx_troubleshooting_plans_finding_updated
    ON troubleshooting_plans(finding_id, updated_at DESC, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_troubleshooting_plans_status_updated
    ON troubleshooting_plans(dashboard_plan_status, updated_at DESC, created_at DESC);
