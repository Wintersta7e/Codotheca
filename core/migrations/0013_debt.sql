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

-- §28.8.3. One per-project list of concrete, derived, **closable** items, keyed on
-- `(subject_key, source, fingerprint)`.
--
-- `subject_key` is `ProjectSubject::to_key()` and **never `project_id`**: §1.7 records that v1
-- keyed the ledger on `project_id` and it broke on merges, and `subject_key` is *total* where
-- `lineage_key` is not. `project_id` is an attribute, repointed by the merge recompute, never key
-- material.
--
-- The cascade-child enumeration is NOT restated here — `0016_health_delta.sql` carries the one
-- copy, by ruling (R129/F7). A number in a comment needs one writer and no readers.
CREATE TABLE debt_item (
  id                     INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id             INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  subject_key            TEXT NOT NULL,
  source                 TEXT NOT NULL
                           CHECK (source IN ('todo_marker', 'missing_readme', 'missing_license',
                                             'missing_tests', 'no_release', 'unpushed_commits',
                                             'ci_red', 'dependency_advisory',
                                             'abandoned_with_debt')),
  -- NOT NULL and '' for a singleton, never NULL: SQLite treats NULLs as distinct inside a
  -- UNIQUE index, so a nullable fingerprint would silently permit duplicate singletons — the
  -- defect 0002_locations_and_roots.sql:7-9 records against location.distro.
  fingerprint            TEXT NOT NULL DEFAULT '',
  state                  TEXT NOT NULL CHECK (state IN ('open', 'unverified')),
  scoring                TEXT NOT NULL CHECK (scoring IN ('scored', 'shown_only')),
  -- The anchor. READ, not diagnostic: sweep.rs compares it with IS against the sweep's
  -- location_id before any closure (§28.3 rule 1).
  last_seen_location_id  INTEGER REFERENCES location(id),
  basis                  TEXT CHECK (basis IS NULL OR
                            basis IN ('head', 'index', 'worktree', 'refs', 'remote')),
  -- Linux paths are arbitrary bytes and cannot round-trip through TEXT; `path_display` is lossy
  -- and for the UI only. Both are ATTRIBUTES: a rename closes nothing.
  path_bytes             BLOB,
  path_display           TEXT,
  line                   INTEGER,
  column                 INTEGER,
  salient_text           TEXT,
  first_seen_at          INTEGER NOT NULL,
  last_seen_at           INTEGER NOT NULL,
  UNIQUE (subject_key, source, fingerprint)
) STRICT;

CREATE INDEX idx_debt_item_project_state ON debt_item(project_id, state);

-- §28.5. **The non-obvious half of this section, and not optional.** Without it,
-- `SELECT count(*) FROM debt_item WHERE project_id = ?` returns 0 for a project with no debt AND
-- for a project nobody ever looked at — *render unknown as zero* on the first day the product is
-- capable of it.
--
-- Where it diverges from `peek_cache`'s precedent, deliberately: the outcome is an explicit
-- stored value, never row-presence. `peek_cache` encodes its distinction as *row present with a
-- NULL column*, invisible to a consumer that does not know the convention; this has six outcomes
-- and four of them are reasons the sweep could not finish, while presence carries one bit.
CREATE TABLE debt_sweep (
  project_id    INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  source        TEXT NOT NULL
                  CHECK (source IN ('todo_marker', 'missing_readme', 'missing_license',
                                    'missing_tests', 'no_release', 'unpushed_commits',
                                    'ci_red', 'dependency_advisory', 'abandoned_with_debt')),
  outcome       TEXT NOT NULL CHECK (outcome IN ('complete', 'partial', 'failed',
                                                 'unobservable', 'skipped_reference',
                                                 'skipped_suppressed')),
  location_id   INTEGER REFERENCES location(id),
  -- Diagnostic: location.scan_generation as it stood at observation. READ BY NO SURFACE and by
  -- no closure rule. location.scan_generation is a live phase-1 mechanism (0002:25), so a later
  -- reader will assume this column means what that one means — it does not. (R129/F9)
  generation    INTEGER,
  basis         TEXT CHECK (basis IS NULL OR
                  basis IN ('head', 'index', 'worktree', 'refs', 'remote')),
  item_count    INTEGER,
  observed_at   INTEGER NOT NULL,
  PRIMARY KEY (project_id, source),
  -- Zero and unknown are different facts and the DDL says so, as scan_problem.count refuses a
  -- zero row (0005:61-62) and uninstall NULLs rather than zeroes (uninstall/command.rs:17-19).
  CHECK ((outcome IN ('complete', 'partial')) = (item_count IS NOT NULL))
) STRICT;
