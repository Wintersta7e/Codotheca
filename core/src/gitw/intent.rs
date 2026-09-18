//! §24.1a: the closed set of write intents, and the total function that renders each one.
//!
//! **Argv is never assembled from caller-supplied strings.** Every element of a rendered argv is
//! either a literal in this file or a value that passed through [`RemoteUrl`] or [`RemoteName`],
//! each of which refuses anything it cannot vouch for. Because [`Intent::argv`] is total over a
//! closed enum, `core/tests/git_write_audit.rs` **enumerates variants** rather than grepping
//! source text — which does not degrade when a subcommand is built from a variable, and which is
//! what the read side's audit could not do (`.dev/decisions/phase2/00-index.md`'s defect 5a).

use std::ffi::OsString;
use std::path::PathBuf;

#[cfg(feature = "testkit")]
use std::path::Path;

#[cfg(feature = "testkit")]
use crate::accounts::keychain::SecretToken;

/// Why a write intent could not be built. A refusal is a reply, not a failure (§24.3d).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentRefusal {
    /// The scheme is not `https`. §24.1c refuses `ssh://`, `git://`, `file://` and `ext::`:
    /// an SSH clone authenticates with the user's own agent and keys, which this product was
    /// never granted, cannot see in §20's tier model, and which are push-capable.
    NotHttps,
    /// Not an absolute `https` URL with a host, or carrying bytes no argv element may carry.
    MalformedUrl,
    /// The authority carries a userinfo field. §24.1c's worst rejected mechanism: it survives in
    /// the produced clone's `.git/config` after the operation ends.
    UrlCarriesUserinfo,
    /// A remote name that is not a single safe segment. A remote name is never a path.
    UnsafeRemoteName,
}

/// An `https` remote URL, validated at construction and carrying no userinfo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUrl(String);

impl RemoteUrl {
    /// Parse an `https` URL, refusing every other scheme and every userinfo field.
    ///
    /// **Hand-validated rather than parsed by `url`, and not because the crate is unavailable.**
    /// `url` 2.5.8 is already in `core/Cargo.lock` transitively through `reqwest`, in both
    /// targets' graphs, so promoting it to a direct dependency would add no new crate to either
    /// build. The reason is that **a parser is not a validator**: `url::Url` parses
    /// `https://user:pass@host/repo` perfectly happily, because userinfo is a legal URL
    /// component, so wrapping it would still need every check below and would add a parse that is
    /// not the security property.
    ///
    /// The security property is the **refusal set** — non-`https` scheme, userinfo, control
    /// characters, whitespace, and a leading `-`, which `git` reads as an option rather than as a
    /// URL. A URL type would never have been asked that last question.
    /// `core/src/identity/remote.rs` normalises remotes by hand for the same kind of reason.
    pub fn parse(raw: &str) -> Result<RemoteUrl, IntentRefusal> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(IntentRefusal::MalformedUrl);
        }
        // A value beginning with `-` is read by git as an option, not as a URL. Refusing it here
        // is what makes "argv is never assembled from caller-supplied strings" true of the URL
        // as well as of the literals around it.
        if trimmed.starts_with('-') {
            return Err(IntentRefusal::MalformedUrl);
        }
        if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(IntentRefusal::MalformedUrl);
        }
        let Some((scheme, rest)) = trimmed.split_once("://") else {
            // `ext::` and any other scheme-less spelling land here rather than below, which is
            // the same answer for a different reason and is deliberately not distinguished.
            return Err(IntentRefusal::NotHttps);
        };
        if !scheme.eq_ignore_ascii_case("https") {
            return Err(IntentRefusal::NotHttps);
        }
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if authority.is_empty() {
            return Err(IntentRefusal::MalformedUrl);
        }
        if authority.contains('@') {
            return Err(IntentRefusal::UrlCarriesUserinfo);
        }
        Ok(RemoteUrl(trimmed.to_owned()))
    }

    /// The URL as it is rendered into argv, byte for byte.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The authority, which is the host the credential helper answers for.
    #[must_use]
    pub fn host(&self) -> &str {
        self.0.split_once("://").map_or("", |(_, rest)| {
            rest.split(['/', '?', '#']).next().unwrap_or("")
        })
    }
}

