-- §48.8.4 — the sidecar's pending records: what a rebuilt index restored for a subject no project
-- holds yet. The rebuild stages each per-project record here; the hand-off that brings the
-- subject back applies it and deletes the row in one transaction, so a record applies exactly
-- once. A record matching no single project stays here, and every export carries it.
--
-- No foreign key: a pending record has no project yet. No AUTOINCREMENT: an id is local to one
-- database and never exported as an identity. No PRAGMA anywhere in this file — the runner owns
-- every pragma, and this file copies, drops and renames nothing.

CREATE TABLE sidecar_pending (
  id                INTEGER PRIMARY KEY,
  source_generation INTEGER NOT NULL,
  subject_key       TEXT NOT NULL,
  location_keys     TEXT NOT NULL,
  record            TEXT NOT NULL,
  queued_at         INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_sidecar_pending_subject ON sidecar_pending(subject_key);
