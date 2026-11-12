-- §56.11: the user's own exclusion entries, after §4.3's built-in thirty, and each copy's
-- coverage by that list, which `exclusion::store::apply_coverage` derives. One table, one
-- `ADD COLUMN` on `location`; nothing is copied or rebuilt.
CREATE TABLE scan_exclusion (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  kind       TEXT NOT NULL CHECK (kind IN ('name', 'path')),
  entry      TEXT NOT NULL,       -- the trimmed name, or the path's lossy display; display only (§1.10)
  path_bytes BLOB,                -- paths::path_bytes; path entries only
  match_key  BLOB NOT NULL,       -- paths::path_key for a path; the name's UTF-8 bytes, unfolded, for a name (GA2)
  distro     TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  UNIQUE (kind, match_key, distro),
  CHECK ((kind = 'path') = (path_bytes IS NOT NULL)),
  CHECK (kind = 'path' OR distro = '')
) STRICT;
ALTER TABLE location ADD COLUMN excluded INTEGER NOT NULL DEFAULT 0 CHECK (excluded IN (0, 1));
