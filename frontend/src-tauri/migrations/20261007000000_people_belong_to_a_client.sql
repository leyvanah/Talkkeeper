-- A person now belongs to one client.
--
-- Until now a profile was keyed by name alone, across the whole archive: two
-- different clients' "Anna" were one person, and an overview of her mixed both
-- conversations. From here a profile is (client, name), and meetings without a
-- client carry no profile at all.
--
-- `normalized_name` loses its archive-wide UNIQUE, which SQLite can only do by
-- rebuilding the table. sqlx runs every SQLite migration inside a transaction,
-- where foreign keys cannot be switched off, so dropping `people` empties
-- `person_speakers` through its ON DELETE CASCADE. The links are therefore
-- copied aside first and put back once the new table has the old name.
--
-- Nothing is decrypted: sealed names and the blind index are copied as they
-- are, so this runs before the archive is unlocked.

CREATE TABLE people_new (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL CHECK (length(trim(display_name)) > 0),
    normalized_name TEXT NOT NULL CHECK (length(normalized_name) > 0),
    notes TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    -- Cleared, not cascaded, when the client goes: what was written about a
    -- person is not lost to a client being renamed away.
    client_id TEXT REFERENCES clients(id) ON DELETE SET NULL,
    UNIQUE (client_id, normalized_name)
);

-- Every client a profile reached through its meetings, and how often.
CREATE TEMP TABLE person_clients AS
SELECT ps.person_id AS person_id,
       m.client_id AS client_id,
       COUNT(DISTINCT m.id) AS meetings,
       MIN(m.created_at) AS first_at
FROM person_speakers ps
JOIN meetings m ON m.id = ps.meeting_id
WHERE m.client_id IS NOT NULL
GROUP BY ps.person_id, m.client_id;

-- The client a profile stays with, notes and id included: the one with the
-- most meetings, then the earliest.
CREATE TEMP TABLE person_home AS
SELECT person_id, client_id FROM (
    SELECT person_id, client_id,
           ROW_NUMBER() OVER (
               PARTITION BY person_id
               ORDER BY meetings DESC, first_at ASC, client_id ASC
           ) AS rank
    FROM person_clients
) WHERE rank = 1;

INSERT INTO people_new (id, display_name, normalized_name, notes, created_at, updated_at, client_id)
SELECT p.id, p.display_name, p.normalized_name, p.notes, p.created_at, p.updated_at, h.client_id
FROM people p
LEFT JOIN person_home h ON h.person_id = p.id;

-- Every other client gets a profile of its own under the same name, without
-- the notes: those were written about one person, not about all of them.
CREATE TEMP TABLE person_splits AS
SELECT pc.person_id AS person_id,
       pc.client_id AS client_id,
       'person-' || lower(hex(randomblob(16))) AS new_id
FROM person_clients pc
JOIN person_home h ON h.person_id = pc.person_id
WHERE pc.client_id <> h.client_id;

INSERT INTO people_new (id, display_name, normalized_name, notes, created_at, updated_at, client_id)
SELECT s.new_id, p.display_name, p.normalized_name, NULL, datetime('now'), datetime('now'), s.client_id
FROM person_splits s
JOIN people p ON p.id = s.person_id;

-- The links, set aside: dropping `people` takes them with it (see above).
-- They are re-pointed here, on a copy with no foreign keys, because the new
-- profiles exist only in `people_new` until it takes the old name.
CREATE TEMP TABLE person_speakers_kept AS SELECT * FROM person_speakers;

UPDATE person_speakers_kept
SET person_id = (
    SELECT s.new_id
    FROM person_splits s
    JOIN meetings m ON m.client_id = s.client_id
    WHERE s.person_id = person_speakers_kept.person_id
      AND m.id = person_speakers_kept.meeting_id
)
WHERE EXISTS (
    SELECT 1
    FROM person_splits s
    JOIN meetings m ON m.client_id = s.client_id
    WHERE s.person_id = person_speakers_kept.person_id
      AND m.id = person_speakers_kept.meeting_id
);

-- A meeting without a client carries no profile. Its lines keep their
-- speaker names; moving it to a client links them again.
DELETE FROM person_speakers_kept
WHERE meeting_id IN (SELECT id FROM meetings WHERE client_id IS NULL);

DROP TABLE people;
ALTER TABLE people_new RENAME TO people;
CREATE INDEX idx_people_display_name ON people(display_name);
CREATE INDEX idx_people_client_id ON people(client_id);

INSERT INTO person_speakers (person_id, meeting_id, speaker_label)
SELECT person_id, meeting_id, speaker_label FROM person_speakers_kept;
DROP TABLE person_speakers_kept;

-- A profile left with no meetings goes, unless something was written in it.
DELETE FROM people
WHERE notes IS NULL
  AND NOT EXISTS (SELECT 1 FROM person_speakers ps WHERE ps.person_id = people.id);

DROP TABLE person_clients;
DROP TABLE person_home;
DROP TABLE person_splits;
