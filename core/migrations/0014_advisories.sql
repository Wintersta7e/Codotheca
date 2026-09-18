-- §32 — dependency health: the advisory cache, the lockfile read, and the task that fills them.
--
-- Three groups, in the order they depend on each other: the `sync_task_state` rebuild that lets
-- a process-wide task exist at all, the library-wide advisory cache, and the per-project rows the
-- cache is joined against.
--
-- **No PRAGMA anywhere in this file (R59).** `apply_all` wraps every migration in a transaction,
-- where `PRAGMA foreign_keys` is a documented no-op; the toggle lives in the runner and is driven
-- by `Migration.rebuilds_a_table`, which is `true` for this file because of group 1.

-- 1. The `sync_task_state` rebuild.
--
-- The table is STRICT and SQLite has no ALTER CONSTRAINT, so widening `task`'s CHECK for
-- 'advisories' is a create-copy-drop-rename. Three properties this copy preserves, each of which
-- is a live failure if it is dropped:
--
--   * **Both partial unique indexes are recreated.** `sync_task_global` is the slot the advisory
--     task occupies; without it two `key IS NULL` rows for one task can coexist and `put`'s
--     `ON CONFLICT(task) WHERE key IS NULL` has no index to name.
--   * **Live rows are copied, `id` included** — a park with a clock, a `deferred` row awaiting a
--     revival cause, a `blocked` row, and the cursor of a listing mid-pagination. Dropping them
--     discards a user's in-flight listing on upgrade, silently.
--   * No AUTOINCREMENT, so there is no `sqlite_sequence` mark to save, and nothing references
--     this table, so the drop fires no cascade. Both stated because two of phase 3's rebuilds do
--     need the first and an executor copying the wrong precedent would omit a needed step.
--
-- The task slugs are `SyncTaskKind`'s, character for character (R26).
CREATE TABLE sync_task_state_new (
  id             INTEGER PRIMARY KEY,
  task           TEXT NOT NULL CHECK (task IN ('account_repos', 'project_remote', 'rename_probe', 'advisories')),
  key            INTEGER,
  state          TEXT NOT NULL CHECK (state IN ('queued', 'running', 'parked', 'ok', 'deferred', 'blocked')),
  cursor         TEXT,
  fail_count     INTEGER NOT NULL DEFAULT 0 CHECK (fail_count >= 0),
  throttle_count INTEGER NOT NULL DEFAULT 0 CHECK (throttle_count >= 0),
  reason         TEXT,
  at             INTEGER NOT NULL,
  not_before     INTEGER NOT NULL DEFAULT 0
) STRICT;

INSERT INTO sync_task_state_new
  (id, task, key, state, cursor, fail_count, throttle_count, reason, at, not_before)
  SELECT id, task, key, state, cursor, fail_count, throttle_count, reason, at, not_before
    FROM sync_task_state;

DROP TABLE sync_task_state;
ALTER TABLE sync_task_state_new RENAME TO sync_task_state;

CREATE UNIQUE INDEX sync_task_keyed  ON sync_task_state(task, key) WHERE key IS NOT NULL;
CREATE UNIQUE INDEX sync_task_global ON sync_task_state(task)      WHERE key IS NULL;

-- 2. Library-wide. No `project_id`, no foreign key to any project table, and a merge never
-- touches any of them.

-- One run of the sweep. `resource` is what the task's own last response named, and the pre-issue
-- budget read is keyed by it rather than by a process-wide guess: if the endpoint names a pool
-- this build did not expect, a constant key looks up a pool the sweep never writes, `may_spend`
-- answers Unknown for ever, and Unknown spends — no brake at all until the source refuses.
--
-- `settled_at` and `outcome` drive the cadence: a sweep is due when the last **settled** one is
-- older than the interval, so a start that never finished does not satisfy it. `started_at`,
-- `triples_requested` and `triples_answered` are diagnostic, read by no surface.
CREATE TABLE advisory_sweep (
  id                INTEGER PRIMARY KEY,
  started_at        INTEGER NOT NULL,
  settled_at        INTEGER,
  outcome           TEXT,
  resource          TEXT,
  triples_requested INTEGER,
  triples_answered  INTEGER,
  complete          INTEGER NOT NULL DEFAULT 0 CHECK (complete IN (0, 1))
) STRICT;

-- The advisory itself. `severity` and `withdrawn_at` are properties of the **advisory**, not of
-- the match: carried on the match row they would be stated once per matching triple, and a
-- withdrawal would flip N rows instead of one.
--
-- **`severity` carries no CHECK and no enum.** It is the source's own vocabulary stored verbatim,
-- and a closed mirror of a third party's vocabulary is R26 by construction — the same ruling
-- `CiRunPayload.conclusion` already carries.
CREATE TABLE advisory (
  advisory_id  TEXT PRIMARY KEY,
  severity     TEXT,
  withdrawn_at INTEGER,
  summary      TEXT NOT NULL,
  url          TEXT NOT NULL,
  observed_at  INTEGER NOT NULL
) STRICT;

-- One GHSA carries several CVE ids or none, so the list is a child table rather than a column.
-- The precedent is `remote_topic`, written for exactly this shape.
CREATE TABLE advisory_cve (
  advisory_id TEXT NOT NULL REFERENCES advisory(advisory_id) ON DELETE CASCADE,
  cve_id      TEXT NOT NULL,
  PRIMARY KEY (advisory_id, cve_id)
) STRICT;

