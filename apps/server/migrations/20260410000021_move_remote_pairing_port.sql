ALTER TABLE device_pairings
    ADD COLUMN remote_pairing_port INTEGER CHECK (remote_pairing_port BETWEEN 1 AND 65535),
    ADD COLUMN remote_pairing_port_expires_at TIMESTAMPTZ,
    ADD CHECK ((remote_pairing_port IS NULL) = (remote_pairing_port_expires_at IS NULL));

-- Preserve a live report for existing pairings. New pairings wait for a fresh report.
UPDATE device_pairings p
SET remote_pairing_port = d.remote_pairing_port,
    remote_pairing_port_expires_at = LEAST(d.remote_pairing_port_expires_at, d.expires_at)
FROM devices d
WHERE d.id = p.device_id AND d.expires_at > statement_timestamp()
  AND d.remote_pairing_port IS NOT NULL
  AND d.remote_pairing_port_expires_at > statement_timestamp();

ALTER TABLE devices
    DROP COLUMN remote_pairing_port,
    DROP COLUMN remote_pairing_port_expires_at;
