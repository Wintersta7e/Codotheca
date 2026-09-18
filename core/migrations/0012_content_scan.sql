-- §29.10. J7's store: the `project_job_state` rebuild that widens `job`'s CHECK for 'j7', then
-- the two library-wide blob-cache tables and the per-project content-scan row. One transaction is
-- applied around this file by the runner; do not add BEGIN/COMMIT.
--
-- THIS FILE CONTAINS NO PRAGMA, and that is R59 rather than a style rule. `apply_all` wraps every
-- migration in a transaction and `PRAGMA foreign_keys` is a documented no-op inside one, so the
-- toggle and the `foreign_key_check` live in the runner, outside the transaction, where the value
-- can be read back.

-- §29.1: `job`'s CHECK gains 'j7'. The table is STRICT and SQLite has no ALTER CONSTRAINT, so
-- this is a create-copy-drop-rename.
--
-- THE COPY MUST CARRY `progress_done` AND `progress_total`. They are not in `0005`'s text:
-- `0007_jobs_derived.sql:4-5` added them by ALTER *after* `0005` declared this table, so a new
-- table drafted from `0005` alone drops both silently — and criterion 20's coverage indicator
-- reads them. `idx_job_state_ready` (`0007:14`) hangs off the dropped table and is recreated
-- below for the same reason.
--
-- There is no AUTOINCREMENT here — the key is `(project_id, job)` — so unlike `0009`'s `project`
-- rebuild there is no `sqlite_sequence` high-water mark to save and restore.
CREATE TABLE project_job_state_new (
  project_id     INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  job            TEXT NOT NULL
                   CHECK (job IN ('j1', 'j1_5', 'j2', 'j3', 'j4', 'j5', 'j6', 'j7')),
  state          TEXT NOT NULL
                   CHECK (state IN ('queued', 'running', 'ok', 'failed', 'deferred_slow')),
  fail_count     INTEGER NOT NULL DEFAULT 0 CHECK (fail_count >= 0),
  reason         TEXT,
  at             INTEGER NOT NULL,
  -- §4.1: J3, J4 and now J7 are chunked with a persisted cursor, never killed and restarted.
  cursor         TEXT,
  progress_done  INTEGER,
  progress_total INTEGER,
  PRIMARY KEY (project_id, job)
) STRICT;

INSERT INTO project_job_state_new
  (project_id, job, state, fail_count, reason, at, cursor, progress_done, progress_total)
SELECT project_id, job, state, fail_count, reason, at, cursor, progress_done, progress_total
  FROM project_job_state;

DROP TABLE project_job_state;
ALTER TABLE project_job_state_new RENAME TO project_job_state;

CREATE INDEX IF NOT EXISTS idx_job_state_ready ON project_job_state(state, job);

-- §29.3. One row per (blob_oid, scanner_version): library-wide, permanent, content-addressed,
-- and deliberately carrying NO project_id — a blob's content is not a property of any project
-- (§29.12.3), which is also why a merge leaves both cache tables alone (§29.10).
--
-- The row exists because A10 re-grained `blob_finding` to one row per occurrence: a blob with no
-- markers writes no occurrence row, so the occurrence table alone cannot tell *read and clean*
-- from *never read* — render-unknown-as-zero at the cache layer, and every clean blob re-read for
-- ever. This is the "a row saying I looked" shape `peek_cache` already ships, one level down.
CREATE TABLE blob_scan (
  blob_oid        TEXT NOT NULL,
  scanner_version INTEGER NOT NULL,
  -- [R26] The serialised values `BlobOutcome::slug` emits, character for character.
  outcome         TEXT NOT NULL CHECK (outcome IN ('scanned', 'too_large', 'binary')),
  -- Diagnostic, read by no surface (R129/F9): `too_large` without the size is a verdict with no
  -- evidence. A later reader must not give it a meaning its writer never promised.
  size_bytes      INTEGER NOT NULL,
  scanned_at      INTEGER NOT NULL,
  PRIMARY KEY (blob_oid, scanner_version)
) STRICT;

-- §29.3. One row per occurrence, child of `blob_scan`. Marker counts are derived from these rows
-- and are stored nowhere beside them (A10).
CREATE TABLE blob_finding (
  blob_oid            TEXT NOT NULL,
  scanner_version     INTEGER NOT NULL,
  -- 0-based, ascending on (line, column) **within this blob**. §28.1's per-project ordinal is a
  -- different number over a different set and the two must never be conflated.
  ordinal_in_blob     INTEGER NOT NULL,
  -- [R26] `concept.md:204`'s three markers, as `Marker::slug` emits them.
  marker              TEXT NOT NULL CHECK (marker IN ('TODO', 'FIXME', 'HACK')),
  salient_sha256      TEXT NOT NULL,
  salient_text_capped TEXT NOT NULL,
  -- 1-based; `column` is in bytes, because J7 does not parse and does not know the encoding.
  line                INTEGER NOT NULL,
  "column"            INTEGER NOT NULL,
  PRIMARY KEY (blob_oid, scanner_version, ordinal_in_blob),
  FOREIGN KEY (blob_oid, scanner_version)
    REFERENCES blob_scan (blob_oid, scanner_version) ON DELETE CASCADE
) STRICT;

-- §29.5. One row per project. It joins the deleted-and-recomputed class at a merge (§29.10).
--
-- `head_oid` is NOT NULL, and that is the unborn-HEAD rule made structural (§29.1): a project
-- with no HEAD tree cannot have a row, and the row's absence is "J7 has never observed this
-- project" — which §31 renders as `unknown`. No fourth tri-state value is invented for it.
--
-- There is NO `basis` column. J7's basis is `head` for every row without exception, so a column
-- would be a per-row copy of a value the table already determines — R12 created on purpose
-- (§29.1, R129/F3). The basis is stated, not stored.
CREATE TABLE project_content_scan (
  project_id           INTEGER PRIMARY KEY REFERENCES project(id) ON DELETE CASCADE,
  head_oid             TEXT NOT NULL,
  -- The head the last **complete** scan covered; NULL until one completes. The per-project marker
  -- aggregate exists if and only if this equals `head_oid` — there is no flag to remember to set.
  complete_head_oid    TEXT,
  -- NULL is *not enumerated*, never 0.
  blobs_total          INTEGER,
  blobs_pending        INTEGER,
  -- §29.4's four lists and §29.2's `prog` filter. NOT `scanner_version`: widening the `tests`
  -- list must not invalidate a library-wide blob cache, and widening the marker set must not
  -- force a re-enumeration. The two are bumped independently.
  predicate_version    INTEGER NOT NULL,
  -- [R26] The spellings §29.4 fixes, as `PresenceState::slug` emits them. `not_read` is not
  -- `absent`: a timeout looks exactly like a missing file.
  has_readme           TEXT NOT NULL CHECK (has_readme  IN ('present', 'absent', 'not_read')),
  has_license          TEXT NOT NULL CHECK (has_license IN ('present', 'absent', 'not_read')),
  has_tests            TEXT NOT NULL CHECK (has_tests   IN ('present', 'absent', 'not_read')),
  has_ci               TEXT NOT NULL CHECK (has_ci      IN ('present', 'absent', 'not_read')),
  presence_observed_at INTEGER NOT NULL,
  enumerated_at        INTEGER NOT NULL,
  completed_at         INTEGER
) STRICT;
