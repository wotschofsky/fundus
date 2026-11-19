-- Static upload and read tokens are gone: apps sign in with OAuth, browsers with a session.
-- Every token row is deleted; only password-mode browser sessions stay. Nothing reads when a
-- session was last used, so that column goes too (OAuth connections keep their own).
DELETE FROM credentials WHERE scope != 'session';
ALTER TABLE credentials DROP COLUMN last_used_at;
