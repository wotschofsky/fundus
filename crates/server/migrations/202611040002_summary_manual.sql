-- A summary the owner edited is kept, the same way a hand-set title is.
ALTER TABLE entries ADD COLUMN summary_manual INTEGER NOT NULL DEFAULT 0;
