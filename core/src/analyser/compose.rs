//! §45.3's composition and §45.9's discharge table.
//!
//! **One table decides whether the network step may run**: step 6 reads the remotes only when no
//! blocker found so far is undischargeable for the act (§45.6), because a verification write
//! that cannot change the answer is not made.

use crate::analyser::remote::RemoteReading;
use crate::analyser::roots::Uncovered;
use crate::analyser::GovernedAct;
use crate::protocol::UninstallBlocker;

/// One of §45.10's escapes, as the discharge table names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escape {
    /// 1: the user pushes to an existing remote, in their own client.
    PushToExisting,
    /// 2: the user creates a repository and pushes to it.
    PushToNew,
    /// `CHECK AGAIN`: the analyser's re-run, which is how either push is confirmed.
    CheckAgain,
}

/// How an act can be freed of one blocker (§45.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discharge {
    /// `—`: named as a reason and offers no escape.
    Never,
    /// These escapes, and nothing else.
    Escapes(&'static [Escape]),
    /// The nested repository's own discharge.
    NestedOwn,
    /// It never stands alone: it accompanies another blocker, whose discharge frees it.
    Accompanies,
}

/// §45.9's discharge column for `act`.
#[must_use]
pub const fn discharge_for(act: GovernedAct, blocker: UninstallBlocker) -> Discharge {
    match act {
        GovernedAct::Uninstall => match blocker {
            UninstallBlocker::UnpushedCommits => Discharge::Escapes(&[
                Escape::PushToExisting,
                Escape::PushToNew,
                Escape::CheckAgain,
            ]),
            UninstallBlocker::UnpushedTag => {
                Discharge::Escapes(&[Escape::PushToExisting, Escape::PushToNew])
            }
            UninstallBlocker::NoRemote => Discharge::Escapes(&[Escape::PushToNew]),
            UninstallBlocker::RemoteUnreachable => Discharge::Escapes(&[Escape::CheckAgain]),
            UninstallBlocker::SubmoduleUnsafe => Discharge::NestedOwn,
            UninstallBlocker::RemoteIsLocalMirror => Discharge::Accompanies,
            UninstallBlocker::StashPresent
            | UninstallBlocker::UncommittedChanges
            | UninstallBlocker::UntrackedPrecious
            | UninstallBlocker::IgnoredPrecious
            | UninstallBlocker::LinkedWorktree
            | UninstallBlocker::LiveSession
            | UninstallBlocker::RefusedPath
            | UninstallBlocker::InterruptedOperation
            | UninstallBlocker::BorrowedByAnotherRepository
            | UninstallBlocker::ShallowClone
            | UninstallBlocker::StashUnreadable
            | UninstallBlocker::NeverObserved
            | UninstallBlocker::RefsUnreadable
            | UninstallBlocker::HiddenFromStatus
            | UninstallBlocker::LfsUnverified
            | UninstallBlocker::NestingTooDeep => Discharge::Never,
        },
    }
}

/// Is `blocker` one no escape discharges for `act`? When one is present, step 6 does not run.
#[must_use]
pub const fn is_undischargeable(act: GovernedAct, blocker: UninstallBlocker) -> bool {
    matches!(discharge_for(act, blocker), Discharge::Never)
}

/// Every configured remote's reading, reduced to what §45.3's table asks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteSummary {
    /// Network remotes, answered or not — a not-admitted transport counts here.
    pub network: usize,
    /// Of those, the ones that answered.
    pub answered: usize,
    /// Same-machine remotes: they contribute nothing, and are named only beside an uncovered root.
    pub same_machine: usize,
    /// `T`: every object id an answering remote advertised that is present locally.
    pub covered: Vec<String>,
}

impl RemoteSummary {
    /// Fold the readings, in any order.
    #[must_use]
    pub fn of(readings: &[RemoteReading]) -> Self {
        let mut summary = Self::default();
        for reading in readings {
            match reading {
                RemoteReading::SameMachine => summary.same_machine += 1,
                RemoteReading::Answered { present, .. } => {
                    summary.network += 1;
                    summary.answered += 1;
                    summary.covered.extend(present.iter().cloned());
                }
                RemoteReading::DidNotAnswer(_) => summary.network += 1,
            }
        }
        summary.covered.sort_unstable();
        summary.covered.dedup();
        summary
    }
}

