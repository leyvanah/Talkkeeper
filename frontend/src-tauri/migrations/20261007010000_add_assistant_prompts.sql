-- Text the owner writes to steer the assistant, such as their own request for
-- a person's overview. Kept with the archive, sealed like everything else they
-- wrote, and never in the source: the default wording lives in the code, the
-- owner's own wording lives only here.
CREATE TABLE assistant_prompts (
    id TEXT PRIMARY KEY,
    prompt TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
