-- When each word of a transcript line was said, as the recognizer reported it:
-- a JSON list of {w, s, e} (word, start and end in seconds of the recording).
--
-- Nullable and filled only by passes that know the timings — re-recognition
-- with Whisper or GigaAM. Everything else keeps working without it. It holds
-- the words, so it is sealed like `transcript`.
ALTER TABLE transcripts ADD COLUMN words TEXT;
