-- §1.9 / §8.4: `peek_cache` holds two jobs' answers. J6 writes the README excerpt and J4 the
-- recent commits, in whichever order they run, so the row's presence can no longer be what says
-- J6 has looked. That moves to its own column. Every existing row was written by J6 alone, so its
-- write time is when it looked.
ALTER TABLE peek_cache ADD COLUMN readme_read_at INTEGER;
UPDATE peek_cache SET readme_read_at = computed_at;
