//! Identity assignment (§1.1). Pure: evidence in, one decision out.

use super::lineage::lineage_key;
use super::remote::{owner_of, pick_canonical_remote};

/// Everything a scan learns about one repository that bears on which project it is.
#[derive(Debug, Clone)]
pub struct IdentityProbe {
    /// The canonical comparison form of `git rev-parse --git-common-dir`. Produced by the same
    /// canonicaliser the caller uses for `location.path_key` (§1.3); `None` when it could not
    /// be read.
    pub common_dir_key: Option<Vec<u8>>,
    /// `git rev-parse --is-shallow-repository` (§3.1).
    pub is_shallow: bool,
    /// The output of `lineage::root_set_argv`, parsed.
    pub root_oids: Vec<String>,
    /// `(remote name, raw url)` from `remote::remote_urls_argv`, parsed.
    pub remote_urls: Vec<(String, String)>,
}

/// The probe reduced to the three facts §1.1's tables are written against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityEvidence {
    /// The probe's folded `git-common-dir`, carried unchanged — definitive evidence when another
    /// location already has it.
    pub common_dir_key: Option<Vec<u8>>,
    /// `None` for a repository with no commits and for a shallow clone (§1.1, and the boundary
    /// argument in `lineage::lineage_key`).
    pub lineage_key: Option<String>,
    /// The canonical `<host>/<owner>/<name>` of the remote picked from the probe's list, or
    /// `None` when there is none.
    pub remote_key: Option<String>,
    /// The `<owner>` part of that remote — what tells a fork from the same project.
    pub remote_owner: Option<String>,
}

/// Reduce a probe to its evidence: the lineage from the root set, and one canonical remote.
#[must_use]
pub fn evidence_from(probe: &IdentityProbe) -> IdentityEvidence {
    let remote = pick_canonical_remote(&probe.remote_urls);
    IdentityEvidence {
        common_dir_key: probe.common_dir_key.clone(),
        lineage_key: lineage_key(&probe.root_oids, probe.is_shallow),
        remote_owner: remote.as_ref().map(|r| r.owner.clone()),
        remote_key: remote.map(|r| r.key),
    }
}

/// A project already in the index that shares the probe's `lineage_key`.
///
/// Tombstoned rows are never candidates. The caller supplies them ordered by `(created_at, id)`,
/// which is what makes every decision below independent of the order the scanner happened to
/// walk in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The candidate project.
    pub project_id: i64,
    /// Its canonical remote, or `None` for a remoteless project.
    pub remote_key: Option<String>,
    /// When it was created, in Unix seconds — the first half of the ordering.
    pub created_at: i64,
}

/// Which project a scanned repository belongs to, and on what evidence (§1.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityDecision {
    /// Same `git-common-dir` — a linked worktree.
    AttachDefinitive {
        /// The project that already owns that common dir.
        project_id: i64,
    },
    /// Same lineage and the same canonical remote.
    AttachStrong {
        /// The first candidate carrying that remote.
        project_id: i64,
    },
    /// Same lineage, one side remoteless, exactly one candidate. Recorded as inferred so
    /// §8.5.2 can name the evidence.
    AttachInferred {
        /// The one candidate attached to.
        project_id: i64,
    },
    /// Same lineage, a different remote owner. Separate projects, linked by `lineage_key` and
    /// surfaced as related; `related` is every project on this lineage under another owner.
    NewFork {
        /// Every candidate whose remote has another owner; each is marked a fork too.
        related: Vec<i64>,
    },
    /// Same lineage, remoteless, two or more candidates. Its own project, flagged
    /// `ambiguous_lineage`. **Never guess.**
    NewAmbiguous {
        /// The candidates that carry a remote — the set §11.1's summary renders.
        candidates: Vec<i64>,
    },
    /// Nothing associates it with anything already indexed.
    New,
}

/// §1.1, as one function. First match wins, and the order is the order of the evidence table:
/// definitive, strong, strong-negative, weak, ambiguous.
#[must_use]
pub fn decide(
    evidence: &IdentityEvidence,
    worktree_of: Option<i64>,
    candidates: &[Candidate],
) -> IdentityDecision {
    // Definitive. A linked worktree is the same project whatever the history says.
    if let Some(project_id) = worktree_of {
        return IdentityDecision::AttachDefinitive { project_id };
    }

    // No lineage: no commits, or a shallow clone. Identity is the location alone; never merges.
    if evidence.lineage_key.is_none() {
        return IdentityDecision::New;
    }

    evidence.remote_key.as_deref().map_or_else(
        || decide_without_remote(candidates),
        |ours| decide_with_remote(ours, evidence.remote_owner.as_deref(), candidates),
    )
}

