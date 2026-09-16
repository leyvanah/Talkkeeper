-- Who each speaker of a meeting is in the conversation: the one holding it,
-- or the one it is held for.
--
-- Only exceptions are stored. The role of a speaker with no row here follows
-- from the label alone (the local microphone holds the conversation, every
-- other voice is on the other side), which is right for every recording made
-- so far and needs no data. A row exists when someone decided otherwise.
--
-- `speaker_label` is sealed the same deterministic way as
-- `transcripts.speaker` and `person_speakers.speaker_label`, so the three can
-- still be compared with `=`.
CREATE TABLE meeting_speaker_roles (
    meeting_id TEXT NOT NULL,
    speaker_label TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('host', 'client')),
    PRIMARY KEY (meeting_id, speaker_label),
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);
