-- A recording shared by link (`share.rs`): anyone holding the token reads it until it expires
-- (NULL: never) or the owner stops sharing. The token itself is kept, so the owner can copy the
-- link again; moving the recording to Trash deletes the row.
CREATE TABLE shares (
  entry_id TEXT PRIMARY KEY REFERENCES entries(id) ON DELETE CASCADE,
  token TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL,
  expires_at TEXT
);
