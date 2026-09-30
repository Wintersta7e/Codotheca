//! R249: whether the installed git can be stopped from lazily fetching, and whether a repository
//! has a promisor remote it would fetch from.
//!
//! A read of a partial clone asks the promisor remote for any object the clone lacks: a network
//! write from the read path, over whatever transport the remote names. `GIT_NO_LAZY_FETCH=1`
//! (pinned on every read child, [`super::invocation::pin_no_lazy_fetch`]) stops it where git
//! honours the variable; the governed floor, 2.29, ignores it (measured: 2.29 fetches with the pin
//! set, 2.43 and 2.55 do not). So the answer is probed on the git in use, never read from a version
//! number (R246), and where the pin is ignored a repository with a promisor remote is refused.
//!
//! The probe runs only when a read meets a promisor remote, so a library with no partial clone
//! never pays for it.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::time::Duration;

use crate::cancel::CancelToken;
use crate::mount::StoreClass;

use super::exec::{GitExec, RunLimits};
use super::repo::{RepoHandle, StoreKey};

/// What the git in use does with `GIT_NO_LAZY_FETCH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LazyFetch {
    /// Honoured: a read of a partial clone answers an absent object as missing and fetches
    /// nothing, so the read proceeds.
    Pinned,
    /// Ignored, or the probe could not tell: a read of a repository with a promisor remote could
    /// fetch, so the read seam refuses it with `GitError::LazyFetchUnguarded`.
    Unguarded,
}

/// The probe repository's leaf name, created beside the empty hooks directory.
pub const LAZY_FETCH_PROBE_DIR_NAME: &str = "git-lazy-fetch-probe";

const PROBE_HEAD: &str = "ref: refs/heads/main\n";

/// A partial clone of a remote that does not exist. The URL is relative to the probe directory,
/// which never holds that name, so a git that fetches anyway fails at once and locally.
const PROBE_CONFIG: &str = "[core]\n\trepositoryformatversion = 1\n\tbare = false\n\
[extensions]\n\tpartialClone = origin\n\
[remote \"origin\"]\n\turl = no-such-remote\n\tpromisor = true\n";

/// An object id no repository holds.
const ABSENT_OID: &str = "1111111111111111111111111111111111111111";

/// Ask `exec`'s git, once, whether it honours the pin.
///
/// It reads an absent object from a file-built partial clone with the read seam's pins and with
/// trace2 events on stderr, and looks for a `fetch` child. **Fails closed**: a probe that cannot
/// write its repository, run git, or hear trace2 at all answers [`LazyFetch::Unguarded`].
pub(crate) fn probe(exec: &GitExec) -> LazyFetch {
    probe_answer(exec).unwrap_or(LazyFetch::Unguarded)
}

fn probe_answer(exec: &GitExec) -> Option<LazyFetch> {
    let dir = exec.hooks_dir().with_file_name(LAZY_FETCH_PROBE_DIR_NAME);
    write_probe_repo(&dir).ok()?;
    let repo =
        RepoHandle::resolve(&dir, StoreKey::new("lazy-fetch-probe"), StoreClass::Local).ok()?;
    let out = exec
        .run_probe(
            &repo,
            &[
                OsStr::new("cat-file"),
                OsStr::new("-p"),
                OsStr::new(ABSENT_OID),
            ],
            &[("GIT_TRACE2_EVENT", OsString::from("1"))],
            // `cat-file -p` of an absent object exits 128 whether or not git tried a fetch.
            RunLimits::after(Duration::from_secs(10)).tolerating(128),
            &CancelToken::new(),
        )
        .ok()?;
    trace_answer(&String::from_utf8_lossy(&out.stderr))
}

/// Read trace2's event stream: a `child_start` whose argv holds `fetch` means the pin was ignored.
/// A stream holding no `version` event is one git never wrote, and proves nothing.
fn trace_answer(events: &str) -> Option<LazyFetch> {
    if !events
        .lines()
        .any(|line| line.contains("\"event\":\"version\""))
    {
        return None;
    }
    let fetched = events
        .lines()
        .any(|line| line.contains("\"event\":\"child_start\"") && line.contains("\"fetch\""));
    Some(if fetched {
        LazyFetch::Unguarded
    } else {
        LazyFetch::Pinned
    })
}

fn write_probe_repo(dir: &Path) -> std::io::Result<()> {
    let git_dir = dir.join(".git");
    std::fs::create_dir_all(git_dir.join("objects"))?;
    std::fs::create_dir_all(git_dir.join("refs").join("heads"))?;
    write_if_changed(&git_dir.join("HEAD"), PROBE_HEAD)?;
    write_if_changed(&git_dir.join("config"), PROBE_CONFIG)
}

/// Rewrite only on a difference, so a probe that finds its repository already in place writes
/// nothing a concurrent reader could catch half-written.
fn write_if_changed(path: &Path, text: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|now| now == text) {
        return Ok(());
    }
    std::fs::write(path, text)
}

