CREATE TABLE device_pairings (
    device_id UUID PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE,
    format_version SMALLINT NOT NULL CHECK (format_version = 1),
    nonce BYTEA NOT NULL CHECK (octet_length(nonce) = 12),
    ciphertext BYTEA NOT NULL CHECK (octet_length(ciphertext) >= 16),
    attempt_started_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Pair-setup alone does not confirm iOS completion.
    verified_at TIMESTAMPTZ
);
