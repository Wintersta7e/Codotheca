-- §43.15: first run's end, for an index that already holds a shelf. One static statement over
-- phase-1 tables that reads no clock and creates nothing. MAX(created_at) places every project
-- the index holds at or before the stamp, so none of them reads NEW. An empty index gets nothing:
-- its first stored shelf is stamped at the end of the scan that stores it (§43.10).
INSERT INTO app_meta (k, v)
SELECT 'first_run_completed_at', CAST((SELECT MAX(created_at) FROM project) AS TEXT)
 WHERE EXISTS (SELECT 1 FROM project)
   AND NOT EXISTS (SELECT 1 FROM app_meta WHERE k = 'first_run_completed_at');
