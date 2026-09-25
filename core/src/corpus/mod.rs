//! The corpus generator (§15.1).
//!
//! A deterministic, re-runnable set of repositories covering the shapes the scanner, the
//! derived values and the freshness model have to survive. Fixtures are described by shape
//! and nothing here names any real project.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub mod fixtures;
mod git_cmd;
mod manifest;

pub use git_cmd::{write_file, CorpusGit, CORPUS_AUTHOR, CORPUS_EMAIL};
pub use manifest::{CorpusFixture, CorpusManifest, CorpusVolume, FixtureExpect};

use crate::mount::StoreClass;

/// Bumped whenever a fixture's shape changes. A manifest from an older version is rejected
/// rather than half-trusted.
pub const CORPUS_VERSION: u32 = 1;

/// Every ordinary corpus commit is dated from here. Fixed, so commit OIDs are fixed.
pub const BASE_UNIX: i64 = 1_700_000_000;
/// The future-dated fixture's timestamp: 2100-01-01T00:00:00Z.
pub const FUTURE_UNIX: i64 = 4_102_444_800;

/// The fixed local volume's id, and its directory under the corpus root.
pub const VOLUME_A: &str = "vol-a";
/// The simulated removable volume's id, and its directory under the corpus root.
pub const VOLUME_B: &str = "vol-b";

/// Why building or reading the corpus failed.
#[derive(Debug)]
pub enum CorpusError {
    /// A filesystem call failed.
    Io {
        /// The path it failed on.
        path: PathBuf,
        /// The OS error's text.
        message: String,
    },
    /// A git invocation could not be spawned or waited on, or exited non-zero.
    Git {
        /// The arguments after the pinned `-c` prefix.
        args: Vec<String>,
        /// The exit code; `None` when git never ran or a signal ended it.
        code: Option<i32>,
        /// What git wrote to stderr, or the spawn error's text.
        stderr: String,
    },
    /// The installed git is below the 2.22 floor (§3.1).
    GitTooOld {
        /// The version it reported.
        found: String,
    },
    /// A fixture name no builder handles.
    UnknownFixture(String),
    /// The manifest could not be read, written or trusted, or a shared corpus never appeared.
    Manifest(String),
    /// A fixture the manifest lacks, or holds unbuilt — then with the reason.
    MissingFixture(String),
}

impl fmt::Display for CorpusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "io at {}: {message}", path.display()),
            Self::Git { args, code, stderr } => {
                write!(f, "git {args:?} exited {code:?}: {}", stderr.trim())
            }
            Self::GitTooOld { found } => write!(f, "git {found} is below the 2.22 floor"),
            Self::UnknownFixture(n) => write!(f, "unknown fixture: {n}"),
            Self::Manifest(m) => write!(f, "manifest: {m}"),
            Self::MissingFixture(m) => write!(f, "fixture unavailable: {m}"),
        }
    }
}

impl std::error::Error for CorpusError {}

/// What to build, where, and how big.
#[derive(Debug, Clone)]
pub struct CorpusOptions {
    /// Where the corpus goes; a relative root is resolved against the working directory.
    pub root: PathBuf,
    /// `None` builds everything. `Some(list)` builds exactly those plus their dependencies;
    /// `Some(empty)` builds the volumes and nothing else.
    pub only: Option<Vec<String>>,
    /// Rebuild even when the manifest on disk already covers the selection.
    pub force: bool,
    /// Build the fixtures that cost real time and disk — currently `deep-history`.
    pub large: bool,
    /// How many untracked files the huge-untracked fixture writes.
    pub untracked_files: u32,
    /// How many commits the deep-history fixture imports.
    pub deep_history_commits: u32,
}

impl CorpusOptions {
    /// Every fixture, at `root`, with `force` and the large fixtures off.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            only: None,
            force: false,
            large: false,
            untracked_files: 5_000,
            deep_history_commits: 50_000,
        }
    }
}

/// A `file://` URL for a local path, so `--depth` clones use a real transport.
#[must_use]
pub fn file_url(path: &Path) -> String {
    let mut text = path
        .to_string_lossy()
        .replace('\\', "/")
        .replace(' ', "%20");
    if !text.starts_with('/') {
        text.insert(0, '/');
    }
    format!("file://{text}")
}