/// Whether git would treat `repo` as having a promisor remote: `extensions.partialClone`, or a
/// `remote.<name>.promisor` set true, in its config or its linked worktree's config.
///
/// Read from the files, so it spawns nothing. A config that does not exist names no promisor. It
/// **fails closed** otherwise: a config it cannot read, a malformed header, or an `include`
/// counts as a promisor, because an answer here is only ever asked where git would fetch.
pub(crate) fn has_promisor(repo: &RepoHandle) -> bool {
    config_file_names_promisor(&repo.common_dir.join("config"))
        || config_file_names_promisor(&repo.git_dir.join("config.worktree"))
}

fn config_file_names_promisor(path: &Path) -> bool {
    match std::fs::read_to_string(path) {
        Ok(text) => config_names_promisor(&text),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// One config file's text, read for the two keys that make a promisor remote.
fn config_names_promisor(text: &str) -> bool {
    let mut section = String::new();
    for raw in text.lines() {
        let mut line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let Some((header, tail)) = rest.split_once(']') else {
                return true;
            };
            section = header
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if section == "include" || section == "includeif" {
                return true;
            }
            // A key may follow its header on the same line: `[extensions] partialClone = x`.
            line = tail.trim();
            if line.is_empty() {
                continue;
            }
        }
        let (key, value) = line
            .split_once('=')
            .map_or((line, None), |(k, v)| (k, Some(v)));
        let key = key.trim().to_ascii_lowercase();
        if section == "extensions" && key == "partialclone" {
            return true;
        }
        // `map_or`, not `is_none_or`: that is 1.82 and the floor is 1.80.
        if section.starts_with("remote") && key == "promisor" && value.map_or(true, is_true) {
            return true;
        }
    }
    false
}

/// git's boolean for a `key = value` line. Unrecognisable text counts as true: git would refuse
/// it outright, and this reader only ever fails closed.
fn is_true(value: &str) -> bool {
    let bare = value
        .split(['#', ';'])
        .next()
        .unwrap_or_default()
        .trim()
        .trim_matches('"')
        .to_ascii_lowercase();
    match bare.as_str() {
        "false" | "no" | "off" | "0" | "" => false,
        // A number is true unless zero; anything unparseable is `Err`, so true — failing closed.
        other => other.parse::<i64>() != Ok(0),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{config_names_promisor, is_true, trace_answer, LazyFetch};

    #[test]
    fn a_clone_config_with_partial_clone_or_a_promisor_names_one() {
        assert!(config_names_promisor(
            "[core]\n\tbare = false\n[extensions]\n\tpartialClone = origin\n"
        ));
        assert!(config_names_promisor(
            "[remote \"origin\"]\n\turl = x\n\tpromisor = true\n"
        ));
        assert!(config_names_promisor("[remote \"o\"]\n\tpromisor\n"));
        assert!(config_names_promisor("[Remote \"o\"] Promisor = yes\n"));
        assert!(config_names_promisor("[extensions] partialclone = o\n"));
    }

    #[test]
    fn an_ordinary_config_names_none() {
        assert!(!config_names_promisor(
            "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = x\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n"
        ));
        assert!(!config_names_promisor(
            "[remote \"o\"]\n\tpromisor = false\n"
        ));
        assert!(!config_names_promisor(
            "[remote \"o\"]\n\tpartialclonefilter = blob:none\n"
        ));
        assert!(!config_names_promisor(
            "# [extensions]\n; partialClone = o\n"
        ));
        assert!(!config_names_promisor(""));
    }

    #[test]
    fn what_it_cannot_read_counts_as_a_promisor() {
        assert!(config_names_promisor("[include]\n\tpath = other\n"));
        assert!(config_names_promisor(
            "[includeIf \"gitdir:x\"]\n\tpath = y\n"
        ));
        assert!(config_names_promisor("[remote \"o\"\n\tpromisor = false\n"));
        assert!(is_true("maybe"));
    }

    #[test]
    fn booleans_read_as_git_reads_them() {
        for yes in ["true", "Yes", "on", "1", "2", "\"true\"", "true # note"] {
            assert!(is_true(yes), "{yes}");
        }
        for no in ["false", "no", "OFF", "0", ""] {
            assert!(!is_true(no), "{no}");
        }
    }

    #[test]
    fn a_fetch_child_in_the_trace_means_the_pin_was_ignored() {
        let version = r#"{"event":"version","evt":"3","exe":"2.29.0"}"#;
        let fetch = r#"{"event":"child_start","argv":["git","-c","fetch.negotiationAlgorithm=noop","fetch","origin"]}"#;
        assert_eq!(
            trace_answer(&format!("{version}\n{fetch}\n")),
            Some(LazyFetch::Unguarded)
        );
        assert_eq!(
            trace_answer(&format!("{version}\n")),
            Some(LazyFetch::Pinned)
        );
        assert_eq!(
            trace_answer(fetch),
            None,
            "no version event: git never traced"
        );
        assert_eq!(trace_answer(""), None);
    }
}
