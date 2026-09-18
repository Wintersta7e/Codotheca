//! §32.6's lockfile read: **six filenames, depth ≤ 3, worktree basis, total or `not_read`.**
//!
//! **The basis is the worktree** (A9). A lockfile's meaning is *what is installed*, not what is
//! committed, so the file set is found by a bounded walk of the working copy rather than by riding
//! §29's HEAD-basis enumeration — a lockfile that is uncommitted or deliberately gitignored is
//! installed and would be invisible there, and a mixed-basis composite is exactly what A9 forbids.
//!
//! **Not compute-suppressed** (A11.2). `compute_suppressed` gates §29's unbounded whole-tree work;
//! this read is bounded, so a suppressed project's verdict is computed and simply not surfaced.

use std::path::Path;

use rusqlite::Transaction;

use crate::advisories::parse::{parse_lockfile, LockfileRead};
use crate::advisories::store::{clear_project_read, write_lockfile_row, write_scan_row};
use crate::advisories::AdvisoryError;
use crate::protocol::{DependencyReadState, Ecosystem, ProjectId};

/// The largest lockfile this read will open.
///
/// **J6's own 256 KB is unchanged** and still governs J6's named-file reads; this read carries its
/// own because 256 KB is refuted by a measurement on this very tree — its `package-lock.json` is
/// **287,417 bytes**, which J6's cap would have truncated. A lockfile is machine-generated and
/// grows with the dependency graph, not with anything a human wrote.
pub const LOCKFILE_BYTE_CAP: u64 = 16 * 1024 * 1024;

/// The largest number of lockfiles this read will open for one project.
///
/// **Two bounds and not three**: each file is parsed and its triples written before the next is
/// opened, so the peak memory is one file and [`LOCKFILE_BYTE_CAP`] already governs it. A third
/// number would be a third thing to drift.
pub const LOCKFILE_COUNT_CAP: usize = 32;

/// How far below the repository root the walk descends, **counting the file itself**: a lockfile
/// at the repository root is depth 1.
///
/// **Three and not one.** A read restricted to the repository root misses a monorepo's per-package
/// lockfiles and produces a **partial triple set**, whose verdict is a lit tick claiming *no known
/// vulnerable dependencies* over a read that never looked.
pub const LOCKFILE_MAX_DEPTH: usize = 3;

/// The six names and the ecosystem each resolves to, **in one place**.
///
/// Six filenames, three ecosystems, **five parsers**: a plan that budgets "three ecosystems" as
/// three parsers under-counts by two, and `poetry.lock` and `uv.lock` share one only because they
/// share a format.
pub const LOCKFILE_NAMES: [(&str, Ecosystem); 6] = [
    ("package-lock.json", Ecosystem::Npm),
    ("yarn.lock", Ecosystem::Npm),
    ("pnpm-lock.yaml", Ecosystem::Npm),
    ("Cargo.lock", Ecosystem::Rust),
    ("poetry.lock", Ecosystem::Pip),
    ("uv.lock", Ecosystem::Pip),
];

/// Manifest names, and the ecosystem each declares. **Presence only: no manifest is opened.**
///
/// A manifest with no lockfile means *this project declares dependencies whose versions are not
/// resolved*, and version matching is server-side and needs a version — so the verdict is
/// `unknown`. Without this list it would be **`clean`**, which is the false clean §32 exists to
/// prevent, and nothing else in this tree records that a project declares dependencies: J6 reads
/// three of these files but stores only a *description*, which a manifest without one never
/// produces.
///
/// `None` is an ecosystem this build ships no parser for. Such a project can never resolve to
/// triples, so its verdict is `unknown` whatever else the walk found — §32.8's unshipped-ecosystem
/// row, which would otherwise read `clean` for every Go, Ruby, PHP, Java and .NET project in the
/// library.
pub const MANIFEST_NAMES: [(&str, Option<Ecosystem>); 11] = [
    ("package.json", Some(Ecosystem::Npm)),
    ("Cargo.toml", Some(Ecosystem::Rust)),
    ("pyproject.toml", Some(Ecosystem::Pip)),
    ("requirements.txt", Some(Ecosystem::Pip)),
    ("Pipfile", Some(Ecosystem::Pip)),
    ("go.mod", None),
    ("Gemfile", None),
    ("composer.json", None),
    ("pom.xml", None),
    ("build.gradle", None),
    ("build.gradle.kts", None),
];

/// Directories the walk never enters.
///
/// `.git` is not a source tree. **`node_modules` holds installed dependencies' own lockfiles**,
/// which declare somebody else's dependency graph — counting them would attribute another
/// project's triples to this one and would make the count depend on whether `npm install` had run.
const SKIPPED_DIRS: [&str; 2] = [".git", "node_modules"];

/// One lockfile the walk found. `size_bytes` is diagnostic and is what `AC-P3-32-16` prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockfileHit {
    pub source_path: String,
    pub ecosystem: Ecosystem,
    pub size_bytes: u64,
}

/// What one walk saw. **`complete` is false when [`LOCKFILE_COUNT_CAP`] was reached**, which is a
/// bound and not a failure — but it is one the verdict has to know about, because a bounded read
/// of an unbounded tree cannot claim to have seen everything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockfileWalk {
    pub files: Vec<LockfileHit>,
    pub dirs_entered: usize,
    pub complete: bool,
    /// How many manifests were seen whose ecosystem produced **no** lockfile — including every
    /// manifest of an ecosystem this build ships no parser for.
    ///
    /// **Non-zero means `unknown`**, never `clean`: the project declares dependencies this read
    /// cannot resolve to versions, and a verdict over a set that was never assembled is the false
    /// clean this section exists to prevent.
    pub unresolved_manifests: usize,
}

