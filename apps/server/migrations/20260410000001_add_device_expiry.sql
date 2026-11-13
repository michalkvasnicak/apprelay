ALTER TABLE devices ADD COLUMN expires_at TIMESTAMPTZ;

UPDATE devices SET expires_at = created_at + INTERVAL '7 days';

ALTER TABLE devices ALTER COLUMN expires_at SET NOT NULL;