/// §45.3's composition over what step 7 left uncovered.
///
/// | Remotes | Uncovered commit root | Uncovered tag object |
/// |---|---|---|
/// | no network remote | `unpushed_commits` + `no_remote` | `unpushed_tag` + `no_remote` |
/// | every network remote answered | `unpushed_commits` | `unpushed_tag` |
/// | at least one did not answer | `remote_unreachable` | `remote_unreachable` |
///
/// `remote_is_local_mirror` joins **only** beside something uncovered and a same-machine remote.
/// Nothing uncovered is no remote blocker, whatever else did not answer.
#[must_use]
pub fn compose_remote_blockers(
    summary: &RemoteSummary,
    uncovered: Uncovered,
) -> Vec<UninstallBlocker> {
    if !uncovered.commit && !uncovered.tag {
        return Vec::new();
    }
    let mut blockers = Vec::new();
    if summary.network == 0 {
        blockers.push(UninstallBlocker::NoRemote);
        if uncovered.commit {
            blockers.push(UninstallBlocker::UnpushedCommits);
        }
        if uncovered.tag {
            blockers.push(UninstallBlocker::UnpushedTag);
        }
    } else if summary.answered == summary.network {
        if uncovered.commit {
            blockers.push(UninstallBlocker::UnpushedCommits);
        }
        if uncovered.tag {
            blockers.push(UninstallBlocker::UnpushedTag);
        }
    } else {
        // Never `unpushed_commits`: a remote that did not answer may hold every one of them.
        blockers.push(UninstallBlocker::RemoteUnreachable);
    }
    if summary.same_machine > 0 {
        blockers.push(UninstallBlocker::RemoteIsLocalMirror);
    }
    blockers
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{compose_remote_blockers, discharge_for, Discharge, RemoteSummary};
    use crate::analyser::remote::{DidNotAnswer, RemoteReading};
    use crate::analyser::roots::Uncovered;
    use crate::analyser::GovernedAct;
    use crate::protocol::UninstallBlocker;

    const COMMIT: Uncovered = Uncovered {
        commit: true,
        tag: false,
    };

    fn answered(present: &[&str]) -> RemoteReading {
        RemoteReading::Answered {
            present: present.iter().map(|s| (*s).to_owned()).collect(),
            fetched: Vec::new(),
        }
    }

    #[test]
    fn every_blocker_has_a_discharge_row_for_uninstall() {
        let never = UninstallBlocker::ALL
            .iter()
            .filter(|b| discharge_for(GovernedAct::Uninstall, **b) == Discharge::Never)
            .count();
        assert!(never > 0 && never < UninstallBlocker::ALL.len());
    }

    #[test]
    fn a_remote_that_did_not_answer_is_never_unpushed() {
        let summary = RemoteSummary::of(&[
            answered(&[]),
            RemoteReading::DidNotAnswer(DidNotAnswer::Failed),
        ]);
        assert_eq!(
            compose_remote_blockers(&summary, COMMIT),
            vec![UninstallBlocker::RemoteUnreachable]
        );
    }

    #[test]
    fn no_network_remote_is_no_remote_and_a_mirror_is_named_beside_it() {
        let summary = RemoteSummary::of(&[RemoteReading::SameMachine]);
        assert_eq!(
            compose_remote_blockers(&summary, COMMIT),
            vec![
                UninstallBlocker::NoRemote,
                UninstallBlocker::UnpushedCommits,
                UninstallBlocker::RemoteIsLocalMirror
            ]
        );
    }

    #[test]
    fn nothing_uncovered_is_no_remote_blocker_whatever_did_not_answer() {
        let summary = RemoteSummary::of(&[
            RemoteReading::DidNotAnswer(DidNotAnswer::Deadline),
            RemoteReading::SameMachine,
        ]);
        let none = Uncovered {
            commit: false,
            tag: false,
        };
        assert!(compose_remote_blockers(&summary, none).is_empty());
    }
}