/// Walk `root` to [`LOCKFILE_MAX_DEPTH`], matching only [`LOCKFILE_NAMES`].
///
/// A root that cannot be read yields an **empty, incomplete** walk rather than an error: the
/// caller's job is to record that nothing could be seen, and `complete: false` is how it says so.
#[must_use]
pub fn walk_lockfiles(root: &Path) -> LockfileWalk {
    let mut files = Vec::new();
    let mut manifests: Vec<Option<Ecosystem>> = Vec::new();
    let mut dirs_entered = 0usize;
    let mut complete = true;
    let mut queue: Vec<(std::path::PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((dir, depth)) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            // A directory this process may not read is a directory it did not see. The walk stays
            // bounded and says it was not complete.
            complete = false;
            continue;
        };
        dirs_entered += 1;
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                complete = false;
                continue;
            };
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if kind.is_dir() {
                if SKIPPED_DIRS.contains(&name) {
                    continue;
                }
                // The *file* depth of anything inside this directory is `depth + 2`, so a
                // directory whose children could only sit past the bound is not entered at all.
                if depth + 2 <= LOCKFILE_MAX_DEPTH {
                    queue.push((entry.path(), depth + 1));
                }
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            if let Some((_, declared)) = MANIFEST_NAMES.iter().find(|(n, _)| *n == name) {
                manifests.push(*declared);
            }
            let Some((_, ecosystem)) = LOCKFILE_NAMES.iter().find(|(n, _)| *n == name) else {
                continue;
            };
            if files.len() >= LOCKFILE_COUNT_CAP {
                complete = false;
                continue;
            }
            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or_default();
            let Some(source_path) = relative_display(root, &entry.path()) else {
                complete = false;
                continue;
            };
            files.push(LockfileHit {
                source_path,
                ecosystem: *ecosystem,
                size_bytes,
            });
        }
    }
    // The walk order is a stack's, which is not stable across filesystems; the stored rows are
    // keyed by path, so a deterministic order is what makes two runs comparable.
    files.sort_by(|a, b| a.source_path.cmp(&b.source_path));
    // A manifest is *resolved* only when a lockfile of its own ecosystem was matched. An
    // unshipped ecosystem has none by construction, so every one of those counts.
    let unresolved_manifests = manifests
        .iter()
        .filter(|declared| match declared {
            None => true,
            Some(eco) => !files.iter().any(|f| f.ecosystem == *eco),
        })
        .count();
    LockfileWalk {
        files,
        dirs_entered,
        complete,
        unresolved_manifests,
    }
}

/// The path below `root`, with forward slashes, so a stored `source_path` reads the same on both
/// platforms and no absolute path is ever written to the index.
fn relative_display(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut out = String::new();
    for part in rel.components() {
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(part.as_os_str().to_str()?);
    }
    Some(out)
}

/// Read one project's lockfiles and write what was seen. Returns the **triple count written**.
///
/// **A cap exceedance is `not_read`, never `absent`, and never a partial parse.** A project with
/// any `not_read` lockfile has an `unknown` verdict, whatever its other lockfiles said. *A timeout
/// looks exactly like a missing file*, and here a truncated read also looks exactly like a shorter
/// dependency list — so a producer must distinguish *absent* from *unreadable* before it may open
/// an item, and `read_capped` returning `None` is the live instance that would have opened a
/// `missing_readme` item on a repository that has one.
///
/// **Three read outcomes are distinguished and stored, never two.** `parsed` and `not_read` are
/// `project_lockfile.read_state`; *the scan has not run* is the **absence** of a
/// `project_dependency_scan` row. Collapsing that last one makes every unscanned project claim to
/// have no dependencies.
///
/// # Errors
/// Fails when SQLite refuses a write.
pub fn read_lockfiles(
    tx: &Transaction<'_>,
    project: ProjectId,
    root: &Path,
    now: i64,
) -> Result<usize, AdvisoryError> {
    let walk = walk_lockfiles(root);
    // A re-read replaces: a lockfile that has been deleted must not leave its triples behind,
    // which would be a dependency this project no longer resolves, dated as if it did.
    clear_project_read(tx, project)?;

    let mut written = 0usize;
    for hit in &walk.files {
        let read = if hit.size_bytes > LOCKFILE_BYTE_CAP {
            LockfileRead::NotRead
        } else {
            match std::fs::read(root.join(&hit.source_path)) {
                Ok(bytes) => {
                    let name = hit.source_path.rsplit('/').next().unwrap_or("");
                    parse_lockfile(name, &bytes)
                }
                // Unreadable is **not** evidence of absence.
                Err(_) => LockfileRead::NotRead,
            }
        };
        let state = match &read {
            LockfileRead::Parsed(_) => DependencyReadState::Parsed,
            LockfileRead::NotRead => DependencyReadState::NotRead,
        };
        write_lockfile_row(tx, project, hit, state, now)?;
        if let LockfileRead::Parsed(pairs) = read {
            written += crate::advisories::store::write_triples(
                tx,
                project,
                &hit.source_path,
                hit.ecosystem,
                &pairs,
                now,
            )?;
        }
    }

    write_scan_row(tx, project, &walk, now)?;
    Ok(written)
}
