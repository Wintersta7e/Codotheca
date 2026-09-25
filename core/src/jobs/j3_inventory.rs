//! J3 — tracked inventory (§4.1): how many files, how many bytes at HEAD, in what languages,
//! and what shape the project is.
//!
//! **§4.1's chunking is not reachable through plan 05's backend, and this module does not
//! pretend otherwise.** `GitBackend::tracked_inventory` is one atomic call — it runs `ls-files`
//! and then the deadlock-safe `cat-file --batch-check` internally — so there is no yield point
//! for a cursor to be written at. The chunk contract itself is built and tested
//! (`JobOutcome::Partial`, `apply_outcome`, `project_job_state.cursor`); what is missing is a
//! streaming inventory method on plan 05's trait for J3 to yield inside. Writing a cursor codec
//! that nothing ever writes would be the "declared for testability, never really implemented"
//! defect this project keeps finding, so it is left out and recorded instead.
//!
//! The practical consequence is bounded: J3's budget is §4.1's *slice*, not a deadline, so the
//! job runs to completion either way. What is lost is the ability to hand the store slot back
//! part-way through a very large tree.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::Transaction;

use super::classify::{archetype_of, language_of_path, primary_language};
use super::JobError;
use crate::git::{GitBackend, JobContext, RepoHandle};
use crate::index::IndexError;
use crate::protocol::{LocationId, ProjectId};

/// Paths retained for [`archetype_of`]. Every rule it applies is decided by the first few
/// thousand paths, and an unbounded sample would grow with the repository for no gain.
pub const ARCHETYPE_SAMPLE: usize = 4_000;

/// What one inventory run concluded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InventoryFacts {
    /// `project.tracked_files`.
    pub tracked_files: u64,
    /// `project.size_tracked_bytes` — HEAD blob bytes, not worktree bytes.
    pub head_bytes: u64,
    /// `project.language_bytes`, folded from extensions through [`language_of_path`].
    pub language_bytes: BTreeMap<String, u64>,
    /// A bounded sample of tracked paths, for the archetype.
    pub sample_paths: Vec<String>,
    /// §5.1's third input. Covers tracked files and the worktree root; a new untracked file at
    /// depth is not seen here, which is why §6's watch set exists beside it.
    pub worktree_newest_mtime: Option<i64>,
}

impl InventoryFacts {
    /// Add one path to the archetype sample, up to the cap.
    pub fn note_path(&mut self, path: &str) {
        if self.sample_paths.len() < ARCHETYPE_SAMPLE {
            self.sample_paths.push(path.to_owned());
        }
    }
}

fn mtime_secs(p: &Path) -> Option<i64> {
    let m = std::fs::metadata(p).ok()?.modified().ok()?;
    i64::try_from(m.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs()).ok()
}

/// Run one inventory.
///
/// `work_dir` is read for mtimes only; every git fact comes through the backend.
///
/// # Errors
///
/// `JobError::Git` carrying the backend's inventory failure; an unreadable mtime is skipped.
pub fn observe(
    git: &dyn GitBackend,
    repo: &RepoHandle,
    ctx: &JobContext<'_>,
    work_dir: &Path,
) -> Result<InventoryFacts, JobError> {
    let inv = git.tracked_inventory(repo, ctx)?;
    let mut facts = InventoryFacts {
        tracked_files: u64::from(inv.tracked_files),
        head_bytes: inv.size_tracked_bytes,
        ..InventoryFacts::default()
    };

    // Extension bytes are the git layer's census; the language fold is this layer's, because
    // `Lang` and its "markup can never be primary" rule belong to §1.2, not to §3.
    for (ext, bytes) in &inv.extension_bytes {
        // `path_extension` yields the bare extension; `language_of_path` wants a path.
        if let Some(lang) = language_of_path(&format!("f.{ext}")) {
            *facts
                .language_bytes
                .entry(lang.name.to_owned())
                .or_insert(0) += *bytes;
        }
    }

    for raw in &inv.paths {
        let path = String::from_utf8_lossy(raw);
        facts.note_path(&path);
        if let Some(t) = mtime_secs(&work_dir.join(path.as_ref())) {
            facts.worktree_newest_mtime = Some(facts.worktree_newest_mtime.map_or(t, |c| c.max(t)));
        }
    }
    if let Some(t) = mtime_secs(work_dir) {
        facts.worktree_newest_mtime = Some(facts.worktree_newest_mtime.map_or(t, |c| c.max(t)));
    }
    Ok(facts)
}

