//! J1.5 — authorship (§4.1a).
//!
//! Runs immediately after J1 and BEFORE J2 and J3, because the most expensive repositories to
//! scan are shallow clones of large public projects, those are exactly the ones that resolve to
//! Reference, and gating cost 0.8% of what it saved.

use std::collections::BTreeMap;

use rusqlite::Transaction;

use super::JobError;
use crate::git::{GitBackend, JobContext, RepoHandle};
use crate::identity::user::IdentitySet;
use crate::index::IndexError;
use crate::protocol::ProjectId;

/// What the census concluded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthorshipFacts {
    /// `project_committer` rows: (email, commits), heaviest first (§1.4's row weight).
    ///
    /// This count is authorship *evidence* — §1.4's card judges an address by it — and is never
    /// a rendered figure. "Never reward volume" bans XP and figures derived from counts.
    pub committers: Vec<(String, u32)>,
    /// NULL = not computed. `Some(false)` ⇒ Reference (§5.5).
    pub authored_by_user: Option<bool>,
}

/// Fold committer addresses into the census.
///
/// An empty history leaves `authored_by_user` as `None`. A zero-commit repository is `empty`
/// (§5.4a), not somebody else's code: calling it Reference would exclude a brand-new repository
/// from every statistic on day one.
#[must_use]
pub fn tally(emails: impl Iterator<Item = String>, identities: &IdentitySet) -> AuthorshipFacts {
    tally_counted(emails.map(|e| (e, 1)), identities)
}

/// The same fold over addresses that are already counted, which is what `GitBackend::authorship`
/// hands back.
///
/// One body for both, because two folds over the same question is how a census and a re-census
/// end up disagreeing about who wrote a repository.
#[must_use]
pub fn tally_counted(
    counted: impl Iterator<Item = (String, u32)>,
    identities: &IdentitySet,
) -> AuthorshipFacts {
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    let mut mine = false;
    let mut any = false;
    for (email, commits) in counted {
        let e = email.trim().to_lowercase();
        if e.is_empty() || commits == 0 {
            continue;
        }
        any = true;
        if identities.contains(&e) {
            mine = true;
        }
        let slot = counts.entry(e).or_insert(0);
        *slot = slot.saturating_add(commits);
    }
    let mut committers: Vec<(String, u32)> = counts.into_iter().collect();
    committers.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    AuthorshipFacts {
        committers,
        authored_by_user: any.then_some(mine),
    }
}

/// §3.3: full walk, committer email, no cap. The `-1000` cap v1 used misclassifies a long-lived
/// repository whose own commits are older than the last thousand.
///
/// **The walk itself is plan 05's** `GitBackend::authorship`, which already tallies per address;
/// this re-folds it against the identity set, which is the part the git layer has no business
/// knowing about.
///
/// # Errors
///
/// `JobError::Git` carrying the walk's failure.
pub fn census(
    git: &dyn GitBackend,
    repo: &RepoHandle,
    identities: &IdentitySet,
    ctx: &JobContext<'_>,
) -> Result<AuthorshipFacts, JobError> {
    let walked = git.authorship(repo, ctx)?;
    Ok(tally_counted(
        walked.committers.into_iter().map(|t| (t.email, t.commits)),
        identities,
    ))
}

/// Write the census, and derive `is_reference` from it.
///
/// §5.5's derivation happens here and nowhere else. A NULL authorship writes neither column:
/// not computed is not "somebody else wrote it".
///
/// # Errors
///
/// `IndexError::Sqlite` when clearing, inserting or updating a row fails; the caller's
/// transaction then rolls the whole census back.
pub fn persist(
    tx: &Transaction<'_>,
    project: ProjectId,
    facts: &AuthorshipFacts,
) -> Result<(), IndexError> {
    tx.execute(
        "DELETE FROM project_committer WHERE project_id = ?1",
        [project.0],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO project_committer (project_id, email, commits) VALUES (?1, ?2, ?3)",
        )?;
        for (email, commits) in &facts.committers {
            stmt.execute(rusqlite::params![project.0, email, i64::from(*commits)])?;
        }
    }
    if let Some(mine) = facts.authored_by_user {
        tx.execute(
            "UPDATE project SET authored_by_user = ?2, is_reference = ?3 WHERE id = ?1",
            rusqlite::params![project.0, i64::from(mine), i64::from(!mine)],
        )?;
    }
    Ok(())
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

    fn ids(list: &[&str]) -> IdentitySet {
        IdentitySet::from_emails(list.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn zero_commits_by_the_user_is_reference() {
        // §5.5: committer emails over the FULL history against the identity set.
        let facts = tally(
            ["a@x".to_owned(), "b@x".to_owned(), "a@x".to_owned()].into_iter(),
            &ids(&["me@x"]),
        );
        assert_eq!(facts.authored_by_user, Some(false));
        assert_eq!(
            facts.committers,
            vec![("a@x".to_owned(), 2), ("b@x".to_owned(), 1)]
        );
    }

    #[test]
    fn one_commit_by_the_user_anywhere_in_history_is_enough() {
        let facts = tally(
            ["a@x".to_owned(), "me@x".to_owned(), "a@x".to_owned()].into_iter(),
            &ids(&["me@x"]),
        );
        assert_eq!(facts.authored_by_user, Some(true));
    }

    #[test]
    fn matching_is_case_insensitive_on_the_address() {
        let facts = tally(["ME@X".to_owned()].into_iter(), &ids(&["me@x"]));
        assert_eq!(facts.authored_by_user, Some(true));
    }

    #[test]
    fn committers_are_sorted_by_count_then_address_so_the_row_order_is_stable() {
        let facts = tally(
            [
                "b@x".to_owned(),
                "a@x".to_owned(),
                "b@x".to_owned(),
                "c@x".to_owned(),
            ]
            .into_iter(),
            &ids(&[]),
        );
        let names: Vec<&str> = facts.committers.iter().map(|(e, _)| e.as_str()).collect();
        assert_eq!(names, vec!["b@x", "a@x", "c@x"]);
    }

    #[test]
    fn an_empty_history_leaves_authorship_not_computed_never_reference() {
        // A zero-commit repository is `empty` (§5.4a), not "somebody else's code". Calling it
        // Reference would exclude a brand-new repository from every statistic on day one.
        let facts = tally(std::iter::empty(), &ids(&["me@x"]));
        assert_eq!(facts.authored_by_user, None);
        assert!(facts.committers.is_empty());
    }

    /// An unseeded identity set must not classify every repository as somebody else's. The
    /// answer is `Some(false)` only because the *set* is empty, which is why §10.4 seeds it
    /// before the first scan — recorded here so a later reader sees the dependency.
    #[test]
    fn an_empty_identity_set_matches_nobody() {
        let facts = tally(["a@x".to_owned()].into_iter(), &ids(&[]));
        assert_eq!(facts.authored_by_user, Some(false));
    }

    #[test]
    fn a_blank_committer_address_is_missing_data_not_the_user() {
        let facts = tally(
            [String::new(), "  ".to_owned()].into_iter(),
            &IdentitySet::from_emails([String::new()]),
        );
        assert_eq!(facts.authored_by_user, None);
        assert!(facts.committers.is_empty());
    }
}
