-- §28. The `xp_events` rebuild that admits `debt_day`, §28.4a's backfill, and §28.8's two debt
-- tables. One transaction is applied around this file by the runner; do not add BEGIN/COMMIT.
--
-- THIS FILE CONTAINS NO PRAGMA, and that is R59 rather than a style rule. `apply_all` wraps every
-- migration in a transaction and `PRAGMA foreign_keys` is a documented no-op inside one, so a
-- `PRAGMA foreign_keys=OFF` written here would do nothing at all. The toggle and the
-- `foreign_key_check` live in the runner, which is outside the transaction and can read the
-- pragma back. See `0009_remote_identity_and_facts.sql:4-10`.
--
-- `xp_events` is STRICT and SQLite has no ALTER CONSTRAINT, so widening two CHECKs is a
-- create-copy-drop-rename. **This rebuild is lower risk than `0009`'s, and the reason is stated
-- so nobody adds a guard for it: nothing references `xp_events.id` and it cascades to nothing** —
-- no `REFERENCES xp_events` occurs anywhere in `core/migrations/`.

-- §1.5's guarantee — "a tombstoned rowid can never be reused" — lives entirely in
-- `sqlite_sequence`'s high-water mark, and nothing in the create-copy-drop-rename procedure
-- carries it across: `DROP TABLE` deletes the row and the copy re-seeds the mark from the ids
-- actually inserted, so it silently drops to `max(id)`. Remembered here, put back after the
-- rename.
CREATE TEMP TABLE xp_events_seq AS
  SELECT seq FROM sqlite_sequence WHERE name = 'xp_events';

CREATE TABLE xp_events_new (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  ts            INTEGER NOT NULL,
  tz_offset_min INTEGER,
  project_id    INTEGER REFERENCES project(id),
  -- The logical subject, shared with the §1.12 sidecar. §28.4a: this declaration was always
  -- right and the code was wrong — `j4_history.rs` rendered a second shape for the same subject.
  -- The backfill below and the convergence in that file make the comment true.
  subject_key   TEXT NOT NULL,
  kind          TEXT NOT NULL
                  CHECK (kind IN ('commit_day', 'release', 'language_first', 'revival',
                                  'first_push', 'session', 'focus', 'debt_day')),
  -- Git-derived: <kind>:<lineage_key>:<remote_key>:<local-date>. The remote component is what
  -- separates a fork from its upstream (§1.7).
  --
  -- §28.4's `debt_day` deliberately does NOT use that shape: it is
  -- `debt_day:<subject_key>:<local-date>`. §1.7's form is safe for `commit_day` only because a
  -- project with no lineage has no commits, and a project with no lineage can absolutely have
  -- markers — every such project would collapse onto `debt_day:::<date>`, where this column's
  -- UNIQUE plus `ON CONFLICT DO NOTHING` turns the collision into silent non-payment.
  dedupe_key    TEXT NOT NULL UNIQUE,
  -- §1.7's two classes. 'git' rows delete and recompute; 'session' rows reparent.
  --
  -- `debt_day` is on the 'session' track and that is not a compromise: `track` means
  -- *recomputable / not recomputable* and 'session' is the name that class already carries. A new
  -- value such as 'observed' would be silently dropped by the sidecar's export filter and
  -- mislabelled by its restore's hard-coded literal, so a rebuild-from-sidecar would take XP
  -- away. The accepted cost is that `track` reads 'session' for a row whose kind is not a
  -- session.
  track         TEXT NOT NULL CHECK (track IN ('git', 'session')),
  meta          TEXT,
  CHECK ((track = 'session') = (kind IN ('session', 'focus', 'debt_day')))
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

-- The only index on the table (`0003:68`), dropped with it and recreated by name.
CREATE INDEX idx_xp_events_project_ts ON xp_events(project_id, ts);

-- Put the high-water mark back. The `> seq` guard makes it monotone: it restores a mark that was
-- lost and can never lower one. The INSERT arm covers a table whose rows have all been deleted,
-- where the copy inserts nothing and SQLite writes no `sqlite_sequence` row to update.
INSERT INTO sqlite_sequence (name, seq)
  SELECT 'xp_events', (SELECT seq FROM xp_events_seq)
   WHERE (SELECT seq FROM xp_events_seq) IS NOT NULL
     AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'xp_events');
UPDATE sqlite_sequence
   SET seq = (SELECT seq FROM xp_events_seq)
 WHERE name = 'xp_events'
   AND (SELECT seq FROM xp_events_seq) > seq;
DROP TABLE xp_events_seq;

-- §28.4a. Rewrite from the **project row**, never by splitting the stored string:
-- `<lineage>:<remote>` cannot be split safely in general, because neither component is
-- guaranteed free of a colon. `lineage_key` is a SHA-256 hex digest and carries none, so
-- `NOT LIKE 'lineage:%'` cannot misfire on a key already in the right shape.
--
-- The written string is byte-identical to `ProjectSubject::to_key()`'s `Lineage` arm.
UPDATE xp_events SET subject_key =
  'lineage:' || (SELECT lineage_key FROM project WHERE id = xp_events.project_id)
  || '|remote:' || COALESCE((SELECT remote_key FROM project WHERE id = xp_events.project_id), '')
 WHERE kind = 'commit_day' AND subject_key NOT LIKE 'lineage:%'
   AND project_id IS NOT NULL
   AND (SELECT lineage_key FROM project WHERE id = xp_events.project_id) IS NOT NULL;
