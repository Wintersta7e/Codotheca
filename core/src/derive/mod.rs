//! §5's derived values. **This module is plan 09's.** `LocationKind` was landed early because
//! plan 07 needed it and R21 forbids a second copy; the rest arrives with this plan.
//!
//! R21 settled `LocationKind` on plan 09 after finding it declared identically in two plans. The
//! alternative to landing it here was a local duplicate in `core::scan`, which is the collision
//! the ruling exists to prevent — the same shape as plan 02 needing `GIT_FLOOR` before plan 05
//! ran, which was resolved the same way.

pub mod condition;
pub mod description;
pub mod persist;

/// Which world a `location` row's path belongs to — `location.kind` (§1.3).
///
/// **R31: declared in `protocol/schema/protocol.json` and generated into `crate::protocol`.**
/// Re-exported so this module's path still names it, and declared nowhere else — a second
/// hand-written copy compiles and then drifts from the wire form. The generated variant renames
/// are `win`/`linux`/`wsl`, matching `as_str` below character for character, which is what the
/// column's `CHECK (kind IN ('win', 'linux', 'wsl'))` requires; a mismatch would fail at insert
/// time rather than in review. Same treatment as `Outcome` in `core/src/proto/wire.rs`.
pub use crate::protocol::LocationKind;

impl LocationKind {
    // [p3] `LocationKind::ALL` is **generated** now, from the schema's own variant list. The
    // hand-written copy that stood here carried a count a human maintained for a type the schema
    // already declares; every call site is unchanged.

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Win => "win",
            Self::Linux => "linux",
            Self::Wsl => "wsl",
        }
    }

    /// `as_str`'s inverse. `None` for anything else: a `kind` this build does not know is a row
    /// from a newer schema, and guessing would file a Windows path under Linux path rules.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "win" => Some(Self::Win),
            "linux" => Some(Self::Linux),
            "wsl" => Some(Self::Wsl),
            _ => None,
        }
    }
}

use crate::protocol::{LocationId, Presence};

impl LocationKind {
    /// "Native side preferred" (§5.1): the side this build of the app runs on.
    #[must_use]
    pub fn is_native(self) -> bool {
        if cfg!(windows) {
            matches!(self, Self::Win)
        } else {
            matches!(self, Self::Linux)
        }
    }
}

/// One copy's facts, as §5.1 aggregates them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationFacts {
    /// Which copy.
    pub location_id: LocationId,
    /// Which world its path belongs to.
    pub kind: LocationKind,
    /// Whether it can be read right now.
    pub presence: Presence,
    /// `None` = never observed. Never rendered as "clean".
    pub is_dirty: Option<bool>,
    /// J3's newest tracked-file mtime.
    pub worktree_newest_mtime: Option<i64>,
    /// J1's `logs/HEAD` mtime — the last sign of local git activity.
    pub reflog_tail_at: Option<i64>,
    /// The branch as J1 read it.
    pub branch: Option<String>,
    /// Commits ahead of upstream, `None` when not computed.
    pub ahead: Option<u32>,
    /// Commits behind upstream, `None` when not computed.
    pub behind: Option<u32>,
}

/// What §5.1 derives from every copy of one project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Aggregated {
    /// §5.1: max(last commit by a user identity, last session end, worktree newest mtime).
    pub last_touched_at: Option<i64>,
    /// §5.1: max(reflog tail, last session end, worktree newest mtime).
    ///
    /// The worktree term is the correction. Without it the sort and the glow are computed from
    /// different inputs, and a repository edited today outside the app sorts to the top of the
    /// shelf while rendering as neglected — the freshest tile on the shelf, dark.
    pub last_interaction_at: Option<i64>,
    /// `None` until something was observed. `Some(false)` is "no changes as of T" on every
    /// location that has been looked at, which is not the same claim as "clean".
    pub any_dirty: Option<bool>,
    /// The copy §5.1 calls primary, `None` when none is present.
    pub primary_location: Option<LocationId>,
}

fn max_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

