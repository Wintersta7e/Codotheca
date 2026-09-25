//! §45.3(a) and §47.4: the verifying read of one configured remote.
//!
//! **A remote covers a commit only by an advertisement read in this call.** Remote-tracking refs
//! are a cache that may predate a force-push or a deleted branch, and they never enter the
//! covered set. Per remote, in order:
//!
//! 1. **Resolve** — `ls-remote --get-url`, contacting nothing — and classify the effective URL.
//!    Only a *network* remote proceeds; a *same-machine* one contributes nothing and is not
//!    itself a blocker; anything else is *not admitted* and counts as a network remote that did
//!    not answer. **The resolved URL is never stored, logged or rendered**: it may carry a
//!    userinfo field.
//! 2. **Advertise** — `ls-remote`, every line parsed; one that does not parse fails the step.
//! 3. **Objects** — only when a `HEAD`, branch or tag tip is absent locally: those tips' objects,
//!    and no ref. Review refs and custom namespaces are never fetched.
//!
//! A remote **answered** iff step 2, and step 3 when it ran, exited 0 inside the deadline.

use crate::git::{GitBackend, GitError, JobContext, RepoHandle};
use crate::gitw::backend::MutatingGit;
use crate::gitw::intent::{AdvertisedRef, Intent, ObjectId, RemoteName, VerifyStep};

/// §45.3(a)'s three classes of remote, by effective URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteClass {
    /// https or ssh (scp form included) to a host that is not this machine. Its advertised
    /// objects cover commits when it answers.
    Network,
    /// `file://`, a local, relative or UNC path, or a loopback host. It contributes nothing to
    /// the covered set, and is not a blocker on its own.
    SameMachine,
    /// Any other transport — `git://`, `ext::`, `http://`. It counts as a network remote that
    /// did not answer.
    NotAdmitted,
}

/// Why a remote did not answer. Every class behaves as unknown — never as *gone*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DidNotAnswer {
    /// The configured name is not one safe segment, so no intent can name it.
    NameRefused,
    /// `ls-remote --get-url` failed or printed nothing.
    Unresolved,
    /// The resolved URL's transport is not https or ssh.
    NotAdmitted,
    /// git refused the transport the URL resolved to under the read's `GIT_ALLOW_PROTOCOL`.
    TransportRefused,
    /// A step outlived `GIT_INVOCATION_DEADLINE` and its process group was killed.
    Deadline,
    /// The advertisement did not parse — an object id that is not one, or a malformed line.
    Unparseable,
    /// A step exited non-zero: offline, a 401, 403 or 404, a refused key, a partial fetch.
    Failed,
    /// The local presence read failed, so what the remote covers cannot be established.
    Unverifiable,
}

/// One remote's reading, inside this call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteReading {
    /// The remote resolved to a copy on this machine.
    SameMachine,
    /// The remote answered.
    Answered {
        /// Every object id it advertised that is present locally after the read — the remote's
        /// share of §45.3's covered set `T`. A tag object's own id is here when it was advertised.
        present: Vec<String>,
        /// The tips the objects step fetched, by name, in order. Empty when nothing was absent.
        fetched: Vec<String>,
    },
    /// The remote did not answer, and why.
    DidNotAnswer(DidNotAnswer),
}

/// The analyser's network seam: read one configured remote of `repo` (§45.14 lets a test double
/// stand in for it; the analyser, the reads and the composition may not be doubled).
pub trait RemoteVerifier: std::fmt::Debug {
    /// Read the remote named `remote` — steps 1 to 3, one at a time.
    fn read(&self, repo: &RepoHandle, remote: &str, ctx: &JobContext<'_>) -> RemoteReading;
}

/// The production verifier: `VerifyRead` through the write seam, presence through the read seam.
pub struct GitRemoteVerifier<'a> {
    write_git: &'a dyn MutatingGit,
    git: &'a dyn GitBackend,
    /// Test-only: admits the fixture's local remotes as network (§47.9 C's one difference).
    #[cfg(feature = "testkit")]
    fixture: Option<crate::gitw::exec::TransportFixture>,
}

