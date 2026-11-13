ALTER TABLE pairing_attempts
    ADD COLUMN timeout_notification_id UUID UNIQUE,
    ADD COLUMN timeout_notification_queued_at TIMESTAMPTZ,
    ADD COLUMN timeout_notification_sent_at TIMESTAMPTZ;
