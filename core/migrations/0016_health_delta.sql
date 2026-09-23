-- §34.3 — the ONE `health_delta` rebuild. One transaction is applied around this file by the
-- runner; do not add BEGIN/COMMIT.
--
-- The table has been live DDL since `0003_identity_and_events.sql:70-80` with no producer, so it
-- is empty in every existing library. It is STRICT and SQLite has no ALTER CONSTRAINT, so it may
-- be rebuilt exactly once, and all three changes ride this one create-copy-drop-rename:
--
--   * `idx_health_delta_project_ts` — the read is *the recent deltas for one project, newest
--     first*, the shape `idx_xp_events_project_ts` already serves for `xp_events`;
--   * A14.1's `layer` CHECK — the five `DecayLayer` variants, character-identical to the
--     generated enum (R26). Without it nothing stops a writer storing the prototype's internal
--     `web` or `growth` while a reader looks for `cobwebs` or `overgrowth` (§27.7);
--   * A14.6's `ON DELETE CASCADE` — which `health_delta` never carried, unlike the seven children
--     `0009_remote_identity_and_facts.sql:4-10` enumerates. That is a comparison and not a claim
--     about the rest of the schema: other `project` children still carry no cascade, so a
--     `DELETE FROM project` is still refused by them after this file.
--
-- Column names, order and declared types do not move. `from_value` and `to_value` stay REAL
-- (§34.9): the layer values are small non-negative counts, exact in IEEE-754, and this rebuild is
-- spent on the three changes above.
--
-- **No PRAGMA anywhere in this file (R59).** `apply_all` wraps every migration in a transaction,
-- where `PRAGMA foreign_keys` is a documented no-op; the toggle and the `foreign_key_check` live
-- in the runner, outside the transaction, driven by `Migration.rebuilds_a_table`. Nothing
-- references `health_delta.id`, so the drop below cascades to nothing either way.
--
-- The `project` children that cascade once this file has run — §28.8's enumeration, stated in
-- the last phase-3 migration and nowhere else, derived from `core/migrations/` rather than from a
-- document. `core/tests/health_delta_migration.rs` compares this list against the migrated schema,
-- so a later migration that adds a child without updating it fails there.
-- cascade-child: project_committer
-- cascade-child: fts_commits
-- cascade-child: collection_member
-- cascade-child: project_job_state
-- cascade-child: art_scene
-- cascade-child: peek_cache
-- cascade-child: project_account
-- cascade-child: project_content_scan
-- cascade-child: debt_item
-- cascade-child: debt_sweep
-- cascade-child: project_dependency_scan
-- cascade-child: project_lockfile
-- cascade-child: project_dependency
-- cascade-child: advisory_notified
-- cascade-child: project_check
-- cascade-child: health_delta

-- `id` is AUTOINCREMENT, so the high-water mark lives in `sqlite_sequence` and nothing in the
-- create-copy-drop-rename procedure carries it across: `DROP TABLE` deletes the row and the copy
-- re-seeds it from the ids actually inserted, silently lowering it to `max(id)`. Remembered here,
-- put back after the rename.
CREATE TEMP TABLE health_delta_seq AS
  SELECT seq FROM sqlite_sequence WHERE name = 'health_delta';

CREATE TABLE health_delta_new (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id  INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  ts          INTEGER NOT NULL,
  layer       TEXT NOT NULL
                CHECK (layer IN ('dust', 'cobwebs', 'rust', 'cracks', 'overgrowth')),
  from_value  REAL,
  to_value    REAL,
  detected_in TEXT NOT NULL CHECK (detected_in IN ('foreground', 'background'))
) STRICT;

INSERT INTO health_delta_new (id, project_id, ts, layer, from_value, to_value, detected_in)
  SELECT id, project_id, ts, layer, from_value, to_value, detected_in FROM health_delta;

DROP TABLE health_delta;
ALTER TABLE health_delta_new RENAME TO health_delta;

CREATE INDEX idx_health_delta_project_ts ON health_delta(project_id, ts);

-- Put the high-water mark back. The `> seq` guard makes it monotone: it restores a mark that was
-- lost and can never lower one. The INSERT arm covers a table whose rows have all been deleted,
-- where the copy inserts nothing and SQLite writes no `sqlite_sequence` row to update.
INSERT INTO sqlite_sequence (name, seq)
  SELECT 'health_delta', (SELECT seq FROM health_delta_seq)
   WHERE (SELECT seq FROM health_delta_seq) IS NOT NULL
     AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'health_delta');
UPDATE sqlite_sequence
   SET seq = (SELECT seq FROM health_delta_seq)
 WHERE name = 'health_delta'
   AND (SELECT seq FROM health_delta_seq) > seq;
DROP TABLE health_delta_seq;