/// A validated git remote name — one segment, never a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteName(String);

impl RemoteName {
    /// Parse a remote name, refusing anything that is not a single safe segment.
    pub fn parse(raw: &str) -> Result<RemoteName, IntentRefusal> {
        let safe = !raw.is_empty()
            && raw != "."
            && raw != ".."
            && !raw.starts_with('-')
            && raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if safe {
            Ok(RemoteName(raw.to_owned()))
        } else {
            Err(IntentRefusal::UnsafeRemoteName)
        }
    }

    /// The name as it is rendered into argv, byte for byte.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The discriminant of an [`Intent`], with no payload.
///
/// It exists so [`Intent::ALL`] can be an **iterable** exhaustive list rather than a count: a
/// `usize` satisfies the letter of *"`Intent::ALL.len()` equals a hard-coded number"* while
/// rendering nothing, and the audit's whole claim is that every variant was rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentKind {
    /// `git clone` — the destination must not exist before the call.
    Clone,
    /// `git fetch` — never with `--prune`.
    Fetch,
}

/// §24.1's whole git write surface: two subcommands, and nothing else reaches a child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Clone into a destination that does not yet exist.
    Clone {
        /// The `https` remote, carrying no userinfo.
        url: RemoteUrl,
        /// Where the clone lands. Composed by the core, never by the renderer (§24.3a).
        dest: PathBuf,
        /// `--depth`, when a shallow clone is wanted.
        depth: Option<u32>,
    },
    /// Fetch from an already-configured remote.
    Fetch {
        /// The repository the fetch runs in, rendered as `-C <work_dir>` by the write path.
        ///
        /// **[p2-24b] A field rather than an argument, and that is the whole point.** p2-24 left
        /// this variant carrying only a remote name and made `SystemMutatingGit::run` refuse it,
        /// because a fetch without a repository *"would fetch in whatever the process's working
        /// directory happens to be, which is a write into a repository nobody named"*. Supplying
        /// it through `WriteEnv` instead would leave that state constructible — an `Intent::Fetch`
        /// beside an env with no `work_dir` compiles and runs. Here it cannot be built at all.
        work_dir: PathBuf,
        /// The remote's name, never a URL and never a path.
        remote: RemoteName,
    },
}

impl Intent {
    /// The exhaustive discriminant list. Its length is a deliberate tripwire on its own growth,
    /// and `core/tests/git_write_audit.rs` pins it.
    pub const ALL: [IntentKind; 2] = [IntentKind::Clone, IntentKind::Fetch];

    /// Render this intent's argv. **Total over the variant** — no `_ =>` arm, and no
    /// caller-supplied string reaches argv unrendered.
    ///
    /// `--progress` is here because git writes progress to **stderr** only when it is not
    /// attached to a terminal and is asked to; §24.4's stage machine parses that stream.
    #[must_use]
    pub fn argv(&self) -> Vec<OsString> {
        match self {
            Intent::Clone { url, dest, depth } => {
                let mut argv = vec![OsString::from("clone"), OsString::from("--progress")];
                if let Some(depth) = *depth {
                    argv.push(OsString::from("--depth"));
                    argv.push(OsString::from(depth.to_string()));
                }
                argv.push(OsString::from(url.as_str()));
                argv.push(dest.clone().into_os_string());
                argv
            }
            // `-C` is **not** rendered here. It is a base argument, prepended by
            // `write_base_args` before the subcommand, and `git_write_audit.rs` reads `argv[0]`
            // as the subcommand — a `-C` in front of `fetch` would make the audit read a path
            // where it looks for a write-allowed verb.
            Intent::Fetch { remote, .. } => vec![
                OsString::from("fetch"),
                OsString::from("--progress"),
                OsString::from(remote.as_str()),
            ],
        }
    }

