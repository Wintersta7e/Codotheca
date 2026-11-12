-- §55.5: the root licence's SPDX id J7 identifies and the head it read it at, and the newest tag
-- J4 picks with the ref basis it read under. Four nullable columns; neither table is rebuilt.
ALTER TABLE project_content_scan ADD COLUMN license_id TEXT;
ALTER TABLE project_content_scan ADD COLUMN license_id_head_oid TEXT;
ALTER TABLE location ADD COLUMN latest_tag_name TEXT;
ALTER TABLE location ADD COLUMN latest_tag_basis TEXT;
