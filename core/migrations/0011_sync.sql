-- §21.13. The sync runner's two tables. It alters nothing, and it touches `project_job_state`
-- **not at all**, which is the point: sync is its own runner and writes no job row (§21.1).
--
-- It runs after 0008 because `sync_budget.account_id` references `account(id)`. R68 moved it
-- from 0010 to 0011 when R65 moved this plan from wave 3 to wave 5: `apply_all` skips any
-- migration whose version <= current and stamps `user_version` per file, so a database that
-- reached the far side of a gap would skip the missing file forever, silently, on a user's
-- machine. A migration's number follows its wave.
--
-- No `PRAGMA` here: `apply_all` wraps every file in a transaction, where `PRAGMA foreign_keys`
-- is a documented no-op.

-- §21.4's durable scheduling state, one row per (task, key).
--
-- The CHECK lists are written from the slugs the writer emits, character for character (R26) —
-- `core/src/sync/task.rs`'s `kind_slug` and `core/src/sync/state.rs`'s `state_slug`, whose
-- agreement with this file is proven by inserting every one of them against a migrated database
-- rather than by reading this text.
--
-- `key` is polymorphic and deliberately carries **no foreign key**: it is an account id for
-- `account_repos` and for `rename_probe`, and a *project* id for `project_remote`, so no single
-- REFERENCES clause is true of it. That is why it is the one account-referencing table that
-- cannot cascade, and why `delete_account` calls `sync::store::delete_account_tasks` by name.
--
-- It is nullable so a process-wide task is representable. Phase 2 declares none; phase 3's
-- dependency-advisory task is the one §21.6 names, and it needs no migration when it arrives.
--
-- `at` is when the row last changed; `not_before` is when it next becomes runnable, and 0 means
-- *now*. `fail_count` and `throttle_count` are two counters and never one (§21.4): a park is not
-- a failure, so a throttled task must never strand itself in `deferred`.
CREATE TABLE sync_task_state (
  id             INTEGER PRIMARY KEY,
  task           TEXT NOT NULL CHECK (task IN ('account_repos', 'project_remote', 'rename_probe')),
  key            INTEGER,
  state          TEXT NOT NULL CHECK (state IN ('queued', 'running', 'parked', 'ok', 'deferred', 'blocked')),
  cursor         TEXT,
  fail_count     INTEGER NOT NULL DEFAULT 0 CHECK (fail_count >= 0),
  throttle_count INTEGER NOT NULL DEFAULT 0 CHECK (throttle_count >= 0),
  reason         TEXT,
  at             INTEGER NOT NULL,
  not_before     INTEGER NOT NULL DEFAULT 0
) STRICT;

-- Paired partial indexes, because neither a PRIMARY KEY nor a plain UNIQUE can express this.
CREATE UNIQUE INDEX sync_task_keyed ON sync_task_state(task, key) WHERE key IS NOT NULL;
CREATE UNIQUE INDEX sync_task_global ON sync_task_state(task)     WHERE key IS NULL;

-- §21.6's rate budget — mirrored from the server's own headers, never counted by us.
--
-- Key `(account_id NULLABLE, resource)`. **NULL is the unauthenticated per-IP pool**, which is
-- per-process and shared across every account, so it is keyed by the *absence* of an account
-- rather than by a sentinel one. This row is the phase-3 seam A1 names: the advisory task adds
-- itself against this table with no migration and no retrofit.
--
-- `limit_` keeps its trailing underscore: `limit` is a SQLite keyword and the column is read by
-- name.
--
-- **Every numeric column is nullable because unobserved is unknown, not zero.** Zero would stall
-- sync forever; the limit would burn the allowance on an assumption. Only `observed_at` is NOT
-- NULL, because a row exists exactly when something was observed.
CREATE TABLE sync_budget (
  id          INTEGER PRIMARY KEY,
  account_id  INTEGER REFERENCES account(id) ON DELETE CASCADE,
  resource    TEXT NOT NULL,
  remaining   INTEGER,
  limit_      INTEGER,
  reset_at    INTEGER,
  observed_at INTEGER NOT NULL
) STRICT;

-- Neither nullable key may be a PRIMARY KEY column and neither may rely on a plain UNIQUE.
-- A STRICT table makes every PRIMARY KEY column implicitly NOT NULL, so
-- `PRIMARY KEY (account_id, resource)` rejects the per-IP row outright; and a UNIQUE index
-- treats NULLs as distinct, so `UNIQUE(account_id, resource)` admits **unlimited duplicate**
-- per-IP rows and "the last observation" stops being a single row. The paired partial indexes
-- are what make the NULL key mean one pool.
CREATE UNIQUE INDEX sync_budget_account ON sync_budget(account_id, resource) WHERE account_id IS NOT NULL;
CREATE UNIQUE INDEX sync_budget_anon    ON sync_budget(resource)             WHERE account_id IS NULL;
