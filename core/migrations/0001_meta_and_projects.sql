-- §1.9 app_meta and view_state; §1.2 project; §1.6 project_redirect.
-- One transaction is applied around this file by the runner; do not add BEGIN/COMMIT.

CREATE TABLE app_meta (
  k TEXT PRIMARY KEY,
  v TEXT NOT NULL
) STRICT;

-- Only the generation counter is seeded. schema_version is mirrored by Index::open after the
-- run; the rest are written by the subsystems that own them (§1.9).
INSERT INTO app_meta (k, v) VALUES ('sidecar_generation', '0');

-- §1.9: query, sort, view mode, density, collapsed sections, scroll, selection, window
-- geometry and monitor. Also §8.0's dismissal keys.
CREATE TABLE view_state (
  k TEXT PRIMARY KEY,
  v TEXT NOT NULL
) STRICT;

CREATE TABLE project (
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
  description_source         TEXT
                               CHECK (description_source IS NULL OR description_source IN
                                 ('manifest', 'readme', 'note', 'detected')),

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

  -- §1.10, in the schema as well as in the writer: unknown is not zero.
  CHECK ((completion_lit IS NULL) = (completion_applicable IS NULL)),
  CHECK (completion_applicable IS NULL OR completion_applicable > 0),
  CHECK (completion_lit IS NULL OR
         (completion_lit >= 0 AND completion_lit <= completion_applicable))
) STRICT;

-- §1.11, all four.
CREATE INDEX idx_project_lineage ON project(lineage_key);
CREATE INDEX idx_project_remote  ON project(remote_key);
CREATE INDEX idx_project_shelf_order
  ON project(last_touched_at DESC) WHERE is_hidden = 0;
CREATE INDEX idx_project_reference_order
  ON project(is_reference, last_touched_at DESC);

-- §1.6: a merge does not delete a row from the caller's point of view. One hop only.
CREATE TABLE project_redirect (
  old_project_id INTEGER PRIMARY KEY REFERENCES project(id),
  new_project_id INTEGER NOT NULL REFERENCES project(id),
  merged_at      INTEGER NOT NULL
) STRICT;