fn decide_with_remote(
    ours: &str,
    our_owner: Option<&str>,
    candidates: &[Candidate],
) -> IdentityDecision {
    // Strong: same lineage and the same canonical remote.
    if let Some(c) = candidates
        .iter()
        .find(|c| c.remote_key.as_deref() == Some(ours))
    {
        return IdentityDecision::AttachStrong {
            project_id: c.project_id,
        };
    }

    // Strong negative: same lineage under another owner. A fork — link, never merge.
    let related: Vec<i64> = candidates
        .iter()
        .filter(|c| {
            c.remote_key
                .as_deref()
                .and_then(owner_of)
                .is_some_and(|theirs| Some(theirs) != our_owner)
        })
        .map(|c| c.project_id)
        .collect();
    if !related.is_empty() {
        return IdentityDecision::NewFork { related };
    }

    // Remaining possibilities are a same-owner different-name remote (a rename or a transfer)
    // and remoteless candidates. Neither may absorb a repository that has a remote of its own:
    // a partial remote match is not a match, and attaching to a remoteless project would make
    // the outcome depend on which fork the scanner reached first.
    IdentityDecision::New
}

fn decide_without_remote(candidates: &[Candidate]) -> IdentityDecision {
    let remoted: Vec<i64> = candidates
        .iter()
        .filter(|c| c.remote_key.is_some())
        .map(|c| c.project_id)
        .collect();

    // §11.1 defines the candidate set the summary renders as exactly this — the projects
    // sharing the lineage that carry a non-NULL remote_key — so the flag and the surface count
    // the same thing.
    if remoted.len() >= 2 {
        return IdentityDecision::NewAmbiguous {
            candidates: remoted,
        };
    }
    if let Some(project_id) = remoted.first() {
        return IdentityDecision::AttachInferred {
            project_id: *project_id,
        };
    }
    // No remoted candidate at all: any remoteless sibling is another copy of the same local
    // repository, and there is no evidence by which they could differ.
    candidates.first().map_or(IdentityDecision::New, |c| {
        IdentityDecision::AttachInferred {
            project_id: c.project_id,
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::{
        decide, evidence_from, Candidate, IdentityDecision as D, IdentityEvidence, IdentityProbe,
    };

    fn ev(lineage: Option<&str>, remote: Option<&str>) -> IdentityEvidence {
        IdentityEvidence {
            common_dir_key: None,
            lineage_key: lineage.map(str::to_owned),
            remote_key: remote.map(str::to_owned),
            remote_owner: remote
                .and_then(super::super::remote::owner_of)
                .map(str::to_owned),
        }
    }

    fn cand(id: i64, remote: Option<&str>, created_at: i64) -> Candidate {
        Candidate {
            project_id: id,
            remote_key: remote.map(str::to_owned),
            created_at,
        }
    }

    // ---- §1.1 evidence table ------------------------------------------------------------

    #[test]
    fn same_git_common_dir_is_definitive_whatever_the_lineage_says() {
        // A linked worktree is always the same project, so this outranks everything, including
        // a lineage that would otherwise read as a fork.
        let e = ev(Some("L"), Some("forge.example/mine/widget"));
        let others = [cand(1, Some("forge.example/acme/widget"), 10)];
        assert_eq!(
            decide(&e, Some(7), &others),
            D::AttachDefinitive { project_id: 7 }
        );
    }

    #[test]
    fn same_lineage_and_same_canonical_remote_is_strong() {
        let e = ev(Some("L"), Some("forge.example/acme/widget"));
        let c = [cand(3, Some("forge.example/acme/widget"), 10)];
        assert_eq!(decide(&e, None, &c), D::AttachStrong { project_id: 3 });
    }

    #[test]
    fn same_lineage_different_remote_owner_is_a_fork_and_never_merges() {
        let e = ev(Some("L"), Some("forge.example/mine/widget"));
        let c = [cand(3, Some("forge.example/acme/widget"), 10)];
        assert_eq!(decide(&e, None, &c), D::NewFork { related: vec![3] });
    }

    #[test]
    fn same_lineage_one_side_remoteless_one_candidate_is_inferred() {
        let e = ev(Some("L"), None);
        let c = [cand(3, Some("forge.example/acme/widget"), 10)];
        assert_eq!(decide(&e, None, &c), D::AttachInferred { project_id: 3 });
    }

    #[test]
    fn same_lineage_one_side_remoteless_two_candidates_is_ambiguous_and_never_guesses() {
        let e = ev(Some("L"), None);
        let c = [
            cand(3, Some("forge.example/acme/widget"), 10),
            cand(4, Some("forge.example/mine/widget"), 20),
        ];
        assert_eq!(
            decide(&e, None, &c),
            D::NewAmbiguous {
                candidates: vec![3, 4]
            }
        );
    }

    #[test]
    fn same_remote_different_lineage_is_not_the_same_project() {
        // The caller only ever passes candidates that share our lineage, so a project with our
        // remote and a different history is simply not in the set — and we are new.
        let e = ev(Some("L2"), Some("forge.example/acme/widget"));
        assert_eq!(decide(&e, None, &[]), D::New);
    }

    // ---- §1.1 case table, the rows the evidence table does not repeat ---------------------

    #[test]
    fn no_commits_at_all_is_the_location_alone_and_never_merges() {
        let e = ev(None, Some("forge.example/acme/widget"));
        let c = [cand(3, Some("forge.example/acme/widget"), 10)];
        assert_eq!(decide(&e, None, &c), D::New);
    }

    #[test]
    fn two_remoteless_copies_of_one_local_repository_collapse_to_one_project() {
        // Criterion 2 says two working copies of one repository collapse. Neither side has an
        // owner, so there is no fork evidence; refusing to attach would render a purely local
        // repository copied twice as two projects.
        let e = ev(Some("L"), None);
        let c = [cand(5, None, 10)];
        assert_eq!(decide(&e, None, &c), D::AttachInferred { project_id: 5 });
    }

    #[test]
    fn a_repository_with_a_remote_never_attaches_to_a_remoteless_project() {
        // Running the weak rule in reverse would make the outcome depend on walk order: of two
        // differently-owned forks arriving after a remoteless project, whichever was walked
        // first would absorb it.
        let e = ev(Some("L"), Some("forge.example/acme/widget"));
        let c = [cand(5, None, 10)];
        assert_eq!(decide(&e, None, &c), D::New);
    }

    #[test]
    fn same_lineage_same_owner_different_repository_name_stays_separate() {
        // A rename or a transfer. It is not a fork — the owner matches — and it is not the same
        // project, because the remote does not match exactly. Never merge on a partial match.
        let e = ev(Some("L"), Some("forge.example/acme/widget"));
        let c = [cand(3, Some("forge.example/acme/gadget"), 10)];
        assert_eq!(decide(&e, None, &c), D::New);
    }

    #[test]
    fn ties_resolve_on_the_earliest_created_project_then_the_lowest_id() {
        let e = ev(Some("L"), Some("forge.example/acme/widget"));
        let c = [
            cand(9, Some("forge.example/acme/widget"), 5),
            cand(2, Some("forge.example/acme/widget"), 5),
        ];
        // The caller supplies candidates ordered by (created_at, id), so the first is the answer
        // and the decision is independent of walk order.
        let mut ordered = c.to_vec();
        ordered.sort_by_key(|x| (x.created_at, x.project_id));
        assert_eq!(
            decide(&e, None, &ordered),
            D::AttachStrong { project_id: 2 }
        );
    }

    // ---- evidence_from ------------------------------------------------------------------

    #[test]
    fn evidence_takes_origin_and_drops_lineage_for_a_shallow_clone() {
        let probe = IdentityProbe {
            common_dir_key: Some(b"/w/x/.git".to_vec()),
            is_shallow: false,
            root_oids: vec!["B".to_owned(), "a".to_owned()],
            remote_urls: vec![
                (
                    "upstream".to_owned(),
                    "https://forge.example/acme/widget.git".to_owned(),
                ),
                (
                    "origin".to_owned(),
                    "git@forge.example:mine/widget.git".to_owned(),
                ),
            ],
        };
        let e = evidence_from(&probe);
        assert_eq!(e.remote_key.as_deref(), Some("forge.example/mine/widget"));
        assert_eq!(e.remote_owner.as_deref(), Some("mine"));
        assert!(e.lineage_key.is_some());

        let shallow = IdentityProbe {
            is_shallow: true,
            ..probe
        };
        assert_eq!(evidence_from(&shallow).lineage_key, None);
    }
}
