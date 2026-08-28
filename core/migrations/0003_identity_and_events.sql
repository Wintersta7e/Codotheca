-- §1.4 identity; §1.5/§1.9 merge_record; §1.7 xp_events; §1.8 health_delta; §1.9 fts_commits.

CREATE TABLE identity (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  email        TEXT NOT NULL,
  name         TEXT,
  is_user      INTEGER NOT NULL DEFAULT 1 CHECK (is_user IN (0, 1)),
  source       TEXT NOT NULL
                 CHECK (source IN ('gitconfig', 'noreply', 'inferred', 'manual')),
  -- NULL is the durable record that the set is seeded but never confirmed (§1.4).
  confirmed_at INTEGER,
  UNIQUE (email COLLATE NOCASE)
) STRICT;

-- §1.4: "any address sharing a local part" needs somewhere to live; v2 had a heading and no
-- table.
CREATE TABLE identity_alias (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  identity_id INTEGER NOT NULL REFERENCES identity(id) ON DELETE CASCADE,
  email       TEXT NOT NULL,
  reason      TEXT NOT NULL CHECK (reason IN ('local_part', 'coauthor', 'manual')),
  UNIQUE (email COLLATE NOCASE)
) STRICT;

-- §1.4: without this, confirming the identity set means re-walking every repository's history.
-- With it the recompute is a join, and §4.1a's authorship-first scheduling costs ~30 ms/repo.
-- This commit count is authorship evidence, not a rendered figure: §1.4's card judges an
-- address by it. The "never reward volume" ban is on XP and figures derived from counts.
CREATE TABLE project_committer (
  project_id INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  email      TEXT NOT NULL,
  commits    INTEGER NOT NULL CHECK (commits >= 0),
  PRIMARY KEY (project_id, email)
) STRICT;

-- §1.9: what lets a phase-4 split restore the absorbed name, description and art seed.
CREATE TABLE merge_record (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  survivor_project_id INTEGER NOT NULL REFERENCES project(id),
  absorbed_project_id INTEGER NOT NULL REFERENCES project(id),
  merged_at           INTEGER NOT NULL,
  association_kind    TEXT NOT NULL
                        CHECK (association_kind IN
                          ('definitive', 'strong', 'inferred', 'manual')),
  evidence_json       TEXT NOT NULL,
  absorbed_json       TEXT NOT NULL
) STRICT;

CREATE TABLE xp_events (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  ts            INTEGER NOT NULL,
  tz_offset_min INTEGER,
  project_id    INTEGER REFERENCES project(id),
  -- The logical subject, shared with the §1.12 sidecar.
  subject_key   TEXT NOT NULL,
  kind          TEXT NOT NULL
                  CHECK (kind IN ('commit_day', 'release', 'language_first', 'revival',
                                  'first_push', 'session', 'focus')),
  -- Git-derived: <kind>:<lineage_key>:<remote_key>:<local-date>. The remote component is what
  -- separates a fork from its upstream (§1.7).
  dedupe_key    TEXT NOT NULL UNIQUE,
  -- §1.7's two classes. 'git' rows delete and recompute; 'session' rows reparent.
  track         TEXT NOT NULL CHECK (track IN ('git', 'session')),
  meta          TEXT,
  CHECK ((track = 'session') = (kind IN ('session', 'focus')))
) STRICT;

CREATE INDEX idx_xp_events_project_ts ON xp_events(project_id, ts);

-- §1.8: shape only. Phase 3 writes it; phase 1 creates it so phase 3 is not a migration on a
-- live database.
CREATE TABLE health_delta (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id  INTEGER NOT NULL REFERENCES project(id),
  ts          INTEGER NOT NULL,
  layer       TEXT NOT NULL,
  from_value  REAL,
  to_value    REAL,
  detected_in TEXT NOT NULL CHECK (detected_in IN ('foreground', 'background'))
) STRICT;

-- §1.9: last 200 subjects per repo. A plain table — FTS5 is phase 3.
CREATE TABLE fts_commits (
  project_id INTEGER PRIMARY KEY REFERENCES project(id) ON DELETE CASCADE,
  subjects   TEXT NOT NULL
) STRICT;
