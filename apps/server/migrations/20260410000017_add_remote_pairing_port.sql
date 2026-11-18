ALTER TABLE devices
    ADD COLUMN remote_pairing_port INTEGER
        CHECK (remote_pairing_port BETWEEN 1 AND 65535),
    ADD COLUMN remote_pairing_port_expires_at TIMESTAMPTZ;
