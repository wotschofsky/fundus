-- Notes requests carry a generation so an Undo can supersede an in-flight summary safely.
ALTER TABLE entries ADD COLUMN notes_revision INTEGER NOT NULL DEFAULT 0;
-- One-level Undo for the latest speaker correction in each recording, durable across reloads.
CREATE TABLE speaker_correction_history (
 id TEXT PRIMARY KEY,
 entry_id TEXT NOT NULL UNIQUE REFERENCES entries(id) ON DELETE CASCADE,
 before_json TEXT NOT NULL,
 after_snapshot TEXT NOT NULL,
 revision INTEGER NOT NULL,
 changed INTEGER NOT NULL,
 summary_refresh INTEGER NOT NULL,
 created_at TEXT NOT NULL
);
