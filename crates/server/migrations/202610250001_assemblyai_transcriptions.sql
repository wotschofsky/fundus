-- One AssemblyAI job per entry. Legacy OpenAI rows in `parts` are kept but no longer read.
-- 'submitted' without remote_id: the create request may have reached AssemblyAI (explicit retry).
CREATE TABLE transcriptions (
 entry_id TEXT PRIMARY KEY REFERENCES entries(id) ON DELETE CASCADE,
 state TEXT NOT NULL CHECK(state IN ('submitted','processing','complete')),
 remote_id TEXT,
 utterances TEXT,
 submitted_at TEXT NOT NULL
);
