-- §1.9/§4.1 project_job_state; §1.9/§7 art_scene; §1.9 peek_cache, scan_run, scan_problem.

CREATE TABLE project_job_state (
  project_id INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  job        TEXT NOT NULL
               CHECK (job IN ('j1', 'j1_5', 'j2', 'j3', 'j4', 'j5', 'j6')),
  state      TEXT NOT NULL
               CHECK (state IN ('queued', 'running', 'ok', 'failed', 'deferred_slow')),
  fail_count INTEGER NOT NULL DEFAULT 0 CHECK (fail_count >= 0),
  reason     TEXT,
  at         INTEGER NOT NULL,
  -- §4.1: J3 and J4 are chunked with a persisted cursor, never killed and restarted.
  cursor     TEXT,
  PRIMARY KEY (project_id, job)
) STRICT;

CREATE TABLE art_scene (
  project_id     INTEGER PRIMARY KEY REFERENCES project(id) ON DELETE CASCADE,
  scene_hash     TEXT NOT NULL,
  scene_json     TEXT NOT NULL,
  schema_version INTEGER NOT NULL,
  rendered_at    INTEGER,
  -- Tracks the `card` rendition only (§7.6).
  state          TEXT NOT NULL CHECK (state IN ('pending', 'ready', 'failed', 'stale')),
  fail_count     INTEGER NOT NULL DEFAULT 0 CHECK (fail_count >= 0)
) STRICT;

-- §1.9: Peek is the phase-1 triage mechanism and v2 gave its payload nowhere to live.
CREATE TABLE peek_cache (
  project_id          INTEGER PRIMARY KEY REFERENCES project(id) ON DELETE CASCADE,
  readme_excerpt      TEXT,
  recent_commits_json TEXT,
  computed_at         INTEGER NOT NULL
) STRICT;

CREATE TABLE scan_run (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  generation  INTEGER NOT NULL,
  started_at  INTEGER NOT NULL,
  ended_at    INTEGER,
  mode        TEXT NOT NULL CHECK (mode IN ('full', 'incremental')),
  roots_json  TEXT NOT NULL,
  walked_dirs INTEGER NOT NULL DEFAULT 0 CHECK (walked_dirs >= 0),
  found_repos INTEGER NOT NULL DEFAULT 0 CHECK (found_repos >= 0),
  cancelled   INTEGER NOT NULL DEFAULT 0 CHECK (cancelled IN (0, 1))
) STRICT;

-- §11.1: the backing store for the six summary groups that read a problem. Deferred-slow reads
-- project_job_state and ambiguous lineage is a live query over project; neither is a kind here.
CREATE TABLE scan_problem (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  scan_run_id  INTEGER NOT NULL REFERENCES scan_run(id) ON DELETE CASCADE,
  kind         TEXT NOT NULL
                 -- [R26] These are the serialised protocol values, character for character.
                 -- The writer emits them from ScanProblemKind; a CHECK that spells one of them
                 -- differently rejects the insert at runtime, not in review.
                 CHECK (kind IN ('permission_denied', 'untrusted_repo', 'unreadable_repo',
                                 'clock_skew', 'non_utf8_path', 'offline_store')),
  path_display TEXT,
  detail       TEXT,
  -- count = 0 removes the group (§11.1); a zero row must not be storable.
  count        INTEGER NOT NULL DEFAULT 1 CHECK (count >= 1)
) STRICT;

CREATE INDEX idx_scan_problem_run_kind ON scan_problem(scan_run_id, kind);
