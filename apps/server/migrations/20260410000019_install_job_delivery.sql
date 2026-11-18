ALTER TABLE install_jobs
    ADD COLUMN dispatched_at TIMESTAMPTZ,
    ADD COLUMN finished_at TIMESTAMPTZ,
    ADD COLUMN outcome TEXT CHECK (outcome IN ('installed', 'failed'));

CREATE INDEX install_jobs_outbox ON install_jobs (queued_at)
    WHERE queued_at IS NOT NULL AND dispatched_at IS NULL AND finished_at IS NULL;
