-- Which container a base camp's workers sit in, so the Bases tab can join
-- `pals.location_container_id` and say who works where.
--
-- Nullable rather than NOT NULL: an existing snapshot ingested before this
-- migration has no value to backfill from (the id lives in the save's
-- WorkerDirector blob, not in the database), so it stays NULL until the next
-- sync re-ingests the world. A NULL here means "not decoded", never "no
-- workers" -- an empty base still has a container id. See
-- paldex-model's rawdata/base_camp.rs for the byte layout it comes from.
ALTER TABLE base_camps ADD COLUMN worker_container_id TEXT;

-- `pals.location_kind` gains a fourth value, 'base', for a Pal working at a
-- base camp -- previously these fell into 'other' alongside genuinely
-- unresolvable containers. The full set is now
-- 'party' | 'box' | 'base' | 'other'.
--
-- 0001_init.sql's comment on that column has been updated to match, but a
-- database that already ran 0001 kept the old comment text, and SQLite cannot
-- alter a comment in place -- so this note is the only place a reader of an
-- already-migrated database will find it.
