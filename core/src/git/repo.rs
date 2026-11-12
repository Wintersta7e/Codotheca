//! Where a repository's three directories are, and how it is scheduled.

use std::path::{Path, PathBuf};

// R4: the store classification is plan 06's, and it is the only one.
use crate::mount::StoreClass;

use super::error::{GitError, GitResult};

/// The runtime scheduler key: the actual mounted device or share (§4.7). Supplied by the
/// `MountResolver` seam; this module only groups by it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StoreKey(String);

impl StoreKey {
    /// Wrap a resolver-produced key.
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// The key as text, which is what `location.store_key` stores.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// [superseded by R4] `pub enum StoreClass { Fast, Slow }` was declared here. How many
// concurrent git processes one store tolerates is `core::mount::StoreClass::per_store_cap()`
// (plan 06) — `Local`/`Unknown` cap 4, `Network`/`Hdd`/`Fuse`/`Removable` cap 1. Two spellings
// of §3.4's rule is one place for it to drift; this module now only groups by the class.

/// One repository, resolved.
#[derive(Debug, Clone)]
pub struct RepoHandle {
    /// The directory git runs in (`-C`). For a bare repository this is the git dir.
    pub work_dir: PathBuf,
    /// This checkout's git dir. A linked worktree has its own.
    pub git_dir: PathBuf,
    /// The shared git dir: refs, packed-refs, config, objects.
    pub common_dir: PathBuf,
    /// Scheduling key.
    pub store: StoreKey,
    /// Scheduling class.
    pub store_class: StoreClass,
    /// True when `location.trusted_at` is non-NULL, which adds `-c safe.directory=<path>`.
    pub trusted: bool,
    /// How invocations select the repository: by discovery, or — for a WSL-spelled pointer —
    /// by naming it.
    pub git_dir_select: GitDirSelect,
}

/// How invocations select a repository (`base_args`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitDirSelect {
    /// `-C <work_dir>`, and git discovers the repository itself, its ownership check
    /// (`safe.directory`) included. Every handle but a translated one.
    Discover,
    /// The `.git` file names its git dir in WSL's `/mnt/<d>/` spelling, which Windows git cannot
    /// follow ([`resolve_gitdir_pointer`]), and nothing has established that this user owns it.
    /// Selected by `-C` like `Discover`, so git refuses the pointer and the read fails closed —
    /// unless the location is trusted ([`RepoHandle::selects_explicitly`]).
    Translated,
    /// Translated, and ownership established ([`RepoHandle::ownership_established`]): selected
    /// by `--git-dir` and `--work-tree`.
    Established,
}