fn mkdir(path: &Path) -> Result<(), CorpusError> {
    std::fs::create_dir_all(path).map_err(|e| CorpusError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

/// Resolve a root against the process's working directory.
///
/// The corpus root must be absolute. Every builder sets git's `cwd` to one directory and hands
/// it a path as an argument, and git reads that argument relative to `cwd` — so a relative root
/// makes `git init <root>/vol-a/upstream` create `<root>/vol-a/<root>/vol-a/upstream` and the
/// next command finds no repository. `--out corpus-check` is exactly that case.
///
/// Joined, not canonicalised: `std::fs::canonicalize` returns the `\\?\` extended-length form
/// on Windows, which would end up in the manifest and in every git argument.
fn absolute(path: &Path) -> Result<PathBuf, CorpusError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(|e| CorpusError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    Ok(cwd.join(path))
}

/// Build the corpus at `options.root`, replacing whatever is there.
///
/// # Errors
/// `CorpusError::Io` when the root cannot be cleared or a directory created, `Git` when git
/// cannot report its version, `GitTooOld` below the 2.22 floor, and whatever a fixture builder
/// or the manifest write returns.
pub fn generate(options: &CorpusOptions) -> Result<CorpusManifest, CorpusError> {
    let root = &absolute(&options.root)?;
    if root.exists() {
        std::fs::remove_dir_all(root).map_err(|e| CorpusError::Io {
            path: root.clone(),
            message: e.to_string(),
        })?;
    }
    mkdir(root)?;

    // Held as their own bindings rather than indexed out of the vector: the crate denies
    // `clippy::indexing_slicing`, and these two are what every builder is handed.
    let vol_a_path = root.join(VOLUME_A);
    let vol_b_path = root.join(VOLUME_B);
    let volumes = vec![
        CorpusVolume {
            id: VOLUME_A.to_owned(),
            path: vol_a_path.clone(),
            store_key: "corpus-store-a".to_owned(),
            volume_key: "corpus-volume-a".to_owned(),
            class: StoreClass::Local,
        },
        CorpusVolume {
            id: VOLUME_B.to_owned(),
            path: vol_b_path.clone(),
            store_key: "corpus-store-b".to_owned(),
            volume_key: "corpus-volume-b".to_owned(),
            class: StoreClass::Removable,
        },
    ];
    mkdir(&vol_a_path)?;
    mkdir(&vol_b_path)?;
    let sources = root.join("sources");
    mkdir(&sources)?;

    let git = CorpusGit::new(root.join("githome"))?;
    let git_version = git.version()?;
    check_floor(&git_version)?;

    let wanted = resolve_wanted(options.only.as_deref());
    let ctx = fixtures::Ctx {
        git: &git,
        vol_a: &vol_a_path,
        vol_b: &vol_b_path,
        sources: &sources,
        options,
    };
    let mut built: Vec<CorpusFixture> = Vec::new();
    for name in &wanted {
        built.extend(fixtures::build(&ctx, name)?);
    }

    let manifest = CorpusManifest {
        corpus_version: CORPUS_VERSION,
        git_version,
        root: root.clone(),
        volumes,
        fixtures: built,
    };
    manifest.save(root)?;
    Ok(manifest)
}

/// `2.22` is the floor (§3.1). Anything below it cannot run the corpus, let alone the app.
fn check_floor(version: &str) -> Result<(), CorpusError> {
    let mut parts = version.split(['.', '-', ' ']);
    let major = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    if (major, minor) < (2, 22) {
        return Err(CorpusError::GitTooOld {
            found: version.to_owned(),
        });
    }
    Ok(())
}

/// Expand a selection with its dependencies and put it in `ORDER`. `None` means everything.
fn resolve_wanted(only: Option<&[String]>) -> Vec<String> {
    let selected: Vec<String> = only.map_or_else(
        || fixtures::ORDER.iter().map(|n| (*n).to_owned()).collect(),
        |list| {
            let mut wanted: Vec<String> = Vec::new();
            let mut queue: Vec<String> = list.to_vec();
            while let Some(name) = queue.pop() {
                if wanted.contains(&name) {
                    continue;
                }
                for dependency in fixtures::dependencies(&name) {
                    queue.push((*dependency).to_owned());
                }
                wanted.push(name);
            }
            wanted
        },
    );
    fixtures::ORDER
        .iter()
        .filter(|name| selected.iter().any(|s| s == *name))
        .map(|n| (*n).to_owned())
        .collect()
}

/// Build the corpus only if what is on disk does not already match.
///
/// Reuse is what makes the corpus usable from a test: generating twenty repositories per test
/// binary would dominate the suite. `force` rebuilds unconditionally.
///
/// # Errors
/// Those of [`generate`], whenever a rebuild is needed.
pub fn ensure(options: &CorpusOptions) -> Result<CorpusManifest, CorpusError> {
    if !options.force {
        if let Ok(existing) = CorpusManifest::load(&options.root) {
            let wanted = resolve_wanted(options.only.as_deref());
            let have_all = wanted.iter().all(|name| existing.fixture(name).is_some());
            if have_all {
                return Ok(existing);
            }
        }
    }
    generate(options)
}

/// How long a waiter gives the process holding the lock: 600 × 500 ms is five minutes, far
/// longer than a full build and short enough to fail rather than hang.
const WAIT_ATTEMPTS: u32 = 600;
const WAIT_DELAY: Duration = Duration::from_millis(500);

/// The corpus every integration test shares.
///
/// `CODOTHECA_CORPUS_DIR` lets CI build it once and hand the path to every job. Otherwise it
/// lands in the system temp directory under a version-stamped name, so a version bump never
/// reuses an incompatible tree.
///
/// # Errors
/// `CorpusError::Io` when the lock's directory cannot be created, those of [`generate`] for the
/// caller that takes the lock and builds, and `CorpusError::Manifest` for a waiter whose wait ran
/// out.
pub fn shared_corpus() -> Result<CorpusManifest, CorpusError> {
    let root = std::env::var_os("CODOTHECA_CORPUS_DIR").map_or_else(
        || std::env::temp_dir().join(format!("codotheca-corpus-v{CORPUS_VERSION}")),
        PathBuf::from,
    );
    shared_corpus_at(&root, WAIT_ATTEMPTS, WAIT_DELAY)
}

fn shared_corpus_at(
    root: &Path,
    attempts: u32,
    delay: Duration,
) -> Result<CorpusManifest, CorpusError> {
    // Test binaries run in parallel. The first to create the lock builds; the rest wait for
    // the manifest to appear rather than racing to build the same repositories.
    let lock = root.with_extension("lock");
    if let Some(parent) = lock.parent() {
        mkdir(parent)?;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
    {
        Ok(_) => {
            let result = ensure(&CorpusOptions::new(root));
            let _ = std::fs::remove_file(&lock);
            result
        }
        Err(_) => wait_for_corpus(root, &lock, attempts, delay),
    }
}

/// Poll for the manifest the lock holder is writing.
///
/// On timeout the lock is removed and named in the error. A run killed mid-build leaves one
/// behind with no manifest beside it, and a lock nobody owns would otherwise make every later
/// run wait the whole budget and fail with nothing to act on.
fn wait_for_corpus(
    root: &Path,
    lock: &Path,
    attempts: u32,
    delay: Duration,
) -> Result<CorpusManifest, CorpusError> {
    for _ in 0..attempts {
        if let Ok(manifest) = CorpusManifest::load(root) {
            return Ok(manifest);
        }
        std::thread::sleep(delay);
    }
    let _ = std::fs::remove_file(lock);
    Err(CorpusError::Manifest(format!(
        "timed out waiting for a shared corpus at {}; removed the stale lock {}, so a rerun builds it",
        root.display(),
        lock.display()
    )))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;

    fn empty_manifest(root: &Path) -> CorpusManifest {
        CorpusManifest {
            corpus_version: CORPUS_VERSION,
            git_version: "2.99.0".to_owned(),
            root: root.to_path_buf(),
            volumes: Vec::new(),
            fixtures: Vec::new(),
        }
    }

    #[test]
    fn a_waiter_returns_the_manifest_the_holder_wrote() {
        let root = std::env::temp_dir().join("codotheca-corpus-unit-waiter");
        let _ = std::fs::remove_dir_all(&root);
        mkdir(&root).unwrap();
        empty_manifest(&root).save(&root).unwrap();
        let lock = root.with_extension("lock");
        let got = wait_for_corpus(&root, &lock, 2, Duration::from_millis(1)).unwrap();
        assert_eq!(got.corpus_version, CORPUS_VERSION);
    }

    #[test]
    fn a_wait_that_times_out_clears_the_lock_and_names_it() {
        let root = std::env::temp_dir().join("codotheca-corpus-unit-stale");
        let _ = std::fs::remove_dir_all(&root);
        let lock = root.with_extension("lock");
        mkdir(&root).unwrap();
        std::fs::write(&lock, b"").unwrap();

        let err = wait_for_corpus(&root, &lock, 2, Duration::from_millis(1)).unwrap_err();
        let text = err.to_string();
        assert!(text.contains(&lock.display().to_string()), "got {text}");
        assert!(
            !lock.exists(),
            "a lock nobody owns must not outlive the wait that gave up on it"
        );
    }
}
