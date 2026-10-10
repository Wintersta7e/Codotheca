-- §38.13. The ledger's seven kinds, its undeletable retained half, the level floor as a level, the
-- completion history backfilled, and every finished J4 queued again. One transaction is applied
-- around this file by the runner; do not add BEGIN/COMMIT.
--
-- THIS FILE CONTAINS NO PRAGMA, and that is not a style rule. `apply_all` wraps every migration
-- in a transaction and `PRAGMA foreign_keys` is a documented no-op inside one, so a
-- `PRAGMA foreign_keys=OFF` written here would do nothing at all. The toggle and the
-- `foreign_key_check` live in the runner, which is outside the transaction and can read the
-- pragma back. See `0009_remote_identity_and_facts.sql:4-10`.
--
-- This is the one `xp_events` rebuild of the phase, so it carries every kind the phase writes. A
-- row of a cut kind fails the copy, and so the whole migration, on purpose: none of those kinds
-- ever had a writer, and filtering a row out would hide a writer nobody ruled on.
--
-- No clock is read here. Every time below is one already stored.

-- 1. The rebuild, on `0013_debt.sql`'s template. `sqlite_sequence`'s high-water mark is what
-- keeps a deleted id from ever being reused, and the create-copy-drop-rename does not carry it:
-- remembered here, put back after the rename.
CREATE TEMP TABLE xp_events_seq AS
  SELECT seq FROM sqlite_sequence WHERE name = 'xp_events';

CREATE TABLE xp_events_new (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  ts            INTEGER NOT NULL,
  tz_offset_min INTEGER,
  -- No cascade: this is not a new table, and a cascading delete could only abort on the
  -- retained half's trigger below.
  project_id    INTEGER REFERENCES project(id),
  subject_key   TEXT NOT NULL,
  -- `core/src/ledger/mod.rs` `KINDS`, in its order; `ledger_migration.rs` compares the two.
  kind          TEXT NOT NULL
                  CHECK (kind IN ('commit_day', 'release', 'focus', 'debt_day',
                                  'completion_check', 'amnesty_verdict', 'clean_archive')),
  dedupe_key    TEXT NOT NULL UNIQUE,
  -- 'git' rows are a function of history: deleted and recomputed. 'session' rows are the
  -- retained half: reparented, never deleted.
  track         TEXT NOT NULL CHECK (track IN ('git', 'session')),
  meta          TEXT,
  -- `RETAINED_KINDS`, in its order.
  CHECK ((track = 'session') = (kind IN ('focus', 'debt_day', 'completion_check',
                                         'amnesty_verdict', 'clean_archive')))
) STRICT;

-- Every column named on both sides. NEVER `SELECT *`: a column-order difference between the two
-- tables would move values silently from one column into another.
INSERT INTO xp_events_new
  (id, ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
SELECT
   id, ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta
FROM xp_events;

DROP TABLE xp_events;
ALTER TABLE xp_events_new RENAME TO xp_events;

-- The only index on the table, dropped with it and recreated by name.
CREATE INDEX idx_xp_events_project_ts ON xp_events(project_id, ts);

-- Monotone: restores a mark that was lost and can never lower one. The INSERT arm covers a table
-- whose rows have all been deleted, where the copy writes no `sqlite_sequence` row to update.
INSERT INTO sqlite_sequence (name, seq)
  SELECT 'xp_events', (SELECT seq FROM xp_events_seq)
   WHERE (SELECT seq FROM xp_events_seq) IS NOT NULL
     AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'xp_events');
UPDATE sqlite_sequence
   SET seq = (SELECT seq FROM xp_events_seq)
 WHERE name = 'xp_events'
   AND (SELECT seq FROM xp_events_seq) > seq;
DROP TABLE xp_events_seq;

-- 2. The retained half is a record history cannot reproduce. Created after the rename; a later
-- rebuild is unaffected, because `DROP TABLE` drops a table's triggers before its rows.
CREATE TRIGGER xp_events_retained_rows_are_kept
BEFORE DELETE ON xp_events
WHEN old.track = 'session'
BEGIN
  SELECT RAISE(ABORT, 'a retained xp_events row cannot be deleted');
END;

-- 3. §38.6.1. The floor is a level, in one row that only ever rises. Seeded before its triggers,
-- which then refuse every other insert. `BEFORE INSERT` fires ahead of the conflict clause, so
-- `INSERT OR REPLACE` and an upsert die with a plain INSERT.
CREATE TABLE level_floor (
  id        INTEGER PRIMARY KEY CHECK (id = 1),
  level     INTEGER NOT NULL CHECK (level >= 1),
  -- NULL until the floor is first raised.
  raised_at INTEGER
) STRICT;

INSERT INTO level_floor VALUES (1, 1, NULL);

CREATE TRIGGER level_floor_has_one_row
BEFORE INSERT ON level_floor
BEGIN
  SELECT RAISE(ABORT, 'level_floor holds exactly one row');
END;

CREATE TRIGGER level_floor_is_never_deleted
BEFORE DELETE ON level_floor
BEGIN
  SELECT RAISE(ABORT, 'level_floor holds exactly one row');
END;

CREATE TRIGGER level_floor_only_rises
BEFORE UPDATE ON level_floor
WHEN NEW.level < OLD.level OR NEW.id <> OLD.id
BEGIN
  SELECT RAISE(ABORT, 'level_floor only rises');
END;

-- 4. §38.6.4. The old floor was an XP count, not a level, and nothing reads it again.
DELETE FROM app_meta WHERE k = 'level_floor';

-- 5. §38.9 step 2. One `completion_check` latch per stored pass of every project the latch's gate
-- admits, dated when that pass was last established. `subject_key` is byte-identical to
-- `ProjectSubject::to_key()`'s `Lineage` arm, the only arm a project with commit-days can have.
-- A latch is keyed on its subject and check, so a second live project on one subject finds the
-- first one's latch already written and adds none.
INSERT INTO xp_events
  (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key, track, meta)
SELECT
  c.observed_at,
  NULL,
  p.id,
  'lineage:' || p.lineage_key || '|remote:' || COALESCE(p.remote_key, ''),
  'completion_check',
  'completion_check:lineage:' || p.lineage_key || '|remote:' || COALESCE(p.remote_key, '')
    || ':' || c.check_key,
  'session',
  json_object('what', c.check_key,
              'when', date(c.observed_at, 'unixepoch'),
              'project_name', p.name,
              'backfilled', json('true'))
FROM project_check c
JOIN project p ON p.id = c.project_id
WHERE c.state = 'pass'
  AND p.authored_by_user = 1
  AND p.is_reference = 0
  AND p.merged_into IS NULL
  AND p.lineage_key IS NOT NULL
  AND (SELECT count(*) FROM xp_events e
        WHERE e.project_id = p.id AND e.kind = 'commit_day') >= 3
ORDER BY p.id, c.check_key
ON CONFLICT (dedupe_key) DO NOTHING;

-- 6. §38.9 step 3. Every finished history walks again at the next scan, to write the kinds this
-- file admits. `at` keeps the time the walk last finished; `reason` says why it is queued.
UPDATE project_job_state
   SET state = 'queued', reason = 'app_upgraded'
 WHERE job = 'j4' AND state = 'ok';

-- 7. §38.8.5a. NULL until observed.
ALTER TABLE location ADD COLUMN archive_reached INTEGER
  CHECK (archive_reached IS NULL OR archive_reached IN (0, 1));

-- 8. §39.12. NULL until observed.
ALTER TABLE session_segment ADD COLUMN signalled INTEGER
  CHECK (signalled IS NULL OR signalled IN (0, 1));
