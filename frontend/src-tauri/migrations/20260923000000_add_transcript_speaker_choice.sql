-- Who said a line, when a person decided it rather than the recognizer.
--
-- Set when the owner gives a line to another speaker. Speaker identification
-- (by device or by the model) leaves such a line's label alone, and so does
-- re-recognition, which keeps the line as it keeps any line a person edited.
ALTER TABLE transcripts ADD COLUMN speaker_set_at TEXT;
