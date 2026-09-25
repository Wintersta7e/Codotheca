//! §45.6 step 1: **is this directory still the repository the row describes?**
//!
//! The lineage is re-derived from disk with the one derivation that writes
//! `project.lineage_key` — [`GitBackend::repo_facts`] for shallowness, [`GitBackend::root_commits`]
//! for the roots, then [`lineage_key`] — the inputs `identity::probe::probe_identity` feeds
//! `identity::decide::evidence_from`. Two derivations could disagree; one cannot.
//!
//! **The comparison is against the row, never against the copy.** Phase 2 built the uninstall
//! warrant's expected identity from the directory it was about to check, so a directory replaced
//! by another repository matched itself (§37.8). The row's `lineage_key`, read in this call, is
//! the only thing the live derivation is compared with.

use crate::git::{GitBackend, JobContext, RepoHandle};
use crate::identity::lineage::lineage_key;

/// Step 1's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityOutcome {
    /// The live lineage is the row's — including two NULLs, which match only for a repository
    /// with no commits that is not shallow.
    Match,
    /// A different repository, or one with commits where the row recorded none: `refused_path`,
    /// and the analysis stops.
    Mismatch,
    /// The live repository is shallow, so it has no lineage to compare (D-2): it yields
    /// `shallow_clone` and the analysis continues locally.
    Shallow,
    /// The derivation failed: `refs_unreadable`, and the analysis stops.
    Unreadable,
}

/// The identity re-derived at the moment of removal, for the warrant to compare against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveIdentity {
    /// The lineage, derived. `None` is a repository with no commits that is not shallow.
    Derived(Option<String>),
    /// Nothing could be derived — the read failed, or the repository is shallow and has no
    /// lineage. Never a match.
    Underivable,
}

/// The one derivation, before it is compared with anything.
enum Derivation {
    Lineage(Option<String>),
    Shallow,
    Failed,
}

fn derive(git: &dyn GitBackend, repo: &RepoHandle, ctx: &JobContext<'_>) -> Derivation {
    let Ok(facts) = git.repo_facts(repo, ctx) else {
        return Derivation::Failed;
    };
    if facts.is_shallow {
        return Derivation::Shallow;
    }
    git.root_commits(repo, ctx)
        .map_or(Derivation::Failed, |roots| {
            let oids: Vec<String> = roots.into_iter().map(|root| root.oid).collect();
            Derivation::Lineage(lineage_key(&oids, false))
        })
}

/// §45.6 step 1: compare `repo`'s live lineage with the row's `lineage_key`.
///
/// A separate public function so every act that must know it is looking at the same repository
/// asks the same question the same way.
#[must_use]
pub fn identify(
    git: &dyn GitBackend,
    repo: &RepoHandle,
    row_lineage: Option<&str>,
    ctx: &JobContext<'_>,
) -> IdentityOutcome {
    match derive(git, repo, ctx) {
        Derivation::Lineage(live) if live.as_deref() == row_lineage => IdentityOutcome::Match,
        Derivation::Lineage(_) => IdentityOutcome::Mismatch,
        Derivation::Shallow => IdentityOutcome::Shallow,
        Derivation::Failed => IdentityOutcome::Unreadable,
    }
}

/// The same derivation, kept rather than compared: what `remove_warranted` checks the warrant's
/// row lineage against at the moment of removal.
#[must_use]
pub fn live_identity(
    git: &dyn GitBackend,
    repo: &RepoHandle,
    ctx: &JobContext<'_>,
) -> LiveIdentity {
    match derive(git, repo, ctx) {
        Derivation::Lineage(live) => LiveIdentity::Derived(live),
        Derivation::Shallow | Derivation::Failed => LiveIdentity::Underivable,
    }
}
