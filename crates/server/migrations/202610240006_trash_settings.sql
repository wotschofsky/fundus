ALTER TABLE entries ADD COLUMN trashed_at TEXT;
CREATE INDEX entries_trash ON entries(trashed_at);
CREATE TABLE app_settings (key TEXT PRIMARY KEY, value_json TEXT NOT NULL);
