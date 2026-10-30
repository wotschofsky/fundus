ALTER TABLE credentials ADD COLUMN last_used_at TEXT;
ALTER TABLE oauth_grants ADD COLUMN last_used_at TEXT;
