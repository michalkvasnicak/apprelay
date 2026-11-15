-- Web capabilities are independent of the per-device PIN/outbox row.
CREATE TABLE public_pairing_attempts (
    id UUID PRIMARY KEY,
    token_hash BYTEA NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    -- Keep token status until retention cleanup even after registration deletion.
    device_id UUID,
    device_expires_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'pairing', 'unverified', 'failed', 'expired')),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT (clock_timestamp() + INTERVAL '10 minutes'),
    queued_at TIMESTAMPTZ,
    reason TEXT CHECK (reason IN ('pairing_failed', 'timeout')),
    CHECK ((device_id IS NULL) = (device_expires_at IS NULL))
);
CREATE UNIQUE INDEX public_pairing_attempts_active_device
    ON public_pairing_attempts (device_id) WHERE status = 'pairing';
CREATE INDEX public_pairing_attempts_expiry ON public_pairing_attempts (expires_at);
