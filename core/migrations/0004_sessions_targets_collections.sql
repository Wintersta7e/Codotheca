-- §1.6/§9 sessions; §1.9/§4bis launch_target; §1.9/§8.8 collections.

CREATE TABLE launch_target (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  -- §4bis.2a: the scope is the triple below. NULL language means "applies to any language",
  -- never "language unknown".
  project_id       INTEGER REFERENCES project(id),
  location_id      INTEGER REFERENCES location(id),
  language         TEXT,
  kind             TEXT NOT NULL
                     CHECK (kind IN ('editor', 'terminal', 'file_manager', 'git_client')),
  name             TEXT NOT NULL,
  -- An executable path is a path: bytes, never display text.
  exec_bytes       BLOB NOT NULL,
  args_json        TEXT NOT NULL DEFAULT '[]',
  cwd_mode         TEXT NOT NULL DEFAULT 'location'
                     CHECK (cwd_mode IN ('location', 'none')),
  env_json         TEXT NOT NULL DEFAULT '{}',
  -- Within a scope the default is the lowest sort_index. There is no is_default column.
  sort_index       INTEGER NOT NULL,
  detected         INTEGER NOT NULL DEFAULT 1 CHECK (detected IN (0, 1)),
  verified_at      INTEGER,
  -- §11.5. Resolution never consults this (§4bis.2a).
  verify_state     TEXT NOT NULL DEFAULT 'unverified'
                     CHECK (verify_state IN ('unverified', 'ok', 'missing', 'not_executable'))
) STRICT;

CREATE INDEX idx_launch_target_project ON launch_target(project_id);

CREATE TABLE session (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id       INTEGER NOT NULL REFERENCES project(id),
  location_id      INTEGER REFERENCES location(id),
  target_id        INTEGER REFERENCES launch_target(id),
  started_at       INTEGER NOT NULL,
  ended_at         INTEGER,
  -- §9: always the sum of the segments' credits, whatever ends the session.
  credited_seconds INTEGER NOT NULL DEFAULT 0 CHECK (credited_seconds >= 0),
  close_reason     TEXT
                     CHECK (close_reason IS NULL OR close_reason IN
                       ('stop', 'idle', 'process_exit', 'app_exit', 'crash', 'orphaned')),
  CHECK ((ended_at IS NULL) = (close_reason IS NULL))
) STRICT;

CREATE INDEX idx_session_project_started ON session(project_id, started_at DESC);

CREATE TABLE session_segment (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id       INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
  started_at       INTEGER NOT NULL,
  ended_at         INTEGER,
  credited_seconds INTEGER NOT NULL DEFAULT 0 CHECK (credited_seconds >= 0),
  closed_by        TEXT
                     CHECK (closed_by IS NULL OR closed_by IN
                       ('idle', 'session_end', 'app_exit', 'crash')),
  CHECK ((ended_at IS NULL) = (closed_by IS NULL))
) STRICT;

CREATE TABLE collection (
  id                    INTEGER PRIMARY KEY AUTOINCREMENT,
  name                  TEXT NOT NULL,
  kind                  TEXT NOT NULL CHECK (kind IN ('manual', 'query')),
  query_text            TEXT,
  query_grammar_version INTEGER,
  sort_index            INTEGER NOT NULL DEFAULT 0,
  UNIQUE (name COLLATE NOCASE),
  CHECK (kind = 'query' OR (query_text IS NULL AND query_grammar_version IS NULL)),
  CHECK (kind = 'manual' OR (query_text IS NOT NULL AND query_grammar_version IS NOT NULL))
) STRICT;

CREATE TABLE collection_member (
  collection_id INTEGER NOT NULL REFERENCES collection(id) ON DELETE CASCADE,
  project_id    INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  PRIMARY KEY (collection_id, project_id)
) STRICT;

CREATE INDEX idx_collection_member_project ON collection_member(project_id);

-- §1.9: a kind='query' row owns no members and the core rejects any written for it. A CHECK
-- cannot reach another table, so this is a trigger.
CREATE TRIGGER collection_member_rejects_query_collection
BEFORE INSERT ON collection_member
WHEN (SELECT kind FROM collection WHERE id = NEW.collection_id) = 'query'
BEGIN
  SELECT RAISE(ABORT, 'a query collection owns no members');
END;