impl std::fmt::Debug for GitRemoteVerifier<'_> {
    /// By hand: it holds two seams whose contents are not a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitRemoteVerifier").finish_non_exhaustive()
    }
}

impl<'a> GitRemoteVerifier<'a> {
    /// A verifier over the product's two git seams.
    #[must_use]
    pub const fn new(write_git: &'a dyn MutatingGit, git: &'a dyn GitBackend) -> Self {
        Self {
            write_git,
            git,
            #[cfg(feature = "testkit")]
            fixture: None,
        }
    }

    /// The production verifier with the test's one permitted difference: a URL under the
    /// fixture's root is admitted as network. The write seam must carry the same fixture.
    #[cfg(feature = "testkit")]
    #[must_use]
    pub const fn with_transport_fixture(
        write_git: &'a dyn MutatingGit,
        git: &'a dyn GitBackend,
        fixture: crate::gitw::exec::TransportFixture,
    ) -> Self {
        Self {
            write_git,
            git,
            fixture: Some(fixture),
        }
    }

    fn classify(&self, url: &str) -> RemoteClass {
        #[cfg(feature = "testkit")]
        if self.fixture.as_ref().is_some_and(|f| f.admits(url)) {
            return RemoteClass::Network;
        }
        // Without the test fixture the verifier's own state plays no part in classification.
        #[cfg(not(feature = "testkit"))]
        let _ = self;
        classify_remote_url(url)
    }

    fn step(
        &self,
        repo: &RepoHandle,
        remote: &RemoteName,
        step: VerifyStep,
        ctx: &JobContext<'_>,
    ) -> Result<Vec<u8>, DidNotAnswer> {
        let intent = Intent::VerifyRead {
            repo: repo.work_dir.clone(),
            remote: remote.clone(),
            step,
        };
        // Stderr is read and dropped: it may carry a URL's userinfo, and it is never logged.
        self.write_git
            .run(&intent, ctx.cancel, &mut |_| {})
            .map(|out| out.stdout)
            .map_err(|error| match error {
                GitError::TransportRefused { .. } => DidNotAnswer::TransportRefused,
                GitError::Budget { .. } => DidNotAnswer::Deadline,
                _ => DidNotAnswer::Failed,
            })
    }

    fn read_network(
        &self,
        repo: &RepoHandle,
        remote: &RemoteName,
        ctx: &JobContext<'_>,
    ) -> Result<RemoteReading, DidNotAnswer> {
        let advertisement = self.step(repo, remote, VerifyStep::Advertise, ctx)?;
        let advertised = parse_advertisement(&advertisement).ok_or(DidNotAnswer::Unparseable)?;

        let mut oids: Vec<String> = advertised
            .iter()
            .map(|entry| entry.oid.as_str().to_owned())
            .collect();
        oids.sort_unstable();
        oids.dedup();
        let mut present = self
            .git
            .objects_present(repo, &oids, ctx)
            .map_err(|_| DidNotAnswer::Unverifiable)?;
        if present.len() != oids.len() {
            return Err(DidNotAnswer::Unverifiable);
        }
        let is_present = |flags: &[bool], oid: &str| {
            oids.binary_search_by(|probe| probe.as_str().cmp(oid))
                .ok()
                .and_then(|at| flags.get(at).copied())
                .unwrap_or(false)
        };

        // Step 3: the branch and tag tips whose objects are absent, and nothing else.
        let mut tips: Vec<AdvertisedRef> = Vec::new();
        let mut tip_oids: Vec<String> = Vec::new();
        for entry in &advertised {
            let Some(tip) = &entry.tip else { continue };
            if tip.is_branch_or_tag_tip()
                && !is_present(&present, entry.oid.as_str())
                && !tips.contains(tip)
            {
                tips.push(tip.clone());
                tip_oids.push(entry.oid.as_str().to_owned());
            }
        }
        let fetched: Vec<String> = tips.iter().map(|t| t.as_str().to_owned()).collect();
        if !tips.is_empty() {
            self.step(repo, remote, VerifyStep::Objects { tips }, ctx)?;
            // Exit 0 is not taken on trust: every fetched tip's object must now be here.
            let arrived = self
                .git
                .objects_present(repo, &tip_oids, ctx)
                .map_err(|_| DidNotAnswer::Unverifiable)?;
            if arrived.len() != tip_oids.len() || arrived.iter().any(|here| !here) {
                return Err(DidNotAnswer::Unverifiable);
            }
            for oid in &tip_oids {
                if let Ok(at) = oids.binary_search(oid) {
                    if let Some(slot) = present.get_mut(at) {
                        *slot = true;
                    }
                }
            }
        }

        let covered = oids
            .iter()
            .zip(&present)
            .filter(|(_, here)| **here)
            .map(|(oid, _)| oid.clone())
            .collect();
        Ok(RemoteReading::Answered {
            present: covered,
            fetched,
        })
    }
}

