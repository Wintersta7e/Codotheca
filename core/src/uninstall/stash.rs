//! §24.7A's stash truth: what a **deletion gate** may believe about stashed work.
//!
//! **It does not read `location.stash_count`.** That column is a cached badge value keyed on the
//! freshness basis — fine for a badge, and not something to clear a working copy on. A deletion
//! gate reads live, from the files, every time.
//!
//! **And it spawns nothing.** A3 binds: nothing in this tree invokes `git stash` and nothing needs
//! to. Stash truth comes off the filesystem exactly as every other ref-state read does (§3.3);
//! adding a `git stash` subcommand would breach the write boundary §24.1 closes.
//!
//! **Three inputs, because any one of them alone lies:**
//!
//! | Input | Why it is not sufficient alone |
//! |---|---|
//! | `logs/refs/stash` | Absent under `core.logAllRefUpdates=false` while a stash exists |
//! | `refs/stash` (loose) | Absent when the ref is packed |
//! | `packed-refs`, entry `refs/stash` | Absent when the ref is loose |

use std::path::Path;

/// What the files say about stashed work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StashTruth {
    /// Every input agreed there is none, and every input was readable.
    None,
    /// At least this many. The reflog gives an exact count; a ref with no reflog gives **one**,
    /// which is a floor and not a guess — the ref proves a stash exists and says nothing about
    /// how many.
    Present(u32),
    /// An input could not be read. **Unsafe**: it produces `UninstallBlocker::StashUnreadable` and
    /// disposition `unknown`, and never `safe`. An unreadable input is not an absent stash.
    Unreadable,
}

/// One file's contents, distinguishing *absent* from *unreadable*.
enum Read {
    Missing,
    Text(String),
    Unreadable,
}

fn read(path: &Path) -> Read {
    match std::fs::read_to_string(path) {
        Ok(text) => Read::Text(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Read::Missing,
        Err(_) => Read::Unreadable,
    }
}

/// Does a loose `refs/stash` exist? `None` means the question could not be answered.
fn loose_stash_exists(common_dir: &Path) -> Option<bool> {
    match std::fs::symlink_metadata(common_dir.join("refs").join("stash")) {
        Ok(_) => Some(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

/// §24.7A's stash truth for one repository.
///
/// **Unreadable wins.** If any of the three inputs could not be read, the answer is `Unreadable`
/// whatever the others said: a gate that let two readable inputs outvote one unreadable one would
/// be deciding on partial evidence about work it is about to delete.
#[must_use]
pub fn read_stash_truth(common_dir: &Path) -> StashTruth {
    let reflog = read(&common_dir.join("logs").join("refs").join("stash"));
    if matches!(reflog, Read::Unreadable) {
        return StashTruth::Unreadable;
    }
    let Some(loose) = loose_stash_exists(common_dir) else {
        return StashTruth::Unreadable;
    };
    let packed_text = read(&common_dir.join("packed-refs"));
    if matches!(packed_text, Read::Unreadable) {
        return StashTruth::Unreadable;
    }

    // The reflog, when it is there, is the only input that can say *how many*.
    if let Read::Text(text) = reflog {
        let entries = text.lines().filter(|line| !line.trim().is_empty()).count();
        if entries > 0 {
            return StashTruth::Present(u32::try_from(entries).unwrap_or(u32::MAX));
        }
    }

    // No reflog, or an empty one. The ref itself still settles *whether*, and a repository with
    // `core.logAllRefUpdates=false` has exactly this shape.
    if loose {
        return StashTruth::Present(1);
    }
    if crate::git::refstate::packed_refs(common_dir).contains_key("refs/stash") {
        return StashTruth::Present(1);
    }
    StashTruth::None
}

/// The blocker this truth contributes, if any.
///
/// `Unreadable` is a blocker of the **unknown** class: it cannot say the copy is safe, and it
/// cannot say it is holding work either.
#[must_use]
pub const fn blocker(truth: StashTruth) -> Option<crate::protocol::UninstallBlocker> {
    match truth {
        StashTruth::None => None,
        StashTruth::Present(_) => Some(crate::protocol::UninstallBlocker::StashPresent),
        StashTruth::Unreadable => Some(crate::protocol::UninstallBlocker::StashUnreadable),
    }
}