-- One row per triple the sweep **asked about**, answered or not.
--
-- It carries no match count: that is `SELECT count(*) FROM advisory_match`, and a stored copy is
-- the second owner. What it does carry is what the match rows cannot express — **no advisory and
-- never asked are both zero match rows**, and only a row here tells them apart. That is NEVER
-- RENDER UNKNOWN AS ZERO, at the storage layer.
CREATE TABLE advisory_triple (
  ecosystem    TEXT NOT NULL CHECK (ecosystem IN ('npm', 'rust', 'pip')),
  package_name TEXT NOT NULL,
  version      TEXT NOT NULL,
  sweep_id     INTEGER NOT NULL REFERENCES advisory_sweep(id),
  observed_at  INTEGER NOT NULL,
  answered     INTEGER NOT NULL CHECK (answered IN (0, 1)),
  PRIMARY KEY (ecosystem, package_name, version)
) STRICT;

-- One triple routinely matches four to six advisories, so the match is its own row and not a
-- column on the triple. `fix_available` and `fixed_version` live **here** and not on `advisory`:
-- fix availability varies per package within one advisory, and a debt item's `scoring` follows
-- it per `(ecosystem, package_name, advisory_id)`. On the advisory row, an advisory fixed in one
-- package and not another would flip both items together and one of them would be wrong.
CREATE TABLE advisory_match (
  ecosystem     TEXT NOT NULL CHECK (ecosystem IN ('npm', 'rust', 'pip')),
  package_name  TEXT NOT NULL,
  version       TEXT NOT NULL,
  advisory_id   TEXT NOT NULL REFERENCES advisory(advisory_id) ON DELETE CASCADE,
  fix_available INTEGER NOT NULL CHECK (fix_available IN (0, 1)),
  fixed_version TEXT,
  PRIMARY KEY (ecosystem, package_name, version, advisory_id),
  FOREIGN KEY (ecosystem, package_name, version)
    REFERENCES advisory_triple(ecosystem, package_name, version) ON DELETE CASCADE
) STRICT;

-- The PK leads with `ecosystem`, so a withdrawal keyed by advisory would be a full scan without
-- this. That is the half of §32.15's index bullet that is load-bearing.
CREATE INDEX idx_advisory_match_advisory ON advisory_match(advisory_id);

-- 3. Per project. Every one cascades from `project(id)`.
--
-- **No `(project_id)` index is created on the three read tables**: each one's primary key leads
-- with `project_id` and SQLite answers a prefix lookup from the PK's own index, so a second index
-- would cost a write on every row for a read that already has one.

-- *The walk ran* — the record that makes three read outcomes storable when the enum has two.
-- Its absence is *the scan has not run*, and collapsing that into *found nothing* makes every
-- unscanned project claim to have no dependencies. `dirs_entered` is diagnostic.
--
-- `unresolved_manifests` is how *a manifest with no lockfile* stays **unknown** rather than
-- reading as clean. §32.8 rules both that row and the unshipped-ecosystem row `unknown`, and
-- nothing else in this tree records that a project declares dependencies at all: J6 reads three
-- manifests but stores only a **description**, which a manifest without one does not produce. The
-- walk that finds lockfiles sees the manifests beside them for no extra read, so it counts the
-- ones whose ecosystem produced no parsed lockfile. **Presence only — no manifest is opened.**
CREATE TABLE project_dependency_scan (
  project_id           INTEGER PRIMARY KEY REFERENCES project(id) ON DELETE CASCADE,
  observed_at          INTEGER NOT NULL,
  files_matched        INTEGER NOT NULL CHECK (files_matched >= 0),
  dirs_entered         INTEGER NOT NULL CHECK (dirs_entered >= 0),
  unresolved_manifests INTEGER NOT NULL CHECK (unresolved_manifests >= 0),
  complete             INTEGER NOT NULL CHECK (complete IN (0, 1))
) STRICT;

-- One row per lockfile found. **`read_state` is a property of the FILE**, which is why it lives
-- here and not beside the triples: a file that was not read produces no (package, version) pair,
-- so a `notRead` row on the triple table would need a sentinel key. `size_bytes` is diagnostic.
--
-- **`notRead` is `DependencyReadState`'s own spelling and is deliberately not snake_cased here.**
-- The CHECK is written from the generated enum character for character (R26): a column that
-- stores a slug the writer never emits is refused at the first write, on a user's machine.
CREATE TABLE project_lockfile (
  project_id  INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  source_path TEXT NOT NULL,
  ecosystem   TEXT NOT NULL CHECK (ecosystem IN ('npm', 'rust', 'pip')),
  read_state  TEXT NOT NULL CHECK (read_state IN ('parsed', 'notRead')),
  size_bytes  INTEGER,
  observed_at INTEGER NOT NULL,
  PRIMARY KEY (project_id, source_path)
) STRICT;

-- The triples one project resolves. **No `basis` column**: its value is always `worktree`, and a
-- basis is a column where it varies per row and is stated per key where the key determines it.
-- The basis travels on `debt_sweep.basis`. `source_path` is diagnostic.
CREATE TABLE project_dependency (
  project_id   INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  ecosystem    TEXT NOT NULL CHECK (ecosystem IN ('npm', 'rust', 'pip')),
  package_name TEXT NOT NULL,
  version      TEXT NOT NULL,
  source_path  TEXT NOT NULL,
  observed_at  INTEGER NOT NULL,
  PRIMARY KEY (project_id, ecosystem, package_name, version)
) STRICT;

-- A latched, once-ever ledger: this pair was either notified or seeded, and either way it never
-- fires again. `seeded = 1` is a project's **first** computation, which writes rows and fires
-- nothing — without it, first run toasts once per critical advisory across the whole library.
CREATE TABLE advisory_notified (
  project_id  INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  advisory_id TEXT NOT NULL,
  at          INTEGER NOT NULL,
  seeded      INTEGER NOT NULL CHECK (seeded IN (0, 1)),
  PRIMARY KEY (project_id, advisory_id)
) STRICT;
