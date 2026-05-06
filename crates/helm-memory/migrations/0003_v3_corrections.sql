-- v3: track correction attempts and response format recovery per episode.
ALTER TABLE episodes ADD COLUMN corrections_used INTEGER NOT NULL DEFAULT 0;
ALTER TABLE episodes ADD COLUMN format_recovery_used INTEGER NOT NULL DEFAULT 0;
ALTER TABLE episodes ADD COLUMN response_format_log TEXT;
