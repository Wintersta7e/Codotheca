-- §22.11 and §25.7. The ONE `project` rebuild phase 2 performs, then the three remote-fact
-- tables. One transaction is applied around this file by the runner; do not add BEGIN/COMMIT.
--
-- THIS FILE CONTAINS NO PRAGMA, and that is R59 rather than a style rule. `apply_all` wraps every
-- migration in a transaction and `PRAGMA foreign_keys` is a documented no-op inside one, so a
-- `PRAGMA foreign_keys=OFF` written here would do nothing and the `DROP TABLE project` below
-- would fire seven `ON DELETE CASCADE` children — every `project_committer`, `fts_commits`,
-- `collection_member`, `project_job_state`, `art_scene`, `peek_cache` and `project_account` row in
-- the library, emptied silently. The toggle and the `foreign_key_check` live in the runner, which
-- is outside the transaction and can read the pragma back.
--
-- `project` is STRICT and SQLite has no ALTER CONSTRAINT, so widening `description_source`'s CHECK
-- is a create-copy-drop-rename. §22.11's and §25.7's columns therefore ride the same rebuild: the
-- table may only be rebuilt once.
--
-- `project` carries NO trigger. The only `CREATE TRIGGER` in the tree is
-- `collection_member_rejects_query_collection` (`0004:81`), which is on another table. Stated
-- positively so nobody concludes one was lost here.

-- §1.5's guarantee — "a tombstoned rowid can never be reused" (`0001:21-22`) — lives entirely in
-- `sqlite_sequence`'s high-water mark, and nothing in the create-copy-drop-rename procedure
-- carries it across: `DROP TABLE` deletes the row and the copy re-seeds the mark from the ids
-- actually inserted, so it silently drops to `max(id)`. Remembered here, put back after the
-- rename.
CREATE TEMP TABLE project_seq AS
  SELECT seq FROM sqlite_sequence WHERE name = 'project';

CREATE TABLE project_new (
  -- §1.5: AUTOINCREMENT so a tombstoned rowid can never be reused.
  id                         INTEGER PRIMARY KEY AUTOINCREMENT,

  -- §1.1 identity. lineage_key is deliberately NOT unique: a fork and its upstream share one.
  lineage_key                TEXT,
  remote_key                 TEXT,
  ambiguous_lineage          INTEGER NOT NULL DEFAULT 0
                               CHECK (ambiguous_lineage IN (0, 1)),
  association_kind           TEXT
                               CHECK (association_kind IS NULL OR association_kind IN
                                 ('definitive', 'strong', 'inferred', 'manual')),

  -- §1.2: deprecated in favour of submodule_edge, kept because §4.4 still sets it.
  parent_project_id          INTEGER REFERENCES project(id),
  submodule_path             TEXT,

  name                       TEXT NOT NULL,
  owner                      TEXT,
  description                TEXT,
  -- §25.7 widens this by one value. The forge's description is a source of its own: folding it
  -- into 'manifest' or 'detected' would make the chain unable to say where the text came from.
  description_source         TEXT
                               CHECK (description_source IS NULL OR description_source IN
                                 ('manifest', 'readme', 'note', 'detected', 'remote')),

  primary_language           TEXT,
  language_bytes             TEXT,
  archetype                  TEXT,

  first_commit_at            INTEGER,
  first_commit_tz_offset_min INTEGER,
  first_commit_sha           TEXT,
  last_commit_at             INTEGER,
  last_commit_subject        TEXT,
  last_interaction_at        INTEGER,
  -- §5.1 owns this. NULL, never 0, until a scan job produces one.
  last_touched_at            INTEGER,

  is_shallow                 INTEGER NOT NULL DEFAULT 0 CHECK (is_shallow IN (0, 1)),
  -- NULL = not computed. 0 implies Reference (§1.2, §5.5).
  authored_by_user           INTEGER CHECK (authored_by_user IS NULL OR authored_by_user IN (0, 1)),
  is_bare                    INTEGER NOT NULL DEFAULT 0 CHECK (is_bare IN (0, 1)),
  is_fork                    INTEGER NOT NULL DEFAULT 0 CHECK (is_fork IN (0, 1)),
  is_pinned                  INTEGER NOT NULL DEFAULT 0 CHECK (is_pinned IN (0, 1)),
  is_archived                INTEGER NOT NULL DEFAULT 0 CHECK (is_archived IN (0, 1)),
  is_hidden                  INTEGER NOT NULL DEFAULT 0 CHECK (is_hidden IN (0, 1)),
  is_reference               INTEGER NOT NULL DEFAULT 0 CHECK (is_reference IN (0, 1)),

  -- §5.4's enum, and only §5.4's. NULL until a scan job produces one; that is neither
  -- 'empty' nor 'offline'.
  condition_signal           TEXT
                               CHECK (condition_signal IS NULL OR condition_signal IN
                                 ('live', 'idle', 'dormant', 'neglected', 'abandoned',
                                  'offline', 'empty')),
  -- Recorded from last_commit_at, not rendered in phase 1 (§1.2).
  condition_material         TEXT,

  -- §1.10: NULL-able with no DEFAULT. Nothing in phase 1 writes either.
  completion_lit             INTEGER,
  completion_applicable      INTEGER,

  size_tracked_bytes         INTEGER,
  size_worktree_bytes        INTEGER,
  tracked_files              INTEGER,

  art_scene_hash             TEXT,
  art_state                  TEXT NOT NULL DEFAULT 'pending'
                               CHECK (art_state IN ('pending', 'ready', 'failed', 'stale')),
  -- §7.4: the directory basename at first index. Write-once; §7.3a derives every art value
  -- from it, and project.name is explicitly the wrong value.
  seed_basename              TEXT NOT NULL,
  -- §7.2: incremented by art.rerender.
  reroll_offset              INTEGER NOT NULL DEFAULT 0 CHECK (reroll_offset >= 0),

  -- §1.2: operator opt-out. Nothing in phase 1 writes it and no surface offers it.
  slow_repo                  INTEGER NOT NULL DEFAULT 0 CHECK (slow_repo IN (0, 1)),

  -- Never-succeeded state, distinct from stale (§6, §11.1).
  error_kind                 TEXT,
  error_detail               TEXT,
  error_at                   INTEGER,

  -- §10.5: NULL until first opened or launched. Never derived from last_interaction_at.
  acknowledged_at            INTEGER,

  notes                      TEXT,

  -- §1.5: the tombstone. An absorbed row is never deleted.
  merged_into                INTEGER REFERENCES project(id),

  created_at                 INTEGER NOT NULL,
  updated_at                 INTEGER NOT NULL,

  -- §5.5's authorship gate, added by 0007.
  last_user_commit_at        INTEGER,

  -- §22.11's identity binding: which forge repository this project row IS. The pair is the key
  -- every §25.7 fact row is keyed on; the basis says which evidence established the link.
  provider                   TEXT,
  provider_repo_id           TEXT,
  remote_link_basis          TEXT
                               CHECK (remote_link_basis IS NULL OR remote_link_basis IN
                                 ('provider_id', 'remote_key')),
  -- §25.7: when the forge's README was last read. NULL is "never observed", never a zero.
  readme_remote_at           INTEGER,

  -- §1.10, in the schema as well as in the writer: unknown is not zero.
  CHECK ((completion_lit IS NULL) = (completion_applicable IS NULL)),
  CHECK (completion_applicable IS NULL OR completion_applicable > 0),
  CHECK (completion_lit IS NULL OR
         (completion_lit >= 0 AND completion_lit <= completion_applicable))
) STRICT;

