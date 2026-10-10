//! §38: the XP ledger's vocabulary — which kinds `xp_events` may hold and which of its two
//! classes each belongs to.
//!
//! `0019_ledger.sql` states the same two lists as CHECK constraints; `ledger_migration.rs` reads
//! them back out of `sqlite_master` and compares, so neither copy can drift alone.
//!
//! What the rows are worth is computed here at read time and never stored: `price` holds the one
//! price table, `curve` turns XP into a level, and `xp` sums the ledger term by term (§38.3–§38.4).
//! `level` is the one reading of all three, and `floor` the level it never falls below (§38.5–§38.6).

pub mod curve;
pub mod floor;
pub mod level;
pub mod price;
pub mod xp;

/// Every kind `xp_events.kind` admits: the git-derived ones first, then the retained ones.
pub const KINDS: [&str; 7] = [
    "commit_day",
    "release",
    "focus",
    "debt_day",
    "completion_check",
    "amnesty_verdict",
    "clean_archive",
];

/// The kinds that are a pure function of history, filed under `track = 'git'`: deleted and
/// recomputed from the commits, never migrated through a merge.
pub const GIT_KINDS: [&str; 2] = ["commit_day", "release"];

/// The kinds that record something history cannot reproduce, filed under `track = 'session'`.
/// The schema refuses to delete any of them.
pub const RETAINED_KINDS: [&str; 5] = [
    "focus",
    "debt_day",
    "completion_check",
    "amnesty_verdict",
    "clean_archive",
];

/// Kinds earlier schemas admitted and §38 removes. None ever had a writer; a stored row of one
/// fails the rebuild rather than being filtered out of it.
pub const CUT_KINDS: [&str; 4] = ["revival", "language_first", "first_push", "session"];

/// The `app_meta` key that held the old floor as an XP count. The migration deletes it and a
/// restored sidecar drops it; nothing reads or writes it as a live value.
pub const LEGACY_FLOOR_KEY: &str = "level_floor";
