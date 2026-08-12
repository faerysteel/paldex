-- Player display names, so the UI's player selector can say "Player One"
-- rather than a UUID.
--
-- Nullable rather than NOT NULL DEFAULT '': a name genuinely can be absent
-- (the world save's IsPlayer entry carries NickName, and nothing guarantees
-- it), and an empty string would be indistinguishable from a real one.
--
-- Both columns live on `players`, which is snapshot-scoped and replaced
-- wholesale each ingest, so a renamed player follows the save with no
-- backfill needed.
ALTER TABLE players ADD COLUMN name TEXT;
ALTER TABLE players ADD COLUMN level INTEGER;
