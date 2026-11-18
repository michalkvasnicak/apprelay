ALTER TABLE public_pairing_attempts ADD COLUMN worker_started_at TIMESTAMPTZ;

ALTER TABLE pairing_attempts ADD COLUMN public_attempt_id UUID
    REFERENCES public_pairing_attempts(id) ON DELETE SET NULL;
