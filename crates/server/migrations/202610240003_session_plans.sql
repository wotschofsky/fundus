ALTER TABLE imports ADD COLUMN session_plan TEXT;
ALTER TABLE uploads ADD COLUMN source_id TEXT;
CREATE UNIQUE INDEX uploads_import_source ON uploads(import_id,source_id) WHERE source_id IS NOT NULL;
ALTER TABLE entries ADD COLUMN trim_start_frame INTEGER;
ALTER TABLE entries ADD COLUMN trim_end_frame INTEGER;
CREATE TABLE import_entries (
 import_id TEXT NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
 entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
 session_index INTEGER NOT NULL,
 PRIMARY KEY(import_id,session_index)
);
CREATE INDEX import_entries_entry ON import_entries(entry_id);
