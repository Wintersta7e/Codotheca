//! §24.7B–G: the gates a removal must clear before anything is touched.
//!
//! Every one of them answers in the same direction: **a fact that cannot be established blocks**.
//! The failure this module exists to prevent is a pre-flight that reports *safe* about something
//! it could not read, and every `Err`, every `None` and every unreachable remote below becomes a
//! blocker rather than a silence.

use std::path::{Path, PathBuf};

use crate::protocol::UninstallBlocker;

/// §24.7B: a shallow clone is never uninstallable.
///
/// *Every local ref present upstream* cannot be proved over a truncated graph, and unknown behaves
/// as unsafe. The same exclusion already applies to span, best-year and commit-day maths.
#[must_use]
pub const fn gate_shallow(is_shallow: bool) -> Option<UninstallBlocker> {
    if is_shallow {
        Some(UninstallBlocker::ShallowClone)
    } else {
        None
    }
}

/// What a live remote check established.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteVerification {
    /// What the check's outcome blocks; `verify_remote` leaves it empty only for `Reached`.
    pub blockers: Vec<UninstallBlocker>,
    /// When the remote was actually reached. `None` means it was not — never an old value, and
    /// never a zero.
    pub verified_at: Option<i64>,
}

/// How a remote check ended. The transport is faked in tests; **the verdict never is**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteOutcome {
    /// The remote answered and the refs were read.
    Reached,
    /// 401, 403 or 404.
    ///
    /// **Never *gone*.** A 404 against a private repository the caller cannot see is
    /// indistinguishable from one that does not exist, and rendering it as *gone* is the specific
    /// mistake that turns this tool into a shredder.
    Refused,
    /// Offline, DNS failure, timeout — anything that never reached the other end.
    Unreachable,
    /// The remote URL resolves to a path on this machine.
    LocalMirror,
}

/// §24.7C: verified, not believed.
///
/// Remote-tracking refs are a cache that may predate a force-push, which is why the label a
/// surface renders is `VERIFIED <age>` and **never `PUSHED`**.
#[must_use]
pub fn verify_remote(outcome: RemoteOutcome, now: i64) -> RemoteVerification {
    match outcome {
        RemoteOutcome::Reached => RemoteVerification {
            blockers: Vec::new(),
            verified_at: Some(now),
        },
        // 401/403/404 and a transport failure are the same claim — *this was not established* —
        // and both are the unknown class. Mapping 404 to *gone*, and therefore to safe, is the
        // shredder.
        RemoteOutcome::Refused | RemoteOutcome::Unreachable => RemoteVerification {
            blockers: vec![UninstallBlocker::RemoteUnreachable],
            verified_at: None,
        },
        // A mirror on the same machine is reported honestly and never counted as a backup: one
        // disk failure takes both copies.
        RemoteOutcome::LocalMirror => RemoteVerification {
            blockers: vec![UninstallBlocker::RemoteIsLocalMirror],
            verified_at: None,
        },
    }
}

/// Does this remote URL resolve to somewhere on this machine?
#[must_use]
pub fn is_local_mirror(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.starts_with("file://") {
        return true;
    }
    // A bare absolute or relative path, or a Windows drive-lettered one. `https://` and `ssh://`
    // and `git@host:` are the remote forms; anything else that names a directory is local.
    if trimmed.contains("://") {
        return false;
    }
    if trimmed.contains('@') && trimmed.contains(':') {
        return false;
    }
    trimmed.starts_with('/')
        || trimmed.starts_with('.')
        || trimmed.starts_with('~')
        || trimmed
            .as_bytes()
            .get(1)
            .is_some_and(|b| *b == b':' || *b == b'\\')
}

/// §24.7D's refusal list. Each of these is `refused_path`, in the **blocked** class.
///
/// `roots` are the configured scan roots. A path under one is fine; a path that **is** one is not,
/// and neither is one outside every root — the containment check runs in both directions, which is
/// what `core/src/firstrun/roots.rs` gets wrong for root nesting and what this must not repeat.
#[must_use]
pub fn gate_path(path: &Path, roots: &[PathBuf]) -> Option<UninstallBlocker> {
    // A symlink is never followed, here or while removing.
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => return Some(UninstallBlocker::RefusedPath),
        Ok(_) => {}
        // A path that cannot be stated cannot be cleared.
        Err(_) => return Some(UninstallBlocker::RefusedPath),
    }

    // A `.git` that is not a direct child is a linked worktree or a separate git dir, and removing
    // the copy would break whatever holds the real one.
    if !path.join(".git").exists() {
        return Some(UninstallBlocker::RefusedPath);
    }

    // A drive root or a filesystem root has no parent to be contained by.
    if path.parent().is_none() {
        return Some(UninstallBlocker::RefusedPath);
    }

    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if home.as_deref() == Some(path) {
        return Some(UninstallBlocker::RefusedPath);
    }

    // **Both directions.** Under a root: fine. Equal to a root: refused. Outside every root:
    // refused, because the app was never given consent to touch it.
    let mut contained = false;
    for root in roots {
        if path == root.as_path() {
            return Some(UninstallBlocker::RefusedPath);
        }
        if path.starts_with(root) {
            contained = true;
        }
    }
    if contained {
        None
    } else {
        Some(UninstallBlocker::RefusedPath)
    }
}

/// §24.7D: a repository with a live launch session.
///
/// # Errors
/// Fails when the `session` table cannot be read — which is itself a refusal, never a pass.
pub fn gate_live_session(
    tx: &rusqlite::Transaction<'_>,
    location: crate::protocol::LocationId,
) -> rusqlite::Result<Option<UninstallBlocker>> {
    let live: i64 = tx.query_row(
        "SELECT COUNT(*) FROM session WHERE location_id = ?1 AND ended_at IS NULL",
        [location.0],
        |row| row.get(0),
    )?;
    Ok((live > 0).then_some(UninstallBlocker::LiveSession))
}

/// §24.7G's first-day lock: **you cannot uninstall what the app has never successfully looked at.**
///
/// *Never observed* and *stale but once known* are distinct and neither may be rendered as the
/// other: this gate fires only on the first, which is why it takes both observation clocks rather
/// than an error flag.
#[must_use]
pub const fn gate_first_day(
    refstate_observed_at: Option<i64>,
    worktree_observed_at: Option<i64>,
) -> Option<UninstallBlocker> {
    if refstate_observed_at.is_none() || worktree_observed_at.is_none() {
        Some(UninstallBlocker::NeverObserved)
    } else {
        None
    }
}