    /// The repository this intent runs in, when it has one.
    ///
    /// `None` for a clone: its destination **does not exist yet**, which is §24.1's precondition,
    /// and rendering `-C` for it would name a directory git is about to create.
    #[must_use]
    pub fn work_dir(&self) -> Option<&std::path::Path> {
        match self {
            Intent::Clone { .. } => None,
            Intent::Fetch { work_dir, .. } => Some(work_dir.as_path()),
        }
    }

    /// The discriminant of this intent, for an audit that reports which variant it rendered.
    #[must_use]
    pub fn kind(&self) -> IntentKind {
        match self {
            Intent::Clone { .. } => IntentKind::Clone,
            Intent::Fetch { .. } => IntentKind::Fetch,
        }
    }

    /// One fully-populated intent per [`IntentKind`], over a fixture carrying the sentinel
    /// credential the audit scans for.
    ///
    /// The `match` below is **exhaustive over `IntentKind`**, so a variant added without a case
    /// here fails to compile. That is strictly stronger than the count in [`Intent::ALL`], and it
    /// is why both are asserted: the count alone is a number somebody can raise, and this is a
    /// build failure nobody can miss.
    #[cfg(feature = "testkit")]
    #[must_use]
    pub fn all_for_audit(fixture: &AuditFixture) -> Vec<Intent> {
        Intent::ALL
            .into_iter()
            .map(|kind| match kind {
                IntentKind::Clone => Intent::Clone {
                    url: fixture.url().clone(),
                    dest: fixture.dest().to_path_buf(),
                    depth: Some(1),
                },
                IntentKind::Fetch => Intent::Fetch {
                    work_dir: fixture.dest().to_path_buf(),
                    remote: fixture.remote().clone(),
                },
            })
            .collect()
    }
}

/// What `core/tests/git_write_audit.rs` renders every variant against.
///
/// It carries a **sentinel** token rather than a real one, so assertion 3 can scan every argv
/// element, every `-c` value, every URL and every environment entry of the child the write path
/// would spawn and prove the sentinel appears in none of them. A fixture that held no credential
/// would make that assertion vacuous.
///
/// The host is a reserved example name and names no real forge.
#[cfg(feature = "testkit")]
#[derive(Debug)]
pub struct AuditFixture {
    url: RemoteUrl,
    dest: PathBuf,
    remote: RemoteName,
    token: SecretToken,
}

#[cfg(feature = "testkit")]
impl AuditFixture {
    /// Build the fixture under `root`, which must be a directory the caller owns.
    ///
    /// The destination is a path under `root` that **does not exist**, which is §24.1's
    /// precondition for `clone` and what assertion 5 checks.
    pub fn new(root: &Path, token: SecretToken) -> Result<AuditFixture, IntentRefusal> {
        Ok(AuditFixture {
            url: RemoteUrl::parse("https://forge.example/acme/widget.git")?,
            dest: root.join("widget"),
            remote: RemoteName::parse("origin")?,
            token,
        })
    }

    /// The sentinel credential every variant is rendered with.
    #[must_use]
    pub fn token(&self) -> &SecretToken {
        &self.token
    }

    /// The fixture's remote URL.
    #[must_use]
    pub fn url(&self) -> &RemoteUrl {
        &self.url
    }

    /// The fixture's clone destination, which does not exist.
    #[must_use]
    pub fn dest(&self) -> &Path {
        &self.dest
    }