/// Fold every present copy into the project's derived clocks.
///
/// Offline copies contribute nothing: a dirty flag from a drive that is not mounted is a memory,
/// not an observation, and §6 forbids claiming currency the app does not have.
#[must_use]
pub fn aggregate(
    locations: &[LocationFacts],
    last_user_commit_at: Option<i64>,
    last_session_end_at: Option<i64>,
) -> Aggregated {
    let present: Vec<&LocationFacts> = locations
        .iter()
        .filter(|l| matches!(l.presence, Presence::Present))
        .collect();

    let worktree = present
        .iter()
        .fold(None, |acc, l| max_opt(acc, l.worktree_newest_mtime));
    let reflog = present
        .iter()
        .fold(None, |acc, l| max_opt(acc, l.reflog_tail_at));

    let last_touched_at = max_opt(max_opt(last_user_commit_at, last_session_end_at), worktree);
    let last_interaction_at = max_opt(max_opt(reflog, last_session_end_at), worktree);

    let mut any_dirty: Option<bool> = None;
    for l in &present {
        match l.is_dirty {
            Some(true) => {
                any_dirty = Some(true);
                break;
            }
            Some(false) => any_dirty = Some(any_dirty.unwrap_or(false)),
            None => {}
        }
    }

    // R41 is open on the exact rule for `LocationDetail.isPrimary`. §5.1's prose is "most
    // recently touched, present, native side preferred" and `location` has no `touched` column,
    // so this uses J3's `worktree_newest_mtime` — an interaction clock. Plan 13 independently
    // used `last_seen_at`, which is a *presence* clock and answers a different question.
    // Reported rather than settled here.
    let primary_location = present
        .iter()
        .max_by(|a, b| {
            a.worktree_newest_mtime
                .cmp(&b.worktree_newest_mtime)
                .then_with(|| a.kind.is_native().cmp(&b.kind.is_native()))
                .then_with(|| b.location_id.0.cmp(&a.location_id.0))
        })
        .map(|l| l.location_id);

    Aggregated {
        last_touched_at,
        last_interaction_at,
        any_dirty,
        primary_location,
    }
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

    fn loc(id: i64, kind: LocationKind, presence: Presence) -> LocationFacts {
        LocationFacts {
            location_id: LocationId(id),
            kind,
            presence,
            is_dirty: None,
            worktree_newest_mtime: None,
            reflog_tail_at: None,
            branch: None,
            ahead: None,
            behind: None,
        }
    }

    #[test]
    fn the_sort_and_the_glow_are_derived_from_the_same_inputs() {
        // §5.1, the v1 defect: the sort came from one input set and the glow from another,
        // so a repo edited today outside the app sorted to the top of the shelf while
        // rendering as neglected — the freshest tile on the shelf, dark.
        let mut a = loc(1, LocationKind::Win, Presence::Present);
        a.worktree_newest_mtime = Some(2_000);
        let agg = aggregate(&[a], Some(100), None);
        assert_eq!(agg.last_touched_at, Some(2_000));
        assert_eq!(
            agg.last_interaction_at,
            Some(2_000),
            "worktree mtime is in BOTH, which is the whole fix"
        );
    }

    #[test]
    fn last_touched_takes_the_max_of_its_three_inputs() {
        let mut a = loc(1, LocationKind::Win, Presence::Present);
        a.worktree_newest_mtime = Some(500);
        assert_eq!(
            aggregate(&[a.clone()], Some(900), Some(700)).last_touched_at,
            Some(900)
        );
        assert_eq!(
            aggregate(&[a.clone()], Some(100), Some(700)).last_touched_at,
            Some(700)
        );
        assert_eq!(aggregate(&[a], None, None).last_touched_at, Some(500));
    }

    #[test]
    fn last_interaction_uses_the_reflog_tail_not_the_commit_clock() {
        let mut a = loc(1, LocationKind::Win, Presence::Present);
        a.reflog_tail_at = Some(400);
        let agg = aggregate(&[a], Some(9_000), None);
        assert_eq!(agg.last_interaction_at, Some(400));
        assert_eq!(agg.last_touched_at, Some(9_000));
    }

    #[test]
    fn nothing_known_stays_not_computed() {
        let agg = aggregate(&[loc(1, LocationKind::Win, Presence::Present)], None, None);
        assert_eq!(agg.last_touched_at, None);
        assert_eq!(agg.last_interaction_at, None);
        assert_eq!(agg.any_dirty, None, "no observation is not 'no changes'");
    }

    #[test]
    fn is_dirty_is_true_if_any_present_location_is_dirty() {
        let mut a = loc(1, LocationKind::Win, Presence::Present);
        a.is_dirty = Some(false);
        let mut b = loc(2, LocationKind::Linux, Presence::Present);
        b.is_dirty = Some(true);
        assert_eq!(aggregate(&[a, b], None, None).any_dirty, Some(true));
    }

    #[test]
    fn an_offline_locations_dirty_flag_does_not_count() {
        let mut a = loc(1, LocationKind::Win, Presence::Present);
        a.is_dirty = Some(false);
        let mut b = loc(2, LocationKind::Wsl, Presence::Offline);
        b.is_dirty = Some(true);
        assert_eq!(aggregate(&[a, b], None, None).any_dirty, Some(false));
    }

    #[test]
    fn one_observed_false_and_one_unobserved_is_still_only_false_so_far() {
        let mut a = loc(1, LocationKind::Win, Presence::Present);
        a.is_dirty = Some(false);
        let b = loc(2, LocationKind::Win, Presence::Present);
        assert_eq!(aggregate(&[a, b], None, None).any_dirty, Some(false));
    }

    #[test]
    fn the_primary_is_the_most_recently_touched_present_location() {
        let mut a = loc(1, LocationKind::Win, Presence::Present);
        a.worktree_newest_mtime = Some(100);
        let mut b = loc(2, LocationKind::Win, Presence::Present);
        b.worktree_newest_mtime = Some(900);
        assert_eq!(
            aggregate(&[a, b], None, None).primary_location,
            Some(LocationId(2))
        );
    }

    #[test]
    fn the_native_side_breaks_a_tie() {
        // Native is the side this build runs on, so the fixture asks for it by that name
        // rather than hard-coding one platform's answer into a cross-platform test.
        let native = if cfg!(windows) {
            LocationKind::Win
        } else {
            LocationKind::Linux
        };
        let a = loc(1, LocationKind::Wsl, Presence::Present);
        let b = loc(2, native, Presence::Present);
        assert_eq!(
            aggregate(&[a, b], None, None).primary_location,
            Some(LocationId(2))
        );
    }

    #[test]
    fn an_all_offline_project_has_no_primary() {
        let a = loc(1, LocationKind::Win, Presence::Offline);
        assert_eq!(aggregate(&[a], None, None).primary_location, None);
    }

    /// An offline copy is not a source of clocks either: a mounted drive's mtime from last
    /// month must not present as the project having been touched then.
    #[test]
    fn an_offline_locations_clocks_do_not_count() {
        let mut offline = loc(1, LocationKind::Win, Presence::Offline);
        offline.worktree_newest_mtime = Some(9_000);
        offline.reflog_tail_at = Some(9_000);
        let agg = aggregate(&[offline], None, None);
        assert_eq!(agg.last_touched_at, None);
        assert_eq!(agg.last_interaction_at, None);
    }
}