impl RepoHandle {
    /// Resolve a working tree. Reads `.git` (directory or `gitdir:` file) and `commondir`;
    /// spawns nothing.
    ///
    /// # Errors
    ///
    /// `GitError::PathGone` when `.git` is not there, `GitError::PermissionDenied` when its stat
    /// is refused, `GitError::Stale` for any other stat or read failure, and
    /// `GitError::Unreadable` when a `.git` file names no `gitdir:`.
    pub fn resolve(work_dir: &Path, store: StoreKey, store_class: StoreClass) -> GitResult<Self> {
        let dot_git = work_dir.join(".git");
        let meta = std::fs::metadata(&dot_git).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => GitError::PathGone {
                detail: e.to_string(),
            },
            std::io::ErrorKind::PermissionDenied => GitError::PermissionDenied {
                detail: e.to_string(),
            },
            _ => GitError::Stale {
                detail: e.to_string(),
            },
        })?;

        let (git_dir, git_dir_select) = if meta.is_dir() {
            (dot_git, GitDirSelect::Discover)
        } else {
            let text = std::fs::read_to_string(&dot_git).map_err(|e| GitError::Stale {
                detail: e.to_string(),
            })?;
            let target = gitdir_line(&text).ok_or_else(|| GitError::Unreadable {
                detail: "`.git` file carries no gitdir:".to_owned(),
            })?;
            let (git_dir, translated) = resolve_gitdir_pointer(work_dir, Path::new(target));
            let select = if translated {
                GitDirSelect::Translated
            } else {
                GitDirSelect::Discover
            };
            (git_dir, select)
        };

        let common_dir = std::fs::read_to_string(git_dir.join("commondir")).map_or_else(
            |_| git_dir.clone(),
            |text| {
                let rel = text.trim();
                let p = PathBuf::from(rel);
                if p.is_absolute() {
                    p
                } else {
                    git_dir.join(p)
                }
            },
        );

        Ok(Self {
            work_dir: work_dir.to_path_buf(),
            git_dir,
            common_dir,
            store,
            store_class,
            trusted: false,
            git_dir_select,
        })
    }

    /// A bare repository: `git_dir` is also the working directory (§4.2).
    #[must_use]
    pub fn bare(git_dir: &Path, store: StoreKey, store_class: StoreClass) -> Self {
        Self {
            work_dir: git_dir.to_path_buf(),
            git_dir: git_dir.to_path_buf(),
            common_dir: git_dir.to_path_buf(),
            store,
            store_class,
            trusted: false,
            git_dir_select: GitDirSelect::Discover,
        }
    }

    /// Set the trust flag from `location.trusted_at`.
    #[must_use]
    pub const fn with_trust(mut self, trusted: bool) -> Self {
        self.trusted = trusted;
        self
    }

    /// True when this checkout does not own its refs — §1.3's `is_worktree`.
    #[must_use]
    pub fn is_linked_worktree(&self) -> bool {
        self.git_dir != self.common_dir
    }

    /// True when invocations name `--git-dir` and `--work-tree` rather than let git discover
    /// the repository.
    ///
    /// **Explicit selection skips git's ownership check**, which runs only on discovery — and
    /// that check is all that stands between a foreign-owned repository's config (a
    /// `filter.<x>.clean`, run by `status`) and a read. So a translated pointer is selected
    /// explicitly only once its location is trusted or its ownership is established.
    #[must_use]
    pub const fn selects_explicitly(&self) -> bool {
        match self.git_dir_select {
            GitDirSelect::Discover => false,
            GitDirSelect::Translated => self.trusted,
            GitDirSelect::Established => true,
        }
    }

    /// Record that this user's ownership of a translated repository is established. Reachable
    /// only from `git::ownership`, whose probes ran in the same resolution against exactly this
    /// handle's directories; no row, candidate or earlier scan stands in for them. A handle that
    /// is not translated is unchanged.
    #[must_use]
    pub(super) const fn ownership_established(mut self) -> Self {
        if matches!(self.git_dir_select, GitDirSelect::Translated) {
            self.git_dir_select = GitDirSelect::Established;
        }
        self
    }

    /// For a handle built from what the walk or the index recorded rather than by
    /// [`Self::resolve`]: re-read the checkout's `.git` file and, where
    /// [`resolve_gitdir_pointer`] translates its pointer, take the translation as the git dir.
    /// Spawns nothing. Off Windows it reads nothing, and a work dir that is not absolute on this
    /// host (an in-distro path) is left alone.
    #[must_use]
    pub fn follow_wsl_pointer(self) -> Self {
        self.follow_pointer(translates_here())
    }

    fn follow_pointer(mut self, translate: bool) -> Self {
        if !translate
            || self.git_dir_select != GitDirSelect::Discover
            || !self.work_dir.is_absolute()
        {
            return self;
        }
        let Ok(text) = std::fs::read_to_string(self.work_dir.join(".git")) else {
            return self;
        };
        if let Some(target) = gitdir_line(&text) {
            let (git_dir, translated) =
                resolve_pointer(&self.work_dir, Path::new(target), translate);
            if translated {
                self.git_dir = git_dir;
                self.git_dir_select = GitDirSelect::Translated;
            }
        }
        self
    }
}

/// The value of a `.git` file's `gitdir:` line, trimmed.
fn gitdir_line(text: &str) -> Option<&str> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("gitdir:"))
        .map(str::trim)
}

