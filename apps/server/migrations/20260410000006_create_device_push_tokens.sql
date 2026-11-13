CREATE TABLE device_push_tokens (
    device_id UUID PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE,
    token BYTEA NOT NULL CHECK (octet_length(token) > 0),
    environment TEXT NOT NULL CHECK (environment IN ('sandbox', 'production')),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