    /// The fixture's remote name.
    #[must_use]
    pub fn remote(&self) -> &RemoteName {
        &self.remote
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

    use std::path::{Path, PathBuf};

    use super::{Intent, IntentRefusal, RemoteName, RemoteUrl};

    #[test]
    fn every_non_https_scheme_is_refused_as_such() {
        for raw in [
            "ssh://git@forge.example/acme/widget.git",
            "git://forge.example/acme/widget.git",
            "file:///tmp/widget",
            "http://forge.example/acme/widget.git",
            // No `://`, so it cannot name a scheme this accepts. `protocol.ext.allow=never` is
            // the second half of the same refusal and is set on every invocation.
            "ext::payload",
        ] {
            assert_eq!(
                RemoteUrl::parse(raw),
                Err(IntentRefusal::NotHttps),
                "{raw} must be refused as a non-https scheme"
            );
        }
    }

    /// The refusal **variant** depends on which guard a spelling trips first, and the security
    /// claim does not. A realistic `ext::` remote carries arguments — `ext::sh -c payload`,
    /// `ext::git-server %S repo` — so it meets the whitespace guard before the scheme guard and
    /// comes back `MalformedUrl` rather than `NotHttps`.
    ///
    /// Written as its own test after the tighter one above caught the difference: asserting one
    /// variant over a mixed set would have had to be *loosened* to pass, and loosening an
    /// assertion to make it green is how a guard stops meaning anything. What matters is that
    /// **no spelling is accepted**, and that is what this asserts.
    #[test]
    fn an_ext_remote_carrying_arguments_is_refused_whichever_guard_catches_it() {
        for raw in [
            "ext::sh -c payload",
            "ext::git-server %S repo",
            "  ext::sh -c payload  ",
        ] {
            assert!(
                RemoteUrl::parse(raw).is_err(),
                "{raw:?} must not produce a RemoteUrl"
            );
        }
    }

    #[test]
    fn a_userinfo_field_is_refused_on_its_own_terms() {
        assert_eq!(
            RemoteUrl::parse("https://token@forge.example/acme/widget.git"),
            Err(IntentRefusal::UrlCarriesUserinfo),
            "userinfo survives on disk in the clone's .git/config and must never be built"
        );
    }

    #[test]
    fn a_url_that_would_read_as_an_option_is_refused() {
        assert_eq!(
            RemoteUrl::parse("--upload-pack=payload"),
            Err(IntentRefusal::MalformedUrl)
        );
    }

    #[test]
    fn a_remote_name_is_one_segment_and_never_a_path() {
        assert!(RemoteName::parse("origin").is_ok());
        for raw in ["", ".", "..", "-x", "a/b", "a\\b", "a b", "a:b"] {
            assert_eq!(
                RemoteName::parse(raw),
                Err(IntentRefusal::UnsafeRemoteName),
                "{raw:?} must be refused as a remote name"
            );
        }
    }

    #[test]
    fn the_host_is_the_authority_and_carries_no_path() {
        let url = RemoteUrl::parse("https://forge.example/acme/widget.git").unwrap();
        assert_eq!(url.host(), "forge.example");
    }

    #[test]
    fn a_fetch_renders_its_remote_and_no_flag_beyond_progress() {
        let intent = Intent::Fetch {
            work_dir: PathBuf::from("/srv/work/thing"),
            remote: RemoteName::parse("origin").unwrap(),
        };
        let argv: Vec<String> = intent
            .argv()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        // The repository is **not** in here: `-C` is a base argument and the audit reads
        // `argv[0]` as the subcommand, so a path in front of `fetch` would be read as a verb.
        assert_eq!(argv, vec!["fetch", "--progress", "origin"]);
    }

    /// [p2-24b] §24.7C's fetch names the repository it runs in, by type.
    ///
    /// The variant that could not say where it ran is the one p2-24 refused to execute, and the
    /// refusal is gone now because the state it guarded against is unconstructible.
    #[test]
    fn a_fetch_names_the_repository_it_runs_in_and_a_clone_does_not() {
        let fetch = Intent::Fetch {
            work_dir: PathBuf::from("/srv/work/thing"),
            remote: RemoteName::parse("origin").unwrap(),
        };
        assert_eq!(fetch.work_dir(), Some(Path::new("/srv/work/thing")));

        let clone = Intent::Clone {
            url: RemoteUrl::parse("https://forge.example/acme/widget.git").unwrap(),
            dest: PathBuf::from("/srv/work/new"),
            depth: None,
        };
        // A clone's destination does not exist yet, so `-C` would name a directory git is about
        // to create.
        assert_eq!(clone.work_dir(), None);
    }
}
