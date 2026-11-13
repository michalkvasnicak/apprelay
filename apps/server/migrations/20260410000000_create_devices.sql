CREATE TABLE devices (
    id UUID PRIMARY KEY,
    wireguard_public_key BYTEA NOT NULL UNIQUE
        CHECK (octet_length(wireguard_public_key) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
