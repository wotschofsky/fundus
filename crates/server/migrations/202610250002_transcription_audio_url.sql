-- The AssemblyAI upload URL, saved before the paid create request. If that request's answer is
-- lost, the job is found by this URL instead of being submitted again.
ALTER TABLE transcriptions ADD COLUMN audio_url TEXT;
