ALTER TABLE entries ADD COLUMN pause_requested INTEGER NOT NULL DEFAULT 0 CHECK(pause_requested IN (0,1));
ALTER TABLE entries ADD COLUMN requested_work TEXT CHECK(requested_work IN ('speakers'));
