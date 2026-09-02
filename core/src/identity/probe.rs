//! `IdentityProbe`, assembled through the git seam.
//!
//! **It takes no index and no lock, and that is the point.** This is the git half of the scan
//! hand-off: three invocations, none of them cheap on a large repository. A signature that
//! accepted an `Index` would let a later edit hold the one `rusqlite::Connection` across all
//! three, which is the failure R39 exists to prevent — `jobs::run_one` has the same shape for
//! the same reason.

use crate::git::{GitBackend, GitResult, JobContext, RepoHandle};
use crate::index::path::PathPlatform;
use crate::paths::path_key;

use super::decide::IdentityProbe;

/// Read §1.1's four identity facts for one repository.
///
/// **R2: `platform` is the *location's*, never the host's.** The plan's signature omits it. On a
/// Windows host scanning a distro, host-folding is wrong: two names differing only in case are
/// two directories inside the distro, and folding them together would make one project out of
/// two — the same defect R2 corrected in `path_key` itself.
///
/// # Errors
/// Any of the three reads may fail; the caller turns that into a `scan_problem` rather than
/// dropping the repository silently.
pub fn probe_identity(
    git: &dyn GitBackend,
    repo: &RepoHandle,
    platform: PathPlatform,
    ctx: &JobContext<'_>,
) -> GitResult<IdentityProbe> {
    let facts = git.repo_facts(repo, ctx)?;
    let roots = git.root_commits(repo, ctx)?;
    let remote_urls = git.remote_urls(repo, ctx)?;
    Ok(IdentityProbe {
        // **The handle's common dir, not `facts.common_dir`.** They name one directory, and
        // `location.common_dir_bytes` stores the handle's — so keying git's spelling instead
        // would make the key stored beside the row and the key identity was decided on two
        // spellings of one value, which is this project's dominant defect. Keyed with the
        // canonicaliser `location.path_key` uses (§1.3), so `worktree_owner`'s lookup compares
        // like with like.
        common_dir_key: Some(path_key(&repo.common_dir, platform)),
        is_shallow: facts.is_shallow,
        root_oids: roots.into_iter().map(|r| r.oid).collect(),
        remote_urls,
    })
}

#[cfg(all(test, feature = "testkit"))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::probe_identity;
    use crate::cancel::CancelToken;
    use crate::git::{JobClass, JobContext, RepoFacts, RepoHandle, RootCommit, StoreKey};
    use crate::identity::decide::evidence_from;
    use crate::index::path::PathPlatform;
    use crate::mount::StoreClass;
    use crate::testing::{FakeGitBackend, GitReply};

    fn handle(dir: &str) -> RepoHandle {
        let path = std::path::PathBuf::from(dir);
        RepoHandle {
            work_dir: path.clone(),
            git_dir: path.join(".git"),
            common_dir: path.join(".git"),
            store: StoreKey::new("store-a"),
            store_class: StoreClass::Local,
            trusted: false,
        }
    }

    fn facts(common_dir: &str, is_shallow: bool) -> RepoFacts {
        RepoFacts {
            is_bare: false,
            is_shallow,
            git_dir: std::path::PathBuf::from(common_dir),
            common_dir: std::path::PathBuf::from(common_dir),
        }
    }

    fn root(oid: &str) -> RootCommit {
        RootCommit {
            oid: oid.to_owned(),
            committed_at: 1_700_000_000,
            tz_offset_min: 0,
        }
    }

    #[test]
    fn a_shallow_clone_has_no_lineage_key() {
        let git = FakeGitBackend::new();
        git.always_repo_facts(GitReply::Ok(facts("/home/u/w/.git", true)));
        git.always_root_commits(GitReply::Ok(vec![root("aaaa")]));
        git.always_remote_urls(GitReply::Ok(vec![(
            "origin".to_owned(),
            "https://example.invalid/o/w.git".to_owned(),
        )]));
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);

        let probe = probe_identity(&git, &handle("/home/u/w"), PathPlatform::Unix, &ctx).unwrap();
        assert!(probe.is_shallow);
        let evidence = evidence_from(&probe);
        assert_eq!(
            evidence.lineage_key, None,
            "§1.1: a shallow clone's root set is whatever the depth cut left, so it is not a \
             lineage and must not key one"
        );
        assert_eq!(
            evidence.remote_key.as_deref(),
            Some("example.invalid/o/w"),
            "the remote is still evidence when the lineage is not"
        );
    }

    #[test]
    fn a_wsl_common_dir_is_not_case_folded() {
        let git = FakeGitBackend::new();
        git.always_repo_facts(GitReply::Ok(facts("/home/u/Widget/.git", false)));
        git.always_root_commits(GitReply::Ok(vec![root("bbbb")]));
        git.always_remote_urls(GitReply::Ok(Vec::new()));
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);

        let probe = probe_identity(&git, &handle("/home/u/Widget"), PathPlatform::Unix, &ctx)
            .unwrap()
            .common_dir_key
            .unwrap();
        assert!(
            String::from_utf8_lossy(&probe).contains("Widget"),
            "R2: inside a distro `Widget` and `widget` are two directories; folding them here \
             merges two projects into one"
        );
    }

    #[test]
    fn a_repository_with_no_commits_has_no_lineage_and_is_not_an_error() {
        let git = FakeGitBackend::new();
        git.always_repo_facts(GitReply::Ok(facts("/home/u/empty/.git", false)));
        git.always_root_commits(GitReply::Ok(Vec::new()));
        git.always_remote_urls(GitReply::Ok(Vec::new()));
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);

        let probe = probe_identity(&git, &handle("/home/u/empty"), PathPlatform::Unix, &ctx)
            .expect("an empty repository is a repository");
        assert!(probe.root_oids.is_empty());
        assert_eq!(evidence_from(&probe).lineage_key, None);
    }
}
