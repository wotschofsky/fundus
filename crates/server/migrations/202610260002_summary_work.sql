-- Regenerating the summary is queued work like speaker reanalysis. SQLite cannot change a CHECK,
-- so the column is recreated with work already queued kept.
ALTER TABLE entries ADD COLUMN work TEXT CHECK(work IN ('speakers','summary'));
UPDATE entries SET work=requested_work;
ALTER TABLE entries DROP COLUMN requested_work;
ALTER TABLE entries RENAME COLUMN work TO requested_work;
