//! §25.3's backup-state producer — **one declaration, two consumers**, and one of them lives in
//! another process.
//!
//! §25.3 required *one exported producer* in the core, where a TypeScript one could not reach.
//! [p4] **§45's deletion analyser imports nothing from it** (§45.7, PA15): it is a display of
//! stored facts, routed to the pre-pass and the card, and a deletion gate reads live.
//!
//! This is the declaration; OVERVIEW receives its verdict as `ProjectDetail.backup` and **holds
//! no first-match table at all**, switching over three variants and writing the copy, which is
//! that page's standing division.
//!
//! **The numbers and the age are not duplicated onto the wire.** The sentence's counts and the
//! plate's age come from the `LocationDetail` the renderer already holds, so the core carries the
//! decision and the renderer carries the copy, and neither carries both.
//!
//! It never grants or refuses a removal: that is the analyser's alone, from reads made in the
//! same call.

use crate::protocol::BackupState;

/// §25.3's four rows, **first match wins**.
///
/// | Condition | Answer |
/// |---|---|
/// | `remote_key IS NULL` | `only_copy` |
/// | `ahead > 0` or `stash_count > 0` | `not_anywhere_else` |
/// | `ahead = 0`, `stash_count` known `0`, a fetch recorded | `verified` |
/// | `ahead` NULL, `stash_count` unknown, or no fetch recorded | **`None` — no block** |
///
/// **Row 4 is the `None`, which is why this is an enum of three and not four.** An absent field
/// and a fourth variant meaning "nothing" are one value stated twice.
///
/// **Row 3 is the only row that claims currency**, so it is the only one gated on an
/// observation; the renderer carries that observation's age. The word `clean` appears in none of
/// the strings this decision produces — absence of dirty is *no changes as of T*, never "clean".
///
/// # R51, and the limb this producer cannot reach yet
/// `core/src/git/refstate.rs`'s `stash_count` is a `u32` and `read_stash_count` fails the whole
/// `read_ref_state` on an unreadable reflog, so an **unreadable** stash on a location that *was*
/// read is not representable. **p2-24b** makes it `Option<u32>` and asserts that limb
/// (`AC-P2-25-11-unknown`). Row 4's other two limbs — a NULL `ahead`, and no fetch recorded —
/// are exercisable today and are exercised.
#[must_use]
pub fn backup_state(
    remote_key: Option<&str>,
    ahead: Option<u32>,
    stash_count: Option<u32>,
    fetch_head_at: Option<i64>,
) -> Option<BackupState> {
    // Row 1, and it is first deliberately: a project with no remote is the only copy whatever
    // its working tree says, so no later row may answer for it.
    if remote_key.is_none() {
        return Some(BackupState::OnlyCopy);
    }
    if ahead.is_some_and(|n| n > 0) || stash_count.is_some_and(|n| n > 0) {
        return Some(BackupState::NotAnywhereElse);
    }
    if ahead == Some(0) && stash_count == Some(0) && fetch_head_at.is_some() {
        return Some(BackupState::Verified);
    }
    None
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    const KEY: Option<&str> = Some("github.com/acme/widget");
    const FETCHED: Option<i64> = Some(1_781_179_200);

    #[test]
    fn no_remote_is_the_only_copy_on_earth() {
        assert_eq!(
            backup_state(None, Some(0), Some(0), FETCHED),
            Some(BackupState::OnlyCopy)
        );
    }

    #[test]
    fn a_commit_or_a_stash_that_exists_nowhere_else() {
        assert_eq!(
            backup_state(KEY, Some(3), Some(0), FETCHED),
            Some(BackupState::NotAnywhereElse)
        );
        assert_eq!(
            backup_state(KEY, Some(0), Some(1), FETCHED),
            Some(BackupState::NotAnywhereElse)
        );
    }

    #[test]
    fn nothing_on_this_branch_is_only_here_is_gated_on_an_observation() {
        assert_eq!(
            backup_state(KEY, Some(0), Some(0), FETCHED),
            Some(BackupState::Verified)
        );
        // The one row that claims currency is the one row that needs a fetch behind it.
        assert_eq!(backup_state(KEY, Some(0), Some(0), None), None);
    }

    #[test]
    fn row_four_draws_no_block_at_all() {
        // `ahead` NULL — nothing compared this copy.
        assert_eq!(backup_state(KEY, None, Some(0), FETCHED), None);
        // `stash_count` unknown.
        assert_eq!(backup_state(KEY, Some(0), None, FETCHED), None);
        // No fetch recorded.
        assert_eq!(backup_state(KEY, Some(0), Some(0), None), None);
    }

    /// First match, proven by a case that satisfies two rows at once. A table evaluated in any
    /// other order would answer `not_anywhere_else` here — true of the working tree, and wrong
    /// about the project, which has no remote to be behind.
    #[test]
    fn the_first_match_wins_when_two_rows_are_true() {
        assert_eq!(
            backup_state(None, Some(3), Some(2), FETCHED),
            Some(BackupState::OnlyCopy)
        );
    }
}
