-- Minimal browser-visible metadata, bound to the pairing saved by this attempt.
-- The remote-pairing identifier is provisionally used as the provisioning UDID.
ALTER TABLE public_pairing_attempts
    ADD COLUMN paired_udid TEXT,
    ADD COLUMN paired_name TEXT,
    ADD COLUMN paired_model TEXT,
    ADD COLUMN pairing_started_at TIMESTAMPTZ;
