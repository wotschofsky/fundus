-- Review drafts never change a transcript. They expire and are invalidated by a changed snapshot.
CREATE TABLE speaker_correction_reviews (
 id TEXT PRIMARY KEY,
 entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
 snapshot TEXT NOT NULL,
 proposal TEXT NOT NULL,
 expires_at INTEGER NOT NULL
);
CREATE INDEX speaker_correction_reviews_entry ON speaker_correction_reviews(entry_id);
-- Accepted passage decisions survive speaker reanalysis and are never used as automatic matches.
CREATE TABLE speaker_corrections (
 segment_id TEXT PRIMARY KEY REFERENCES segments(id) ON DELETE CASCADE,
 entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
 speaker TEXT NOT NULL,
 created_at TEXT NOT NULL
);
CREATE INDEX speaker_corrections_entry ON speaker_corrections(entry_id);