/// WSL's spelling of a Windows drive, `/mnt/<letter>/<rest>`, as Windows spells it:
/// `<LETTER>:/<rest>`. `None` for anything else.
///
/// Exactly that shape — one ASCII letter, then a separator. `/mnt/cc/…`, `/mnt/c` alone,
/// `/mnt/`, a relative pointer, `\\wsl$\…` and every other mount root stay as they are: a
/// worktree created from WSL records its pointer the way the distro sees the drive, and this is
/// the spelling WSL mounts a drive at by default. Pure, so every host tests it; only Windows
/// applies it ([`resolve_gitdir_pointer`]).
#[must_use]
pub fn wsl_drive_path(pointer: &str) -> Option<PathBuf> {
    let rest = pointer.strip_prefix("/mnt/")?;
    let mut chars = rest.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    let tail = chars.as_str().strip_prefix('/')?;
    Some(PathBuf::from(format!(
        "{}:/{tail}",
        letter.to_ascii_uppercase()
    )))
}

/// Where a `gitdir:` pointer read from `<dir>/.git` leads, and whether it was translated.
///
/// [`wsl_drive_path`]'s translation applies only on Windows: Windows git refuses the
/// untranslated pointer (*"gitfile does not point to a valid repository"*), and on Linux the
/// `/mnt/<d>/` path is the real one.
///
/// A relative pointer is relative to `dir`. The one reading of a pointer: the walk's
/// classification and [`RepoHandle::resolve`] both call it.
#[must_use]
pub fn resolve_gitdir_pointer(dir: &Path, pointer: &Path) -> (PathBuf, bool) {
    resolve_pointer(dir, pointer, translates_here())
}

fn resolve_pointer(dir: &Path, pointer: &Path, translate: bool) -> (PathBuf, bool) {
    if translate {
        if let Some(translated) = pointer.to_str().and_then(host_drive_path) {
            return (translated, true);
        }
    }
    if pointer.is_absolute() {
        (pointer.to_path_buf(), false)
    } else {
        (dir.join(pointer), false)
    }
}

/// Whether this host translates a WSL-spelled pointer: Windows does, and — under `testkit`,
/// once a test has asked ([`translate_wsl_pointers_for_tests`]) — so does any host.
// cfg twin: without `testkit` the body is `const`-able, with it the atomic load is not.
#[allow(clippy::missing_const_for_fn)]
fn translates_here() -> bool {
    #[cfg(feature = "testkit")]
    if TESTKIT_TRANSLATES.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    cfg!(windows)
}

/// [`wsl_drive_path`] on Windows. Off Windows, only a test translates, and `/mnt/<d>/<rest>`
/// becomes `/<rest>`: the drive is this host's root, so a real repository is reachable through
/// the translated spelling and unreachable through the WSL one, as on Windows.
fn host_drive_path(pointer: &str) -> Option<PathBuf> {
    let translated = wsl_drive_path(pointer)?;
    if cfg!(windows) {
        return Some(translated);
    }
    let (_, rest) = pointer.strip_prefix("/mnt/")?.split_once('/')?;
    Some(Path::new("/").join(rest))
}

