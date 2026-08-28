-- §1.3 location; §1.9 scan_root and submodule_edge.

CREATE TABLE location (
  id                   INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id           INTEGER NOT NULL REFERENCES project(id),
  kind                 TEXT NOT NULL CHECK (kind IN ('win', 'linux', 'wsl')),
  -- §1.3: NOT NULL, '' when not WSL. v1 used NULL and SQLite treats NULLs as distinct in a
  -- UNIQUE index, so the constraint below silently permitted duplicates.
  distro               TEXT NOT NULL DEFAULT '',

  -- Linux paths are arbitrary bytes and cannot round-trip through TEXT or JSON.
  path_bytes           BLOB NOT NULL,
  path_key             BLOB NOT NULL,
  -- Lossy, for the UI only. Never used to open, launch, or compare (§1.10).
  path_display         TEXT NOT NULL,

  -- [R27] Nullable, matching `MountFacts.volume_key: Option<String>`. NULL means no stable
  -- identifier exists — a bind mount, overlayfs, tmpfs — and §4.6 must handle that rather than
  -- invent one, so it is not mapped to ''. The `distro` argument for NOT NULL does not apply
  -- here: `distro` sits in a UNIQUE index, where SQLite treats NULLs as distinct; this does not.
  volume_key           TEXT,
  store_key            TEXT NOT NULL,
  presence             TEXT NOT NULL
                         CHECK (presence IN ('present', 'offline', 'missing', 'unscanned')),
  scan_generation      INTEGER NOT NULL DEFAULT 0,
  last_seen_at         INTEGER,

  -- Observed facts. All NULL until observed; 0 would claim currency the app does not have (§6).
  branch               TEXT,
  is_dirty             INTEGER CHECK (is_dirty IS NULL OR is_dirty IN (0, 1)),
  untracked_count      INTEGER,
  ahead                INTEGER,
  behind               INTEGER,
  stash_count          INTEGER,
  interrupted_op       TEXT,
  head_oid             TEXT,

  refstate_observed_at INTEGER,
  refstate_basis       TEXT,
  -- A content clock, not an observation clock. NULL = no fetch recorded, never 0 (§1.3).
  fetch_head_at        INTEGER,
  -- §11.1's TRUST THIS REPOSITORY, scoped to the exact path safe.directory takes.
  trusted_at           INTEGER,
  -- No basis exists; worktree state is never cacheable (§6).
  worktree_observed_at INTEGER,

  -- [R27] Replaces `is_worktree INTEGER`, which could not hold `RepoKind`'s four variants.
  -- The distinction is identity-relevant: a linked worktree is the same project elsewhere, a
  -- separate git dir is its own repository, and §1.5 decides lineage on exactly that. Any
  -- `is_worktree` predicate becomes `repo_kind != 'bare'`, which is `RepoKind::has_worktree()`.
  repo_kind            TEXT NOT NULL
                         CHECK (repo_kind IN ('worktree', 'linked_worktree',
                                              'separate_git_dir', 'bare')),
  common_dir_bytes     BLOB,

  CHECK (kind = 'wsl' OR distro = ''),
  UNIQUE (kind, distro, path_key)
) STRICT;

CREATE INDEX idx_location_project        ON location(project_id);
CREATE INDEX idx_location_store_presence ON location(store_key, presence);

CREATE TABLE scan_root (
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,
  kind               TEXT NOT NULL CHECK (kind IN ('win', 'linux', 'wsl')),
  distro             TEXT NOT NULL DEFAULT '',
  path_bytes         BLOB NOT NULL,
  path_key           BLOB NOT NULL,
  path_display       TEXT NOT NULL,
  enabled            INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  added_by           TEXT NOT NULL CHECK (added_by IN ('suggested', 'user')),
  -- §4.2: stop descending at a repository root unless this is set.
  descend_into_repos INTEGER NOT NULL DEFAULT 0 CHECK (descend_into_repos IN (0, 1)),
  added_at           INTEGER NOT NULL,
  CHECK (kind = 'wsl' OR distro = ''),
  UNIQUE (kind, distro, path_key)
) STRICT;

-- §1.2/§4.4: the same library can be a submodule of two parents, or appear twice under one,
-- so the relationship is an edge and not a column on project.
CREATE TABLE submodule_edge (
  parent_project_id  INTEGER NOT NULL REFERENCES project(id),
  child_project_id   INTEGER NOT NULL REFERENCES project(id),
  parent_location_id INTEGER REFERENCES location(id),
  path_bytes         BLOB NOT NULL,
  gitlink_oid        TEXT,
  PRIMARY KEY (parent_project_id, child_project_id, path_bytes)
) STRICT;
