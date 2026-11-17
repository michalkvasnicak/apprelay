CREATE TABLE ipa_artifacts (
    id uuid PRIMARY KEY,
    token_hash bytea NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    object_key text NOT NULL UNIQUE,
    file_name text NOT NULL CHECK (char_length(file_name) BETWEEN 5 AND 200),
    size_bytes bigint NOT NULL CHECK (size_bytes BETWEEN 1 AND 104857600),
    sha256 text NOT NULL CHECK (char_length(sha256) = 44),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    expires_at timestamptz NOT NULL DEFAULT (clock_timestamp() + interval '7 days'),
    completed_at timestamptz
);

CREATE INDEX ipa_artifacts_expiry_idx ON ipa_artifacts (expires_at);