#[cfg(feature = "testkit")]
static TESTKIT_TRANSLATES: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Translate WSL-spelled pointers on this host for the rest of the process.
///
/// So the sites that read one run on a Linux gate too ([`host_drive_path`]). Process-wide: a test
/// binary that calls it translates in every test it runs.
#[cfg(feature = "testkit")]
pub fn translate_wsl_pointers_for_tests() {
    TESTKIT_TRANSLATES.store(true, std::sync::atomic::Ordering::Relaxed);
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

    #[test]
    fn a_wsl_drive_pointer_translates_to_its_drive() {
        assert_eq!(
            wsl_drive_path("/mnt/d/work/main-repo/.git/worktrees/lane-a"),
            Some(PathBuf::from("D:/work/main-repo/.git/worktrees/lane-a"))
        );
        assert_eq!(wsl_drive_path("/mnt/E/x"), Some(PathBuf::from("E:/x")));
    }

    #[test]
    fn nothing_but_the_exact_shape_translates() {
        for pointer in [
            "/mnt/cc/x",
            "/mnt/",
            "/mnt/c",
            "/mnt",
            "mnt/c/x",
            "../main-repo/.git/worktrees/lane-a",
            "/mnt/1/x",
            "/mnt/é/x",
            "/mnt/wsl/x",
            "/home/u/main-repo/.git",
            r"\\wsl$\distro\mnt\c\x",
            "D:/work/main-repo/.git",
        ] {
            assert_eq!(wsl_drive_path(pointer), None, "{pointer}");
        }
    }

    #[test]
    fn a_pointer_is_translated_only_where_translation_applies() {
        let dir = Path::new("/w/lane-a");
        let wsl = Path::new("/mnt/d/work/main-repo/.git/worktrees/lane-a");
        assert_eq!(resolve_pointer(dir, wsl, true), (translated_lane(), true));
        assert!(!resolve_pointer(dir, wsl, false).1);
        let relative = Path::new("../main-repo/.git/worktrees/lane-a");
        assert_eq!(
            resolve_pointer(dir, relative, true),
            (dir.join(relative), false)
        );
        assert_eq!(
            resolve_gitdir_pointer(dir, wsl).1,
            cfg!(windows),
            "only Windows applies the translation"
        );
    }

    /// What `/mnt/d/work/main-repo/.git/worktrees/lane-a` translates to on this host: the drive
    /// on Windows, the root under a test's switch elsewhere ([`host_drive_path`]).
    fn translated_lane() -> PathBuf {
        PathBuf::from(if cfg!(windows) {
            "D:/work/main-repo/.git/worktrees/lane-a"
        } else {
            "/work/main-repo/.git/worktrees/lane-a"
        })
    }

    fn handle(select: GitDirSelect, trusted: bool) -> RepoHandle {
        RepoHandle {
            work_dir: PathBuf::from("/w/lane-a"),
            git_dir: PathBuf::from("D:/work/main-repo/.git/worktrees/lane-a"),
            common_dir: PathBuf::from("D:/work/main-repo/.git"),
            store: StoreKey::new("s"),
            store_class: StoreClass::Local,
            trusted,
            git_dir_select: select,
        }
    }

    /// Explicit selection skips git's ownership check, so it needs trust or established
    /// ownership; a handle git discovers is never selected explicitly, trusted or not.
    #[test]
    fn a_translated_handle_is_selected_explicitly_only_once_owned_or_trusted() {
        assert!(!handle(GitDirSelect::Discover, false).selects_explicitly());
        assert!(!handle(GitDirSelect::Discover, true).selects_explicitly());
        assert!(!handle(GitDirSelect::Translated, false).selects_explicitly());
        assert!(handle(GitDirSelect::Translated, true).selects_explicitly());
        assert!(handle(GitDirSelect::Established, false).selects_explicitly());
        assert_eq!(
            handle(GitDirSelect::Translated, false)
                .ownership_established()
                .git_dir_select,
            GitDirSelect::Established
        );
        assert_eq!(
            handle(GitDirSelect::Discover, false)
                .ownership_established()
                .git_dir_select,
            GitDirSelect::Discover
        );
    }

    #[test]
    fn following_a_recorded_handle_rereads_a_wsl_pointer() {
        let base = tempfile::tempdir().unwrap();
        let work = base.path().join("lane-a");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(
            work.join(".git"),
            b"gitdir: /mnt/d/work/main-repo/.git/worktrees/lane-a\n",
        )
        .unwrap();
        let recorded = RepoHandle {
            work_dir: work,
            git_dir: PathBuf::from("D:/work/main-repo/.git"),
            common_dir: PathBuf::from("D:/work/main-repo/.git"),
            store: StoreKey::new("s"),
            store_class: StoreClass::Local,
            trusted: false,
            git_dir_select: GitDirSelect::Discover,
        };
        let followed = recorded.clone().follow_pointer(true);
        assert_eq!(followed.git_dir_select, GitDirSelect::Translated);
        assert_eq!(followed.git_dir, translated_lane());
        let untranslated = recorded.follow_pointer(false);
        assert_eq!(untranslated.git_dir_select, GitDirSelect::Discover);
    }
}