/// Write a completed inventory.
///
/// A half-counted total rendered as `41 GB tracked` is a wrong figure, and §8.2's `indexedCount`
/// counts completed inventories precisely so a partial one is absent rather than wrong.
///
/// # Errors
///
/// `IndexError::Sqlite` when either update fails.
pub fn commit_inventory(
    tx: &Transaction<'_>,
    project: ProjectId,
    location: LocationId,
    facts: &InventoryFacts,
) -> Result<(), IndexError> {
    let langs = serde_json::to_string(&facts.language_bytes).unwrap_or_else(|_| "{}".to_owned());
    tx.execute(
        "UPDATE project
            SET tracked_files = ?2, size_tracked_bytes = ?3, language_bytes = ?4,
                primary_language = ?5, archetype = ?6
          WHERE id = ?1",
        rusqlite::params![
            project.0,
            i64::try_from(facts.tracked_files).unwrap_or(i64::MAX),
            i64::try_from(facts.head_bytes).unwrap_or(i64::MAX),
            langs,
            primary_language(&facts.language_bytes),
            archetype_of(&facts.sample_paths),
        ],
    )?;
    tx.execute(
        "UPDATE location SET worktree_newest_mtime = ?2 WHERE id = ?1",
        rusqlite::params![location.0, facts.worktree_newest_mtime],
    )?;
    Ok(())
}

#[cfg(all(test, feature = "testkit"))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::cancel::CancelToken;
    use crate::git::{JobClass, StoreKey, TrackedInventory};
    use crate::mount::StoreClass;
    use crate::testing::{FakeGitBackend, GitReply};

    fn handle() -> RepoHandle {
        RepoHandle::bare(Path::new("/w/repo"), StoreKey::new("s"), StoreClass::Local)
    }

    fn inv(ext: &[(&str, u64)], paths: &[&str]) -> TrackedInventory {
        TrackedInventory {
            tracked_files: u32::try_from(paths.len()).unwrap(),
            size_tracked_bytes: ext.iter().map(|(_, b)| *b).sum(),
            extension_bytes: ext
                .iter()
                .map(|(e, b)| ((*e).to_owned(), *b))
                .collect::<BTreeMap<_, _>>(),
            paths: paths.iter().map(|p| p.as_bytes().to_vec()).collect(),
            observed_at: 10,
        }
    }

    fn run(inventory: TrackedInventory, work_dir: &Path) -> InventoryFacts {
        let git = FakeGitBackend::new();
        git.always_tracked_inventory(GitReply::Ok(inventory));
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);
        observe(&git, &handle(), &ctx, work_dir).unwrap()
    }

    #[test]
    fn extension_bytes_fold_into_language_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let facts = run(
            inv(
                &[("rs", 900), ("tsx", 100), ("bin", 5_000)],
                &["src/lib.rs"],
            ),
            dir.path(),
        );
        assert_eq!(facts.language_bytes.get("Rust").copied(), Some(900));
        assert_eq!(facts.language_bytes.get("TypeScript").copied(), Some(100));
        assert!(
            !facts.language_bytes.contains_key("bin"),
            "an unrecognised extension is not a language"
        );
        // The byte total is the git layer's and is not recomputed from the languages, so an
        // unrecognised extension still counts towards the size.
        assert_eq!(facts.head_bytes, 6_000);
    }

    #[test]
    fn two_extensions_of_one_language_are_summed() {
        let dir = tempfile::tempdir().unwrap();
        let facts = run(inv(&[("ts", 40), ("tsx", 60)], &["a.ts"]), dir.path());
        assert_eq!(facts.language_bytes.get("TypeScript").copied(), Some(100));
    }

    #[test]
    fn the_archetype_sample_is_bounded_so_memory_cannot_grow_with_the_repository() {
        let mut facts = InventoryFacts::default();
        for i in 0..(ARCHETYPE_SAMPLE + 500) {
            facts.note_path(&format!("f{i}.rs"));
        }
        assert_eq!(facts.sample_paths.len(), ARCHETYPE_SAMPLE);
    }

    /// The archetype needs basenames a histogram cannot carry, which is why `TrackedInventory`
    /// gained its `paths` field.
    #[test]
    fn the_path_set_reaches_the_archetype() {
        let dir = tempfile::tempdir().unwrap();
        let facts = run(inv(&[("go", 100)], &["Dockerfile", "main.go"]), dir.path());
        assert_eq!(archetype_of(&facts.sample_paths), "service");
    }

    #[test]
    fn the_worktree_mtime_is_the_newest_of_the_tracked_files_and_the_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), b"x").unwrap();
        let facts = run(inv(&[("rs", 1)], &["a.rs"]), dir.path());
        assert!(facts.worktree_newest_mtime.is_some());
    }

    /// An empty repository has no tracked files, and that is a measured zero rather than an
    /// absent mtime standing in for one.
    #[test]
    fn an_empty_index_yields_zero_files_and_no_language() {
        let dir = tempfile::tempdir().unwrap();
        let facts = run(inv(&[], &[]), dir.path());
        assert_eq!(facts.tracked_files, 0);
        assert!(facts.language_bytes.is_empty());
        assert_eq!(primary_language(&facts.language_bytes), None);
    }
}
