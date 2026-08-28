//! §1.5's `location` writer. **This module is plan 08's** and holds, for now, only the input
//! type — landed early because plan 07's `ScanStore` seam names it and R1 forbids a second copy.
//!
//! **There is deliberately no `upsert_location` here yet, and plan 07 must not add one.** R1
//! assigns it to plan 08 because the `location` row *is* the project↔path association
//! `resolve_identity` has just decided, and it has to be written in that same transaction. The
//! two `insert_location` functions that existed when R1 was written were both test helpers, so
//! no production path could persist a discovery; adding a second writer here would put the row
//! under two owners, which is the defect R1 exists to close rather than the one it found.
//!
//! `crate::scan::store::SqliteScanStore::upsert_location` therefore refuses, in as many words,
//! and a test pins the refusal so the seam cannot be mistaken for a working one.

use crate::derive::LocationKind;
use crate::index::path::StoredPath;
use crate::protocol::Presence;
use crate::scan::discover::RepoKind;

/// Everything one `location` row needs except its ids (§1.3).
///
/// **This is R27's shape, which supersedes R1's**, and each correction is a column that would
/// otherwise be written wrong:
///
/// * `presence` is explicit — the column has no default, so a writer without this field has to
///   hard-code `'present'`.
/// * `volume_key` is `Option` and the column is nullable — `None` means no stable identifier
///   exists, and mapping it to `''` invents one.
/// * there is no `MountFacts`: the store *class* is a property of the mount right now, not of
///   the location, and a persisted copy goes stale the moment a drive is remounted elsewhere.
/// * `common_dir_bytes` is raw bytes, not a folded key — `path_bytes` and `path_key` are separate
///   columns because that distinction is semantic.
/// * `repo_kind` replaces `is_worktree`, which could not hold `RepoKind`'s four variants; the
///   difference between a linked worktree and a separate git dir is what §1.5 decides lineage on.
///
/// **No serde yet, and R27 asks for it.** Its stated reason is that plan 18's in-distro worker
/// sends one back over the protocol. `StoredPath`'s fields are private and two of the three are
/// *derived* from the third plus a `PathPlatform`, so a derived impl would put the key and the
/// display string on the wire as independent values that can disagree with the bytes — the drift
/// this project keeps finding. The encoding has to carry `(bytes, platform)` and nothing else,
/// and that is a decision for the change that gives plan 18 a wire type to put it in, not a guess
/// made here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationInput {
    pub kind: LocationKind,
    /// `None` for everything but WSL. The column is `NOT NULL` and holds `''` in that case —
    /// v1 used NULL, and SQLite treats NULLs as distinct in a UNIQUE index, so the
    /// `(kind, distro, path_key)` constraint silently permitted duplicates.
    pub distro: Option<String>,
    /// `path_bytes`, `path_key` and `path_display` in one value, keyed for the platform the
    /// path belongs to rather than the host (R2).
    pub path: StoredPath,
    pub store_key: String,
    /// `None` where no stable identifier exists — a bind mount, overlayfs, tmpfs. Absent is not
    /// unknown-and-therefore-empty: a location with no volume key can never be recognised
    /// across a remount, and callers must handle that rather than invent one.
    pub volume_key: Option<String>,
    pub presence: Presence,
    pub repo_kind: RepoKind,
    pub common_dir_bytes: Option<Vec<u8>>,
    /// `location.scan_generation` — the run that last saw this path (§4.6).
    pub generation: i64,
    pub last_seen_at: Option<i64>,
}
