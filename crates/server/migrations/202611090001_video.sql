-- A recording made from one untrimmed video file also plays its picture (src/video.rs).
-- video_status: NULL without video; 'original' plays the upload as it is; 'queued' ->
-- 'converting' -> 'ready' (video.mp4) or 'failed'. Width and height are the size the browser
-- shows. video_attempts counts conversions the server did not live through.
ALTER TABLE entries ADD COLUMN video_width INTEGER CHECK(video_width > 0);
ALTER TABLE entries ADD COLUMN video_height INTEGER CHECK(video_height > 0);
ALTER TABLE entries ADD COLUMN video_failure TEXT;
ALTER TABLE entries ADD COLUMN video_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE entries ADD COLUMN video_status TEXT
    CHECK(video_status IN ('original','queued','converting','ready','failed'))
    CHECK((video_status IS NULL) = (video_width IS NULL) AND (video_status IS NULL) = (video_height IS NULL))
    CHECK((video_status IS 'failed') = (video_failure IS NOT NULL));
