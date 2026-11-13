ALTER TABLE pairing_attempts
    ADD COLUMN success_notification_id UUID UNIQUE,
    ADD COLUMN success_notification_expires_at TIMESTAMPTZ,
    ADD COLUMN success_notification_queued_at TIMESTAMPTZ,
    ADD COLUMN success_notification_sent_at TIMESTAMPTZ;
