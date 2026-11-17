-- Preserve existing deadlines; new uploads supply their configured deadline or NULL.
ALTER TABLE ipa_artifacts
    ALTER COLUMN expires_at DROP NOT NULL,
    ALTER COLUMN expires_at DROP DEFAULT;
