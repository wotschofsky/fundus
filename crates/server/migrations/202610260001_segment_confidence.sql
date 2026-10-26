-- AssemblyAI's confidence per utterance; NULL for transcripts saved before it was kept.
ALTER TABLE segments ADD COLUMN confidence REAL;
