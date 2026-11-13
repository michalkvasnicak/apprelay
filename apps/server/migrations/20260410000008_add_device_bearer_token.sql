ALTER TABLE devices ADD COLUMN bearer_token_hash bytea
    CHECK (octet_length(bearer_token_hash) = 32);
