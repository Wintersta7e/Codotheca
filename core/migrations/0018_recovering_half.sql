-- §46.15 — the recovering half's tables. One transaction is applied around this file by the
-- runner; do not add BEGIN/COMMIT. No PRAGMA anywhere in this file: nothing here is copied,
-- dropped or renamed, and the runner owns every pragma.
--
--   * `project.removed_at` — the user declared this project removed. NULL for every existing row:
--     there is no backfill, because a copy uninstalled before this file has no capture to point at.
--   * `parcel` — one preserved copy of a working copy, written to the user's chosen folder. A
--     sealed or purged parcel carries its seal (digest, manifest hash, directory name, sealed time).
--     `check_result` is never NULL: a parcel nobody has checked reads `unchecked`, never `ok`.
--   * `parcel_ref` — a sealed parcel's ref set, one OID per (repository, ref name). `repo_path` is
--     the repository's path relative to the copy's root as raw bytes, the empty blob for the
--     top-level repository. It lives here so no reader opens a manifest on a drive that may be
--     unplugged.
--   * `removal_record` — the journal of one Uninstall or Remove of one copy. At most one record
--     per location is open (`journaled`, `disposing`, `interrupted`), and `recovery` is `parcel`
--     exactly when the record names a parcel.
--   * `removal_log` — the copy's full commit log, captured before any byte moves and never
--     truncated; `format` names its line layout.
--
-- `lineage_key` is NOT NULL in both tables: a repository with no commit is refused before any row
-- is written. Every value list a CHECK spells is a generated enum's, character for character.
--
-- Foreign keys into `project` cascade; foreign keys into `location` do not, as everywhere else in
-- the schema. The `project` children that cascade are listed in
-- `core/tests/fixtures/cascade_children.txt`; `parcel_ref` and `removal_log` cascade from their
-- parent rows here, not from `project`.

ALTER TABLE project ADD COLUMN removed_at INTEGER NULL;

CREATE TABLE parcel (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  location_id INTEGER NOT NULL REFERENCES location(id),
  state TEXT NOT NULL CHECK (state IN ('preserving','sealed','abandoned','purged')),
  folder_bytes BLOB NOT NULL, dir_name TEXT NULL, staging_bytes BLOB NOT NULL,
  volume_key TEXT NULL, store_class TEXT NULL,
  lineage_key TEXT NOT NULL, state_digest TEXT NULL, manifest_sha256 TEXT NULL,
  total_bytes INTEGER NULL, git_version TEXT NULL, tar_version TEXT NULL,
  session_nonce BLOB NOT NULL, created_at INTEGER NOT NULL, sealed_at INTEGER NULL,
  checked_at INTEGER NULL, full_checked_at INTEGER NULL, full_checked_git TEXT NULL,
  check_result TEXT NOT NULL DEFAULT 'unchecked'
    CHECK (check_result IN ('unchecked','ok','missing','altered','unrestorable','unreachable')),
  purged_at INTEGER NULL,
  CHECK (state NOT IN ('sealed','purged') OR (state_digest IS NOT NULL AND manifest_sha256 IS NOT NULL
         AND dir_name IS NOT NULL AND sealed_at IS NOT NULL))
) STRICT;
CREATE INDEX parcel_by_project ON parcel(project_id, state);

CREATE TABLE parcel_ref (
  parcel_id INTEGER NOT NULL REFERENCES parcel(id) ON DELETE CASCADE,
  repo_path BLOB NOT NULL,
  ref_name  TEXT NOT NULL,
  oid       TEXT NOT NULL,
  PRIMARY KEY (parcel_id, repo_path, ref_name)
) STRICT;

CREATE TABLE removal_record (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  location_id INTEGER NOT NULL REFERENCES location(id),
  kind TEXT NOT NULL CHECK (kind IN ('uninstall','remove')),
  state TEXT NOT NULL CHECK (state IN ('journaled','disposing','done','abandoned','refused','interrupted')),
  path_bytes BLOB NOT NULL, planned TEXT NOT NULL CHECK (planned IN ('trash','hard_delete')),
  holding_bytes BLOB NULL, lineage_key TEXT NOT NULL, state_digest TEXT NOT NULL,
  recovery TEXT NOT NULL CHECK (recovery IN ('remote','parcel')),
  remotes_json TEXT NULL, remote_verified_at INTEGER NULL,
  parcel_id INTEGER NULL REFERENCES parcel(id),
  disposal TEXT NULL CHECK (disposal IN ('trashed','hard_deleted','gone_unconfirmed')),
  readme_name TEXT NULL, readme_text TEXT NULL,
  readme_truncated INTEGER NULL CHECK (readme_truncated IN (0,1)),
  session_nonce BLOB NOT NULL, started_at INTEGER NOT NULL, ended_at INTEGER NULL,
  CHECK ((recovery = 'parcel') = (parcel_id IS NOT NULL))
) STRICT;
CREATE INDEX removal_record_by_project ON removal_record(project_id);
-- At most one open record per location.
CREATE UNIQUE INDEX removal_record_one_open ON removal_record(location_id)
  WHERE state IN ('journaled','disposing','interrupted');

CREATE TABLE removal_log (
  removal_id INTEGER PRIMARY KEY REFERENCES removal_record(id) ON DELETE CASCADE,
  format INTEGER NOT NULL, log TEXT NOT NULL
) STRICT;