-- Every column named on both sides. NEVER `SELECT *`: a column-order difference between the two
-- tables would move values silently from one column into another.
INSERT INTO project_new
  (id, lineage_key, remote_key, ambiguous_lineage, association_kind, parent_project_id,
   submodule_path, name, owner, description, description_source, primary_language,
   language_bytes, archetype, first_commit_at, first_commit_tz_offset_min, first_commit_sha,
   last_commit_at, last_commit_subject, last_interaction_at, last_touched_at, is_shallow,
   authored_by_user, is_bare, is_fork, is_pinned, is_archived, is_hidden, is_reference,
   condition_signal, condition_material, completion_lit, completion_applicable,
   size_tracked_bytes, size_worktree_bytes, tracked_files, art_scene_hash, art_state,
   seed_basename, reroll_offset, slow_repo, error_kind, error_detail, error_at,
   acknowledged_at, notes, merged_into, created_at, updated_at, last_user_commit_at)
SELECT
   id, lineage_key, remote_key, ambiguous_lineage, association_kind, parent_project_id,
   submodule_path, name, owner, description, description_source, primary_language,
   language_bytes, archetype, first_commit_at, first_commit_tz_offset_min, first_commit_sha,
   last_commit_at, last_commit_subject, last_interaction_at, last_touched_at, is_shallow,
   authored_by_user, is_bare, is_fork, is_pinned, is_archived, is_hidden, is_reference,
   condition_signal, condition_material, completion_lit, completion_applicable,
   size_tracked_bytes, size_worktree_bytes, tracked_files, art_scene_hash, art_state,
   seed_basename, reroll_offset, slow_repo, error_kind, error_detail, error_at,
   acknowledged_at, notes, merged_into, created_at, updated_at, last_user_commit_at
FROM project;

DROP TABLE project;
ALTER TABLE project_new RENAME TO project;

