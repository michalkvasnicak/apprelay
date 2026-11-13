-- Legacy identity-only registrations must be assigned valid, unused VPN
-- addresses before this migration. Do not silently delete them or guess a subnet.
ALTER TABLE devices ALTER COLUMN address SET NOT NULL;
