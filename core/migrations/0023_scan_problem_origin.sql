-- §11.1's TRUST THIS REPOSITORY for a folder git refused: the raw path and the root it was found
-- under, so the core can find that one folder again from its own row. One transaction is applied
-- around this file by the runner; do not add BEGIN/COMMIT.
--
-- THIS FILE CONTAINS NO PRAGMA: the runner owns the foreign-key toggle, outside the transaction
-- (see `0009_remote_identity_and_facts.sql:4-10`). Nothing is rebuilt: two `ADD COLUMN`s, so
-- nothing is copied, dropped or renamed.
--
-- `path_display` stays write-once and is never read by the core (§1.10); `path_bytes` is the
-- lossless form, in the encoding `location.path_bytes` stores. The writer fills both columns for
-- `untrusted_repo` rows only; every other kind leaves them NULL. A root removed after the scan
-- sets `root_id` NULL, and the trust command refuses such a row rather than guess its root.

ALTER TABLE scan_problem ADD COLUMN path_bytes BLOB;
ALTER TABLE scan_problem ADD COLUMN root_id INTEGER REFERENCES scan_root(id) ON DELETE SET NULL;