-- §1.11's four, byte-identical to `0001:120-125`, plus `0006:25`'s fifth. Two of them are not
-- plain: `idx_project_shelf_order` is partial and `idx_project_reference_order` is composite, and
-- recreating either one plainly would change the shelf's query plan and its row set without
-- changing any name a test compares.
CREATE INDEX idx_project_lineage ON project(lineage_key);
CREATE INDEX idx_project_remote  ON project(remote_key);
CREATE INDEX idx_project_shelf_order
  ON project(last_touched_at DESC) WHERE is_hidden = 0;
CREATE INDEX idx_project_reference_order
  ON project(is_reference, last_touched_at DESC);
CREATE INDEX idx_project_merged_into ON project(merged_into);

-- §22.11's per-listing lookup, which must not table-scan. **Deliberately NOT UNIQUE.** §22.5's
-- two live projects on one `remote_key` after a history rewrite, and §22.7's rename repair
-- resolving two such projects to one forge id, both reach a second row on one pair — and under a
-- UNIQUE index that becomes a failed transaction where §22.5 requires ambiguity. What enforces
-- one project row per pair is behavioural and lands in three plans; the index is a performance
-- requirement, not a constraint.
CREATE INDEX idx_project_provider_repo ON project(provider, provider_repo_id);

-- Put the high-water mark back. The `> seq` guard makes it monotone: it restores a mark that was
-- lost and can never lower one. The INSERT arm covers a table whose rows have all been deleted,
-- where the copy inserts nothing and SQLite writes no `sqlite_sequence` row to update.
INSERT INTO sqlite_sequence (name, seq)
  SELECT 'project', (SELECT seq FROM project_seq)
   WHERE (SELECT seq FROM project_seq) IS NOT NULL
     AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'project');
UPDATE sqlite_sequence
   SET seq = (SELECT seq FROM project_seq)
 WHERE name = 'project'
   AND (SELECT seq FROM project_seq) > seq;
DROP TABLE project_seq;

-- §25.7's three fact tables. **Keyed `(provider, provider_repo_id)`, never on `remote_key`**
-- (§22.9): the forge's stable id survives a rename, a transfer and a host alias, so the facts do
-- too, and `remote_key` is deliberately non-unique because two live projects can legitimately
-- carry one.
--
-- They are a fact cache with **no identity role**. The binding that says *this project row is
-- that forge repository* — `provider`, `provider_repo_id`, `remote_link_basis` — lives on
-- `project` above; these rows reference it and are not a second home for it, which is why none
-- of them carries a `remote_link_basis` column.
--
-- This plan creates them and writes **no row** into any of them: §21 writes them per listing
-- page and §25 reads them. `fork_parent_remote_key` is created here and written there (§22.8):
-- it is a rendered forge fact the matcher never reads, so it takes this row's observation clock.
--
-- Every `*_at` is nullable-means-unknown. A `200` writes a value and its clock together, a `304`
-- confirms both, and a `403` or `404` dates neither.
CREATE TABLE remote_repo (
  provider               TEXT NOT NULL,
  provider_repo_id       TEXT NOT NULL,
  visibility             TEXT,
  description            TEXT,
  fork_parent_remote_key TEXT,
  stars                  INTEGER,
  open_issues            INTEGER,
  good_first_issues      INTEGER,
  open_prs               INTEGER,
  open_prs_from_user     INTEGER,
  permitted              INTEGER NOT NULL DEFAULT 1,
  observed_at            INTEGER,
  etag                   TEXT,
  -- The Actions read has its own clock and its own validator; one call that did not observe the
  -- other's value must not move it. There is no `etag_observed_at`: an ETag is never rendered as
  -- a time (A15).
  ci_observed_at         INTEGER,
  ci_etag                TEXT,
  PRIMARY KEY (provider, provider_repo_id)
) STRICT;

CREATE TABLE remote_topic (
  provider         TEXT NOT NULL,
  provider_repo_id TEXT NOT NULL,
  topic            TEXT NOT NULL,
  PRIMARY KEY (provider, provider_repo_id, topic)
) STRICT, WITHOUT ROWID;

-- At most five rows per pair, trimmed by §21.
--
-- `conclusion` is TEXT NULL with **no CHECK**: the vocabulary belongs to the forge, and a closed
-- mirror of a third party's vocabulary is R26 by construction — a DDL CHECK that rejects the
-- values the product's own source will one day emit.
CREATE TABLE remote_ci_run (
  provider         TEXT NOT NULL,
  provider_repo_id TEXT NOT NULL,
  run_id           INTEGER NOT NULL,
  workflow_name    TEXT NOT NULL,
  conclusion       TEXT,
  branch           TEXT NOT NULL,
  run_number       INTEGER NOT NULL,
  started_at       INTEGER,
  PRIMARY KEY (provider, provider_repo_id, run_id)
) STRICT;
