-- §1.1/§1.5 columns the earlier migrations do not carry.
--
-- common_dir_key: §1.1 makes "same git-common-dir" *definitive* evidence, which is a
--                 comparison. `0002` stores raw `common_dir_bytes` only, and two spellings of
--                 one directory — separator, case on Windows — compare unequal, losing the one
--                 piece of evidence that can never be wrong. This is to `common_dir_bytes`
--                 exactly what `path_key` is to `path_bytes`, and it is derived by the same
--                 canonicaliser inside `identity::store::upsert_location` rather than taken on
--                 trust from a caller.
-- disabled:       §1.5 keeps the losing row of a launch_target (kind, name) collision
--                 "disabled"; §1.9's row has nowhere to keep that, and deleting it instead
--                 would be a destructive operation phase 1 does not have (§17).
--
-- `project.association_kind` and `project.merged_into` are NOT here: `0001` already declares
-- both. Two migrations declaring one fact is the one-value-in-two-places defect this project
-- has already counted 105 of.

ALTER TABLE location ADD COLUMN common_dir_key BLOB;

ALTER TABLE launch_target ADD COLUMN disabled INTEGER NOT NULL DEFAULT 0
    CHECK (disabled IN (0, 1));

-- §1.6 resolves a stale id by looking for the tombstone; §11.1's live ambiguous-lineage query
-- excludes tombstoned rows on every read. Both scan `merged_into`.
CREATE INDEX idx_project_merged_into ON project(merged_into);
-- The definitive-evidence lookup: which project already owns a location with this common dir.
CREATE INDEX idx_location_common_dir_key ON location(common_dir_key);
