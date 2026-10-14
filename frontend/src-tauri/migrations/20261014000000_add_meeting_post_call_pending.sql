-- Whether a recording still waits for its processing after Stop: the refining
-- pass and telling the speakers apart. Set when the recording is saved as a
-- meeting, cleared when the processing has settled.
--
-- The queue of that work lived only in the window's memory, so quitting or
-- reloading in the middle left recordings unprocessed with nothing to say so;
-- one recorded without live text was left with no transcript at all. With the
-- mark in the database the window picks the queue up again at the next start.
ALTER TABLE meetings ADD COLUMN post_call_pending INTEGER NOT NULL DEFAULT 0;
