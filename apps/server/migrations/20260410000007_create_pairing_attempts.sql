CREATE TABLE pairing_attempts (
    id UUID PRIMARY KEY,
    device_id UUID NOT NULL UNIQUE REFERENCES devices(id) ON DELETE CASCADE,
    device_expires_at TIMESTAMPTZ NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    ended_at TIMESTAMPTZ,
    code_id UUID UNIQUE,
    nonce BYTEA,
    ciphertext BYTEA,
    notification_queued_at TIMESTAMPTZ,
    notification_sent_at TIMESTAMPTZ,
    CHECK (
        (code_id IS NULL AND nonce IS NULL AND ciphertext IS NULL)
        OR
        (code_id IS NOT NULL AND nonce IS NOT NULL AND octet_length(nonce) = 12
         AND ciphertext IS NOT NULL AND octet_length(ciphertext) = 22)
    )
);
CREATE INDEX pairing_attempts_expiry ON pairing_attempts (expires_at);
