//! A migrated, empty index in a temporary directory, plus the handful of writes the
//! derived-value tests need.
//!
//! It exists so those tests assert against the real DDL — the NULL-ability, the CHECK
//! constraints and the column names — rather than against a mock that would agree with whatever
//! the code happened to do.
//!
//! The `column` arguments are interpolated into SQL, which is why this type lives behind the
//! default-off `testkit` feature and is never compiled into the shipped binary.
//!
//! `expect` and `panic` are denied crate-wide because a panic kills the process the shell
//! supervises. That reasoning does not reach here: nothing in this file runs outside a test
//! binary, and a fixture that cannot build its own database must abort loudly rather than hand
//! a test a half-made index to assert against.
#![allow(clippy::expect_used, clippy::panic)]

use std::path::Path;

use crate::index::Index;
use crate::jobs::state::{put, JobStateRow};
use crate::jobs::{JobKind, JobState};
use crate::protocol::{LocationId, ProjectId};

/// An index on disk that goes away when the value does.
#[derive(Debug)]
pub struct TempIndex {
    dir: tempfile::TempDir,
    index: Index,
}

impl TempIndex {
    /// Open a fresh, fully migrated index.
    ///
    /// # Panics
    /// If the temporary directory cannot be made or the migrations do not apply.
    #[must_use]
    pub fn new() -> TempIndex {
        let dir = tempfile::tempdir().expect("temp dir");
        let index = Index::open_at(dir.path(), 0).expect("open index");
        TempIndex { dir, index }
    }

    /// The index itself.
    #[must_use]
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// The index, mutably — `Index::with_tx` needs it, because so does
    /// `rusqlite::Connection::transaction`.
    pub fn index_mut(&mut self) -> &mut Index {
        &mut self.index
    }

    fn exec(&self, sql: &str, params: &[&dyn rusqlite::ToSql]) {
        self.index
            .conn()
            .execute(sql, params)
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// A minimal `project` row.
    ///
    /// # Panics
    /// If the insert is refused.
    pub fn insert_project(&self) -> ProjectId {
        self.exec(
            "INSERT INTO project (name, seed_basename, created_at, updated_at)
             VALUES ('p', 'p', 0, 0)",
            &[],
        );
        ProjectId(self.index.conn().last_insert_rowid())
    }

    /// A present, native `location` row at `path`.
    ///
    /// # Panics
    /// If the insert is refused.
    pub fn insert_location(&self, project: ProjectId, path: &str) -> LocationId {
        let native = if cfg!(windows) { "win" } else { "linux" };
        self.exec(
            "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                   store_key, presence, repo_kind)
             VALUES (?1, ?2, ?3, ?3, ?4, 'store', 'present', 'worktree')",
            &[&project.0, &native, &path.as_bytes(), &path],
        );
        LocationId(self.index.conn().last_insert_rowid())
    }

    /// Set one `project` column to an integer.
    ///
    /// # Panics
    /// If the column does not exist or the update is refused.
    pub fn set_project_i64(&self, project: ProjectId, column: &str, value: i64) {
        self.exec(
            &format!("UPDATE project SET {column} = ?2 WHERE id = ?1"),
            &[&project.0, &value],
        );
    }

    /// Set one `location` column to an integer.
    ///
    /// # Panics
    /// If the column does not exist or the update is refused.
    pub fn set_location_i64(&self, location: LocationId, column: &str, value: i64) {
        self.exec(
            &format!("UPDATE location SET {column} = ?2 WHERE id = ?1"),
            &[&location.0, &value],
        );
    }

    /// Set one `location` column to text.
    ///
    /// # Panics
    /// If the column does not exist or the value violates its CHECK.
    pub fn set_location_text(&self, location: LocationId, column: &str, value: &str) {
        self.exec(
            &format!("UPDATE location SET {column} = ?2 WHERE id = ?1"),
            &[&location.0, &value],
        );
    }

    /// Record one job as finished.
    ///
    /// # Panics
    /// If the write is refused.
    pub fn mark_job_done(&mut self, project: ProjectId, job: JobKind) {
        self.index
            .with_tx(|tx| put(tx, project, &JobStateRow::fresh(job, JobState::Done, 0)))
            .expect("mark job done");
    }

    /// Read one `project` column as an integer, `None` for SQL NULL.
    ///
    /// # Panics
    /// If the column does not exist.
    #[must_use]
    pub fn project_i64(&self, column: &str, project: ProjectId) -> Option<i64> {
        self.index
            .conn()
            .query_row(
                &format!("SELECT {column} FROM project WHERE id = ?1"),
                [project.0],
                |r| r.get::<_, Option<i64>>(0),
            )
            .expect("read project column")
    }

    /// Read one `project` column as text, `None` for SQL NULL.
    ///
    /// # Panics
    /// If the column does not exist.
    #[must_use]
    pub fn project_text(&self, column: &str, project: ProjectId) -> Option<String> {
        self.index
            .conn()
            .query_row(
                &format!("SELECT {column} FROM project WHERE id = ?1"),
                [project.0],
                |r| r.get::<_, Option<String>>(0),
            )
            .expect("read project column")
    }

    /// The directory the database lives in, for a test that measures its size.
    #[must_use]
    pub fn dir(&self) -> &Path {
        self.dir.path()
    }
}

impl Default for TempIndex {
    fn default() -> Self {
        Self::new()
    }
}
