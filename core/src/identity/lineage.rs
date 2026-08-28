//! The identity ref set and the lineage digest (§1.1).

use sha2::{Digest as _, Sha256};

/// The identity ref set, stated (§1.1): `HEAD` plus all local branches plus all tags,
/// **excluding remote-tracking refs**, which churn on fetch (§6).
///
/// `include_head` is false only when `HEAD` does not resolve — an unborn branch — because a
/// bare `HEAD` argument then fails the whole invocation. The trailing `--` stops a ref whose
/// name looks like a path from being taken as a pathspec.
#[must_use]
pub fn root_set_argv(include_head: bool) -> Vec<&'static str> {
    let mut argv = vec!["rev-list", "--max-parents=0", "--branches", "--tags"];
    if include_head {
        argv.push("HEAD");
    }
    argv.push("--");
    argv
}

/// One OID per line. Anything that is not hex is discarded rather than trusted: this parses the
/// output of a process, and a warning on stdout must not become part of an identity.
#[must_use]
pub fn parse_root_oids(stdout: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(stdout);
    let mut oids: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .collect();
    oids.sort();
    oids.dedup();
    oids
}

/// SHA-256 of the sorted set of root commit OIDs, lowercase hex (§1.1).
///
/// **Deliberately not unique** — a fork and its upstream share it, and that is correct. The
/// digest is taken over the deduplicated, sorted OIDs joined by a single `\n` with no trailing
/// newline; multi-root repositories key on the whole set.
///
/// `None` in the two cases that have no lineage at all: no commits, and a shallow clone.
#[must_use]
pub fn lineage_key(root_oids: &[String], is_shallow: bool) -> Option<String> {
    use std::fmt::Write as _;

    if is_shallow || root_oids.is_empty() {
        return None;
    }
    let mut sorted: Vec<String> = root_oids.iter().map(|s| s.to_ascii_lowercase()).collect();
    sorted.sort();
    sorted.dedup();

    let mut hasher = Sha256::new();
    hasher.update(sorted.join("\n").as_bytes());

    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(hex, "{b:02x}");
    }
    Some(hex)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    #[test]
    fn the_ref_set_is_head_plus_local_branches_plus_tags_and_never_remote_tracking() {
        // `rev-list --max-parents=0 HEAD` misses roots reachable only from another branch
        // (§1.1). `--all` would pull in refs/remotes/**, which churn on every fetch (§6), so
        // the argv names --branches and --tags and never --remotes or --all.
        let argv = super::root_set_argv(true);
        assert_eq!(
            argv,
            vec![
                "rev-list",
                "--max-parents=0",
                "--branches",
                "--tags",
                "HEAD",
                "--"
            ]
        );
        assert!(!argv.contains(&"--all"));
        assert!(!argv.contains(&"--remotes"));

        // An unborn HEAD is not an argument git can resolve, so it is omitted rather than
        // letting the whole invocation fail.
        assert_eq!(
            super::root_set_argv(false),
            vec!["rev-list", "--max-parents=0", "--branches", "--tags", "--"]
        );
    }

    #[test]
    fn root_oids_are_lowercased_deduplicated_and_sorted() {
        let out = b"B2\nA1\nA1\n\n  C3  \nnot-an-oid\n";
        assert_eq!(super::parse_root_oids(out), vec!["a1", "b2", "c3"]);
    }

    #[test]
    fn the_key_is_the_digest_of_the_sorted_set_and_ignores_input_order() {
        let a = super::lineage_key(&["b".to_owned(), "a".to_owned()], false);
        let b = super::lineage_key(&["a".to_owned(), "b".to_owned(), "a".to_owned()], false);
        assert_eq!(a, b);
        assert_eq!(a.as_deref().map(str::len), Some(64));
        assert_ne!(a, super::lineage_key(&["a".to_owned()], false));
    }

    #[test]
    fn a_repository_with_no_commits_and_a_shallow_clone_both_have_no_lineage() {
        // No commits at all: identity is the location alone; no lineage, never merges (§1.1).
        assert_eq!(super::lineage_key(&[], false), None);
        // A shallow clone's parentless commits are the grafted boundary, not roots. Storing
        // one as a lineage would assert a history the repository does not have, and would
        // match any other clone cut at the same depth.
        assert_eq!(super::lineage_key(&["a".to_owned()], true), None);
    }
}