impl RemoteVerifier for GitRemoteVerifier<'_> {
    fn read(&self, repo: &RepoHandle, remote: &str, ctx: &JobContext<'_>) -> RemoteReading {
        let Ok(name) = RemoteName::parse(remote) else {
            return RemoteReading::DidNotAnswer(DidNotAnswer::NameRefused);
        };
        let resolved = match self.step(repo, &name, VerifyStep::ResolveUrl, ctx) {
            Ok(stdout) => String::from_utf8_lossy(&stdout).trim().to_owned(),
            Err(DidNotAnswer::Deadline) => {
                return RemoteReading::DidNotAnswer(DidNotAnswer::Deadline)
            }
            Err(_) => return RemoteReading::DidNotAnswer(DidNotAnswer::Unresolved),
        };
        if resolved.is_empty() {
            return RemoteReading::DidNotAnswer(DidNotAnswer::Unresolved);
        }
        match self.classify(&resolved) {
            RemoteClass::SameMachine => RemoteReading::SameMachine,
            RemoteClass::NotAdmitted => RemoteReading::DidNotAnswer(DidNotAnswer::NotAdmitted),
            RemoteClass::Network => self
                .read_network(repo, &name, ctx)
                .unwrap_or_else(RemoteReading::DidNotAnswer),
        }
    }
}

/// One line of the advertisement.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Advertised {
    oid: ObjectId,
    /// The ref as a fetchable tip, or `None` for a peeled `^{}` line or a name §47.2's grammar
    /// refuses — which still contributes its object id and is never a tip.
    tip: Option<AdvertisedRef>,
}

/// Parse `ls-remote`'s `<oid>\t<ref>` lines. `None` when any line fails: a partial parse is a
/// remote that did not answer, never a shorter advertisement.
fn parse_advertisement(stdout: &[u8]) -> Option<Vec<Advertised>> {
    let text = std::str::from_utf8(stdout).ok()?;
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let (oid, name) = line.split_once('\t')?;
        let oid = ObjectId::parse(oid.trim()).ok()?;
        let tip = if name.ends_with("^{}") {
            None
        } else {
            AdvertisedRef::parse(name.trim()).ok()
        };
        out.push(Advertised { oid, tip });
    }
    Some(out)
}

/// This machine's name, as far as it can be read without a system call: `COMPUTERNAME` on
/// Windows, `HOSTNAME` or `/etc/hostname` elsewhere. `None` when none is readable, and then only
/// the loopback names count as this machine.
fn this_host() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
}

/// Is `host` this machine — `localhost`, `127.0.0.0/8`, `::1`, or this host's own name?
fn is_loopback(host: &str) -> bool {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if host == "localhost" || host == "::1" || host.ends_with(".localhost") {
        return true;
    }
    if let Some(rest) = host.strip_prefix("127.") {
        let octets: Vec<&str> = rest.split('.').collect();
        if octets.len() == 3 && octets.iter().all(|o| o.parse::<u8>().is_ok()) {
            return true;
        }
    }
    this_host().is_some_and(|me| host == me || host.split('.').next() == Some(me.as_str()))
}

/// The host of an authority: userinfo and port stripped, IPv6 brackets kept for `is_loopback`.
fn host_of(authority: &str) -> &str {
    let without_user = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if without_user.starts_with('[') {
        return without_user
            .split_once(']')
            .map_or(without_user, |(host, _)| host);
    }
    without_user
        .split_once(':')
        .map_or(without_user, |(host, _)| host)
}

