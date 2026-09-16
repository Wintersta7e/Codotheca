-- §24.9. Durable install runs and the location tombstone timestamp used by Uninstall.
-- One transaction is applied around this file by the runner; do not add transaction controls.

CREATE TABLE install_run (
  id                INTEGER PRIMARY KEY,
  project_id        INTEGER NOT NULL REFERENCES project(id),
  root_id           INTEGER NOT NULL REFERENCES scan_root(id),
  staging_bytes     BLOB NOT NULL,
  destination_bytes BLOB NOT NULL,
  state             TEXT NOT NULL
                      CHECK (state IN ('running', 'done', 'failed', 'cancelled')),
  stage             TEXT NULL,
  reason            TEXT NULL,
  detail            TEXT NULL,
  started_at        INTEGER NOT NULL,
  ended_at          INTEGER NULL
) STRICT;

ALTER TABLE location ADD COLUMN removed_at INTEGER NULL;
