ALTER TABLE pairing_attempts
    ADD COLUMN started_notification_queued_at TIMESTAMPTZ,
    ADD COLUMN started_notification_sent_at TIMESTAMPTZ;
