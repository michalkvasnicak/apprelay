ALTER TABLE public_pairing_attempts
    DROP CONSTRAINT public_pairing_attempts_status_check;

UPDATE public_pairing_attempts SET status = 'success' WHERE status = 'unverified';

ALTER TABLE public_pairing_attempts
    ADD CONSTRAINT public_pairing_attempts_status_check
    CHECK (status IN ('pending', 'pairing', 'success', 'failed', 'expired'));
