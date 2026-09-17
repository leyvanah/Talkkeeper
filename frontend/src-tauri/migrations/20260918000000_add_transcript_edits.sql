-- Corrections to a transcript, kept apart from what the recognizer wrote.
--
-- `edited_at` marks a line a person changed; re-recognition leaves such lines
-- alone and writes nothing over their stretch of time.
--
-- A line a person removed leaves no text behind, only where it was: the span
-- and which track it came from, so re-recognition does not bring it back.
ALTER TABLE transcripts ADD COLUMN edited_at TEXT;

CREATE TABLE transcript_removals (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL,
    audio_start_time REAL NOT NULL,
    audio_end_time REAL NOT NULL,
    track TEXT NOT NULL CHECK (track IN ('mic', 'system', 'any')),
    removed_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

CREATE INDEX idx_transcript_removals_meeting_id ON transcript_removals(meeting_id);
