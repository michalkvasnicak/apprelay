-- Existing identity-only registrations receive an address on their next request.
-- Expired and pending registrations retain their addresses until peer removal.
ALTER TABLE devices ADD COLUMN address INET UNIQUE
    CHECK (family(address) = 4 AND masklen(address) = 32);
