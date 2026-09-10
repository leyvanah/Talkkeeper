-- A client groups recordings that belong to the same person over time.
--
-- The link from a meeting is deliberately soft (nullable, and cleared rather
-- than cascaded on delete): removing a client is a naming decision, while
-- removing a meeting destroys audio that cannot be recovered. Those two must
-- never be the same click.
CREATE TABLE clients (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL CHECK (length(trim(display_name)) > 0),
    normalized_name TEXT NOT NULL CHECK (length(normalized_name) > 0),
    notes TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Not UNIQUE on purpose: two different people may share a first name, and a
-- database that refuses the second one is wrong about the world. Duplicate
-- names are surfaced as a warning at the point of typing instead.
CREATE INDEX idx_clients_normalized_name ON clients(normalized_name);

ALTER TABLE meetings
ADD COLUMN client_id TEXT REFERENCES clients(id) ON DELETE SET NULL;

CREATE INDEX idx_meetings_client_id ON meetings(client_id);