/// §45.3(a): classify a remote by its **effective** URL, as `ls-remote --get-url` resolved it.
#[must_use]
pub fn classify_remote_url(url: &str) -> RemoteClass {
    let url = url.trim();
    if let Some((scheme, rest)) = url.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if scheme == "file" {
            return RemoteClass::SameMachine;
        }
        if scheme != "https" && scheme != "ssh" {
            return RemoteClass::NotAdmitted;
        }
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = host_of(authority);
        if host.is_empty() {
            return RemoteClass::NotAdmitted;
        }
        return if is_loopback(host) {
            RemoteClass::SameMachine
        } else {
            RemoteClass::Network
        };
    }
    if url.contains("::") {
        // `ext::`, `fd::` and every other transport-helper spelling.
        return RemoteClass::NotAdmitted;
    }
    let bytes = url.as_bytes();
    let drive = bytes.get(1) == Some(&b':')
        && bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && matches!(bytes.get(2), Some(b'/' | b'\\') | None);
    if drive || url.starts_with('/') || url.starts_with('\\') || url.starts_with('.') {
        // An absolute, UNC or relative path on this machine.
        return RemoteClass::SameMachine;
    }
    // scp form, `[user@]host:path` — git's rule: a `:` before any `/`.
    match url.split_once(':') {
        Some((before, _)) if !before.contains('/') && !before.is_empty() => {
            let host = host_of(before);
            if is_loopback(host) {
                RemoteClass::SameMachine
            } else {
                RemoteClass::Network
            }
        }
        // No scheme and no scp host: a relative path.
        _ => RemoteClass::SameMachine,
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

    use super::{classify_remote_url, parse_advertisement, RemoteClass};

    #[test]
    fn a_remote_is_classified_by_its_effective_url() {
        for (url, class) in [
            (
                "https://forge.example/acme/widget.git",
                RemoteClass::Network,
            ),
            (
                "ssh://git@forge.example/acme/widget.git",
                RemoteClass::Network,
            ),
            ("git@forge.example:acme/widget.git", RemoteClass::Network),
            (
                "https://127.0.0.1/acme/widget.git",
                RemoteClass::SameMachine,
            ),
            ("https://127.8.9.10:8443/x.git", RemoteClass::SameMachine),
            ("https://localhost/x.git", RemoteClass::SameMachine),
            ("https://[::1]:3000/x.git", RemoteClass::SameMachine),
            ("ssh://user@localhost/x.git", RemoteClass::SameMachine),
            ("file:///srv/mirror.git", RemoteClass::SameMachine),
            ("/srv/mirror.git", RemoteClass::SameMachine),
            ("../mirror.git", RemoteClass::SameMachine),
            ("C:/mirrors/widget.git", RemoteClass::SameMachine),
            (r"C:\mirrors\widget.git", RemoteClass::SameMachine),
            (r"\\server\share\widget.git", RemoteClass::SameMachine),
            ("mirror", RemoteClass::SameMachine),
            (
                "git://forge.example/acme/widget.git",
                RemoteClass::NotAdmitted,
            ),
            (
                "http://forge.example/acme/widget.git",
                RemoteClass::NotAdmitted,
            ),
            ("ext::sh -c payload", RemoteClass::NotAdmitted),
        ] {
            assert_eq!(classify_remote_url(url), class, "{url}");
        }
    }

    #[test]
    fn an_advertisement_parses_whole_or_not_at_all() {
        let oid = "a".repeat(40);
        let good = format!(
            "{oid}\tHEAD\n{oid}\trefs/heads/main\n{oid}\trefs/tags/v1^{{}}\n{oid}\trefs/weird:name\n"
        );
        let parsed = parse_advertisement(good.as_bytes()).unwrap();
        assert_eq!(parsed.len(), 4);
        assert!(parsed[2].tip.is_none(), "a peeled line is never a tip");
        assert!(parsed[3].tip.is_none(), "a refused name is never a tip");
        assert!(parse_advertisement(b"not-an-oid\trefs/heads/main\n").is_none());
        assert!(parse_advertisement(b"no tab here\n").is_none());
    }
}
