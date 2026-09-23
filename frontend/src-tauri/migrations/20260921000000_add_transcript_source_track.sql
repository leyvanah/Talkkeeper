-- Which recorded track a transcript line came from: 'mic', 'system', or NULL
-- when that is not known (an imported recording, a line from before this).
--
-- Kept on its own because the speaker label cannot carry it: a label is
-- renamed, merged and relabelled by diarization, and every one of those used
-- to erase where the line was actually heard. With the track stored, a
-- recording made with separate tracks is labelled by device — the microphone
-- is the owner, the speakers are the other side — with no model at all.
ALTER TABLE transcripts ADD COLUMN source_track TEXT;
