-- §40.7. The eight badges' awards, latched by the schema: one row per kind, ever. One transaction
-- is applied around this file by the runner; do not add BEGIN/COMMIT.
--
-- THIS FILE CONTAINS NO PRAGMA: the runner owns the foreign-key toggle, outside the transaction
-- (see `0009_remote_identity_and_facts.sql:4-10`). Nothing is rebuilt: one new table and its
-- triggers, so nothing is copied, dropped or renamed, and `project` is untouched.
--
-- The cascade on `project_id` is declared, and `badge_award_no_delete` turns it into a refusal:
-- a `DELETE FROM project` whose cascade reaches an award fires the delete trigger and aborts the
-- whole statement, with `recursive_triggers` off and on (measured, §40.7). A project delete that
-- ever appears fails loudly instead of removing an earned thing.
--
-- The insert trigger is what closes `INSERT OR REPLACE`: with `recursive_triggers` off, SQLite's
-- default, a REPLACE deletes the conflicting row without firing the delete trigger, so without
-- the insert guard it would overwrite a held award. It also makes `INSERT ... ON CONFLICT DO
-- UPDATE` of a held kind leave the row untouched.
--
-- `project_id` is the one column that moves (a merge's repoint, a restore's resolution);
-- `landed_at`/`landed_on` go from NULL to a value once, together. Nothing else is written twice.
--
-- This table's cascade line lives in `core/tests/fixtures/cascade_children.txt`, not here.
--
-- No clock is read here. Every time column is written by the core, never defaulted from SQL.

CREATE TABLE badge_award (
  kind          TEXT PRIMARY KEY
                  CHECK (kind IN ('necromancer','polyglot','comeback','curator','marathon',
                                  'undertaker','firefighter','lifeline')),
  earned_at     INTEGER NOT NULL,  -- the moment; a day-grain earn stores the day's midnight
  tz_offset_min INTEGER NOT NULL,  -- 0 for a day-grain earn (as commit_day), else the moment's offset
  earned_on     TEXT    NOT NULL,  -- local YYYY-MM-DD: the one rendered date
  project_id    INTEGER REFERENCES project(id) ON DELETE CASCADE,  -- ATTRIBUTE; NULL = library-wide
  subject_key   TEXT,              -- ProjectSubject::to_key() at award time; never rewritten
  evidence      TEXT    NOT NULL,  -- the generated BadgeEvidence, serialised; never re-derived
  awarded_at    INTEGER NOT NULL,  -- when the core wrote the row
  awarded_on    TEXT    NOT NULL,  -- its local date: the retro/live input (§40.8)
  landed_at     INTEGER,           -- write-once; NULL = never shown as a moment
  landed_on     TEXT,              -- its local date: the one-per-day input (§40.8)
  CHECK ((landed_at IS NULL) = (landed_on IS NULL))
) STRICT;

CREATE TRIGGER badge_award_no_delete BEFORE DELETE ON badge_award
BEGIN SELECT RAISE(ABORT, 'badge_award is latched'); END;

CREATE TRIGGER badge_award_no_reinsert BEFORE INSERT ON badge_award
WHEN EXISTS (SELECT 1 FROM badge_award WHERE kind = NEW.kind)
BEGIN SELECT RAISE(IGNORE); END;

CREATE TRIGGER badge_award_no_rewrite BEFORE UPDATE ON badge_award
WHEN NEW.kind IS NOT OLD.kind OR NEW.earned_at IS NOT OLD.earned_at
  OR NEW.tz_offset_min IS NOT OLD.tz_offset_min OR NEW.earned_on IS NOT OLD.earned_on
  OR NEW.subject_key IS NOT OLD.subject_key OR NEW.evidence IS NOT OLD.evidence
  OR NEW.awarded_at IS NOT OLD.awarded_at OR NEW.awarded_on IS NOT OLD.awarded_on
  OR (OLD.landed_at IS NOT NULL
      AND (NEW.landed_at IS NOT OLD.landed_at OR NEW.landed_on IS NOT OLD.landed_on))
BEGIN SELECT RAISE(ABORT, 'badge_award is latched'); END;
