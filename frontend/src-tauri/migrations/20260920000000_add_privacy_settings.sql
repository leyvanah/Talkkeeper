-- What to hide from a model that is not on this machine.
--
-- One row, id '1'. `hidden_terms` is sealed like the rest of the text in this
-- database: it holds the names of real people, which is exactly what the
-- archive exists to protect.
CREATE TABLE IF NOT EXISTS privacy_settings (
    id TEXT PRIMARY KEY,
    anonymize_cloud INTEGER NOT NULL DEFAULT 1,
    hidden_terms TEXT
);

INSERT OR IGNORE INTO privacy_settings (id, anonymize_cloud, hidden_terms)
VALUES ('1', 1, NULL);
