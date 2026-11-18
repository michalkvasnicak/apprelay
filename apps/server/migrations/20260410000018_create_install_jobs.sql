-- Keep these rows independent of device/attempt deletion so the cleanup task
-- still has the R2 key after a pairing is gone.
CREATE TABLE install_jobs (
    id UUID PRIMARY KEY,
    public_attempt_id UUID NOT NULL,
    device_id UUID NOT NULL,
    device_expires_at TIMESTAMPTZ NOT NULL,
    pairing_started_at TIMESTAMPTZ NOT NULL,
    size_bytes BIGINT NOT NULL CHECK (size_bytes BETWEEN 1 AND 104857600),
    sha256 TEXT NOT NULL CHECK (length(sha256) = 44),
    -- Start deleting one hour before the external 24-hour retention target.
    expires_at TIMESTAMPTZ NOT NULL DEFAULT (clock_timestamp() + INTERVAL '23 hours'),
    queued_at TIMESTAMPTZ
);
CREATE INDEX install_jobs_expiry ON install_jobs (expires_at);
CREATE INDEX install_jobs_queue ON install_jobs (queued_at) WHERE queued_at IS NOT NULL;
