-- Columns §4.1 and §5.1 require and §1 never declared.
-- `project_job_state.cursor` already exists (0005); what is missing is the coverage pair, so
-- criterion 20's "visible coverage indicator" has something to read.
ALTER TABLE project_job_state ADD COLUMN progress_done INTEGER;
ALTER TABLE project_job_state ADD COLUMN progress_total INTEGER;

-- §5.1 takes a max over three inputs. Two of them had nowhere to live.
ALTER TABLE location ADD COLUMN reflog_tail_at INTEGER;
ALTER TABLE location ADD COLUMN worktree_newest_mtime INTEGER;

-- §5.1's first input is "last commit by a user identity", which is not last_commit_at (any author).
ALTER TABLE project ADD COLUMN last_user_commit_at INTEGER;

CREATE INDEX IF NOT EXISTS idx_job_state_ready ON project_job_state(state, job);
