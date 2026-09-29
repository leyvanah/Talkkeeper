-- Whether the built-in model looks for names the archive does not know
-- before a conversation goes to a model off this machine. On unless the owner
-- turns it off: a name said in passing is otherwise sent as it is.
ALTER TABLE privacy_settings ADD COLUMN find_names_locally INTEGER NOT NULL DEFAULT 1;
