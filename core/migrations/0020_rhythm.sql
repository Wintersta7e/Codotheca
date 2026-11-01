-- §39.12. The settled weekly rhythm, raise-only in the schema, and the three push-record columns J1
-- reads from the copy's tracking reflogs. One transaction is applied around this file by the
-- runner; do not add BEGIN/COMMIT.
--
-- THIS FILE CONTAINS NO PRAGMA: the runner owns the foreign-key toggle, outside the transaction
-- (see `0009_remote_identity_and_facts.sql:4-10`). Nothing is rebuilt: a new table and three
-- `ADD COLUMN`s, so nothing is copied, dropped or renamed, and `project` is untouched.
--
-- `rhythm_week` references no table, so no cascade reaches it and it adds no cascade line.
--
-- No clock is read here. `settled_at` is written by the core, never defaulted from SQL.

CREATE TABLE rhythm_week (
  week_start   TEXT PRIMARY KEY,                -- the Monday, YYYY-MM-DD (writer-enforced)
  active_days  INTEGER NOT NULL CHECK (active_days BETWEEN 0 AND 7),
  target       INTEGER NOT NULL CHECK (target BETWEEN 1 AND 7),
  -- `core/src/rings/rhythm.rs` `RhythmVerdict::ALL`; `rhythm_migration.rs` inserts every one.
  verdict      TEXT NOT NULL CHECK (verdict IN ('counted', 'missed', 'waived')),
  -- The schema's `RhythmCoverage`; `rhythm_migration.rs` inserts every variant.
  git_coverage TEXT NOT NULL CHECK (git_coverage IN ('complete', 'partial')),
  settled_at   INTEGER NOT NULL,
  CHECK ((verdict = 'counted') = (active_days >= target))
) STRICT;

-- A settled week only rises: more active days, `missed`/`waived` to `counted`, `partial` to
-- `complete`. Its Monday, its target and when it settled never change.
CREATE TRIGGER rhythm_week_raise_only
BEFORE UPDATE ON rhythm_week
WHEN NEW.active_days < OLD.active_days
  OR NEW.week_start IS NOT OLD.week_start
  OR NEW.target IS NOT OLD.target
  OR NEW.settled_at IS NOT OLD.settled_at
  OR (NEW.verdict IS NOT OLD.verdict
      AND NOT (OLD.verdict IN ('missed', 'waived') AND NEW.verdict = 'counted'))
  OR (NEW.git_coverage IS NOT OLD.git_coverage
      AND NOT (OLD.git_coverage = 'partial' AND NEW.git_coverage = 'complete'))
BEGIN
  SELECT RAISE(ABORT, 'rhythm_week is raise-only');
END;

CREATE TRIGGER rhythm_week_never_deleted
BEFORE DELETE ON rhythm_week
BEGIN
  SELECT RAISE(ABORT, 'rhythm_week rows are never deleted');
END;

-- A REPLACE deletes without firing delete triggers unless recursive_triggers is on, and an upsert
-- is an insert first; both hit this.
CREATE TRIGGER rhythm_week_inserted_once
BEFORE INSERT ON rhythm_week
WHEN EXISTS (SELECT 1 FROM rhythm_week WHERE week_start = NEW.week_start)
BEGIN
  SELECT RAISE(ABORT, 'a settled week is inserted once');
END;

-- NULL in all three is "not computed": J1 has not read this copy's push record yet.
ALTER TABLE location ADD COLUMN last_push_at INTEGER;
ALTER TABLE location ADD COLUMN last_push_tz_offset_min INTEGER;
ALTER TABLE location ADD COLUMN push_record TEXT
    CHECK (push_record IS NULL OR
           push_record IN ('read', 'no_remote', 'absent', 'unreadable', 'unsupported'));
