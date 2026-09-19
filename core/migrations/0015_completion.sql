-- §31 — per-project completion: the ten per-check rows, and the column J1 has been computing
-- and discarding since phase 1.
--
-- Two groups. `project_check` becomes the ONLY owner of check state, and
-- `project.completion_lit` / `project.completion_applicable` become a projection recomputed from
-- these rows in the same transaction. Both of `0001_meta_and_projects.sql:113-116`'s completion
-- CHECKs hold unchanged and `project` is not rebuilt.
--
-- **No PRAGMA anywhere in this file (R59).** `apply_all` wraps every migration in a transaction,
-- where `PRAGMA foreign_keys` is a documented no-op (`0009_remote_identity_and_facts.sql:4-10`);
-- the toggle and the `foreign_key_check` live in the runner, outside the transaction, and are
-- driven by `Migration.rebuilds_a_table`, which is **false** for this file. This file adds a
-- `REFERENCES project(id)` table AND an `ALTER` in one place, which is the combination most
-- likely to make an author reach for `PRAGMA foreign_keys=OFF` by habit. An earlier phase-2
-- revision did exactly that and would have destroyed data on every existing library.

-- 1. The ten checks.
--
-- `check_key` and `state` are OURS, closed by ruling (§31.1, concept.md:228-230), so their CHECKs
-- are not R26 — they mirror no third party. `unknown_reason` is a different case: it mirrors
-- `UnknownReason`, an enum §30 owns and may extend, and carries R26's full weight. All six
-- variants are written here character for character, and `completion_migration.rs` inserts one
-- row per variant **enumerated from the generated enum** rather than from a literal six. Writing
-- four of them would require a second rebuild of a WITHOUT ROWID table to correct.
--
-- **No `basis` column.** The basis is a property of the check KEY, constant per key, and is
-- stated in §31.1a's table (A9). A stored constant is one value in two places waiting to drift.
--
-- **No second index.** The primary key serves the only read there is — all ten rows for one
-- project — and `WITHOUT ROWID` makes that key the table's own storage order.
CREATE TABLE project_check (
  project_id     INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  check_key      TEXT NOT NULL
                   CHECK (check_key IN ('remote', 'readme', 'license', 'description', 'tests',
                                        'ci', 'ciGreen', 'pushed', 'deps', 'release')),
  state          TEXT NOT NULL CHECK (state IN ('pass', 'fail', 'unknown', 'na')),
  -- NULL = the user has not ruled. A J3 re-run re-proposes; it never erases this. `0` is the
  -- user OVERRIDING a proposal, which is a third value and not the absence of one.
  user_na        INTEGER CHECK (user_na IS NULL OR user_na IN (0, 1)),
  -- Why the check could not be read. NULL unless state = 'unknown'.
  unknown_reason TEXT
                   CHECK (unknown_reason IS NULL OR
                          unknown_reason IN ('needsAccount', 'notSynced', 'notRead',
                                             'notObserved', 'notRunYet', 'unreachable')),
  -- When the state was last ESTABLISHED, not when it was last attempted: a no-change recompute
  -- leaves it where it was, which is the conservative direction — an older timestamp never
  -- over-claims currency.
  observed_at    INTEGER NOT NULL,
  -- §31.1: an `unknown` check is counted AND named. A reason without the state, or the state
  -- without a reason, are both the invariant failing quietly.
  CHECK ((state = 'unknown') = (unknown_reason IS NOT NULL)),
  PRIMARY KEY (project_id, check_key)
) STRICT, WITHOUT ROWID;

-- 2. §31.2's column.
--
-- `location` is STRICT but not WITHOUT ROWID and already takes ALTERs (`0006:18`, `0007:8-9`,
-- `0010:19`), so this is an ALTER and not a create-copy-drop-rename.
--
-- Nullable with no default, joining `0002_locations_and_roots.sql:28-35`'s *observed facts*
-- block and its rule: **all NULL until observed; 0 would claim currency the app does not have**.
-- This column is that rule's own example — a depth-1 clone fetches no tags, so a stored `0` on a
-- shallow clone says *not fetched*, not *no release*, which is why §31.2's predicate reads
-- `project.is_shallow` too and is therefore a two-table read (A13.2). NULL means J1 has not
-- persisted a refstate for that location at all.
ALTER TABLE location ADD COLUMN tag_count INTEGER;
