//! §24.1a: the closed set of write intents, and the total function that renders each one.
//!
//! **Argv is never assembled from caller-supplied strings.** Every element of a rendered argv is
//! either a literal in this file or a value that passed through [`RemoteUrl`] or [`RemoteName`],
//! each of which refuses anything it cannot vouch for. Because [`Intent::argv`] is total over a
//! closed enum, `core/tests/git_write_audit.rs` **enumerates variants** rather than grepping
//! source text — which does not degrade when a subcommand is built from a variable, and which is
//! what the read side's audit could not do (`.dev/decisions/phase2/00-index.md`'s defect 5a).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(feature = "testkit")]
use crate::accounts::keychain::SecretToken;

/// §47.3's per-invocation deadline: an intent that runs while a user waits is killed, with its
/// whole process group, when this elapses.
///
/// **A declared value, not a measurement.** It is the 20 s the uninstall pre-flight declared for
/// its fetch and nothing ever enforced; the Windows-native measurement that confirms or replaces
/// it is still owed, and until then no latency may be quoted from it. The analyser's reads take
/// the same value, so one per-invocation deadline governs the whole verdict (§45.6).
pub const GIT_INVOCATION_DEADLINE: Duration = Duration::from_secs(20);

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
    /// Not `HEAD` and not a `refs/` name git's ref-name rules admit (§47.2's `AdvertisedRef`).
    /// A stdin line is argv by another channel: a destination on it wrote a ref (§47 M5).
    UnsafeRefName,
    /// Not 40 or 64 lowercase hex characters.
    NotAnObjectId,
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
    ///
    /// # Errors
    /// [`IntentRefusal::NotHttps`] for any scheme but `https`, or none;
    /// [`IntentRefusal::UrlCarriesUserinfo`] for an `@` in the authority; and
    /// [`IntentRefusal::MalformedUrl`] for an empty value, a leading `-`, whitespace or a control
    /// character, or an empty authority.
    pub fn parse(raw: &str) -> Result<Self, IntentRefusal> {
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
        Ok(Self(trimmed.to_owned()))
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
    ///
    /// # Errors
    /// [`IntentRefusal::UnsafeRemoteName`] for an empty name, `.`, `..`, a leading `-`, or any
    /// character outside ASCII letters, digits, `-`, `_` and `.`.
    pub fn parse(raw: &str) -> Result<Self, IntentRefusal> {
        let safe = !raw.is_empty()
            && raw != "."
            && raw != ".."
            && !raw.starts_with('-')
            && raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if safe {
            Ok(Self(raw.to_owned()))
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

/// One ref name a remote advertised, fit to be a source on the objects step's stdin (§47.2).
///
/// **Built only from one line of `ls-remote`'s output.** It refuses anything but `HEAD` or a
/// `refs/` name git's ref-name rules admit: no `:`, `*`, `?`, `[`, `^`, `~`, `\`, space or control
/// byte, no `..` or `@{`, no empty component, none starting with `.` or ending with `.lock`, and
/// no trailing `/` or `.`. The `:` is the one that matters most: a stdin line
/// `refs/heads/main:refs/heads/injected` wrote a ref under every other pin (§47 M5), so the
/// grammar, not a flag, is what keeps the objects step objects-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvertisedRef(String);

impl AdvertisedRef {
    /// Parse one advertised ref name.
    ///
    /// # Errors
    /// [`IntentRefusal::UnsafeRefName`] for anything §47.2's grammar refuses.
    pub fn parse(raw: &str) -> Result<Self, IntentRefusal> {
        if raw == "HEAD" {
            return Ok(Self(raw.to_owned()));
        }
        let forbidden_byte = raw.bytes().any(|b| {
            b.is_ascii_control()
                || matches!(b, b':' | b'*' | b'?' | b'[' | b'^' | b'~' | b'\\' | b' ')
        });
        let bad_component = raw.split('/').any(|component| {
            component.is_empty()
                || component.starts_with('.')
                || Path::new(component)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("lock"))
        });
        let safe = raw.starts_with("refs/")
            && !forbidden_byte
            && !bad_component
            && !raw.contains("..")
            && !raw.contains("@{")
            && !raw.ends_with('.');
        if safe {
            Ok(Self(raw.to_owned()))
        } else {
            Err(IntentRefusal::UnsafeRefName)
        }
    }

    /// The name as it is written to stdin, byte for byte.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Is this a tip the objects step may fetch — `HEAD`, a branch or a tag (§47.4 step 3)?
    /// Review refs and custom namespaces are never fetched.
    #[must_use]
    pub fn is_branch_or_tag_tip(&self) -> bool {
        self.0 == "HEAD" || self.0.starts_with("refs/heads/") || self.0.starts_with("refs/tags/")
    }
}

/// An object id: 40 or 64 lowercase hex characters, the shape the read path's
/// [`crate::git::is_object_id`] owns.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectId(String);

impl ObjectId {
    /// Parse an object id from a read.
    ///
    /// # Errors
    /// [`IntentRefusal::NotAnObjectId`] for anything but 40 or 64 lowercase hex characters.
    pub fn parse(raw: &str) -> Result<Self, IntentRefusal> {
        if crate::git::is_object_id(raw) {
            Ok(Self(raw.to_owned()))
        } else {
            Err(IntentRefusal::NotAnObjectId)
        }
    }

    /// The id as hex.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The verifying read's three steps (§47.4), one child each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyStep {
    /// `ls-remote --get-url <remote>`: the effective URL after `insteadOf`, contacting nothing.
    ResolveUrl,
    /// `ls-remote <remote>`: the advertisement.
    Advertise,
    /// `fetch --stdin …`: the objects of the branch and tag tips absent locally, and no ref.
    Objects {
        /// The tips to fetch, one stdin line each.
        tips: Vec<AdvertisedRef>,
    },
}

/// The discriminant of a [`VerifyStep`], for the audit's exhaustive rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyStepKind {
    /// [`VerifyStep::ResolveUrl`].
    ResolveUrl,
    /// [`VerifyStep::Advertise`].
    Advertise,
    /// [`VerifyStep::Objects`].
    Objects,
}

impl VerifyStep {
    /// Every step, for the audit to render each one.
    pub const ALL: [VerifyStepKind; 3] = [
        VerifyStepKind::ResolveUrl,
        VerifyStepKind::Advertise,
        VerifyStepKind::Objects,
    ];

    /// This step's discriminant.
    #[must_use]
    pub const fn kind(&self) -> VerifyStepKind {
        match self {
            Self::ResolveUrl => VerifyStepKind::ResolveUrl,
            Self::Advertise => VerifyStepKind::Advertise,
            Self::Objects { .. } => VerifyStepKind::Objects,
        }
    }
}

/// What the write path does with a child's stdout (§47.3's *Streams*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdoutUse {
    /// Kept and returned: the answer is on stdout.
    Parse,
    /// Read and discarded, so a chatty child cannot fill the pipe. It never reaches the core's
    /// own stdout, which carries protocol frames and nothing else.
    Drain,
}

/// What an intent is declared to write (§47.1): §47.9 C compares the repository against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredEffect {
    /// A new repository at a destination that did not exist.
    NewRepository,
    /// Nothing at all.
    Nothing,
    /// Objects into the repository's own store; no ref, no `FETCH_HEAD`, no config.
    ObjectsOnly,
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
    /// §47.4's verifying read — objects into the repository's own store, and no ref.
    VerifyRead,
}

/// §47.2's intent set, as far as Lane 0 lands it: every git write this product makes.
///
/// **`Fetch` is retired** (PA1). Its argv was `fetch --progress <remote>` and nothing else, and a
/// user's refspec, `fetch.prune` and `pruneTags` made it delete a checked-out branch holding a
/// local-only commit and a local-only tag (§37.8, M2). `VerifyRead` replaces it; each later
/// variant lands with its production caller, never before it.
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
    /// §47.4: one step of the verifying read of one configured remote.
    VerifyRead {
        /// The repository the read runs in, rendered as `-C` by `write_base_args` and never by
        /// `argv()`: core-resolved, a location re-resolved from disk or a nested repository.
        repo: PathBuf,
        /// The remote's configured name, never a URL and never a path.
        remote: RemoteName,
        /// Which of the three steps.
        step: VerifyStep,
    },
}

impl Intent {
    /// The exhaustive discriminant list. Its length is a deliberate tripwire on its own growth,
    /// and `core/tests/git_write_audit.rs` pins it.
    pub const ALL: [IntentKind; 2] = [IntentKind::Clone, IntentKind::VerifyRead];

    /// Render this intent's argv. **Total over (variant, step)** — no `_ =>` arm, and no
    /// caller-supplied string reaches argv unrendered.
    ///
    /// `--progress` is here because git writes progress to **stderr** only when it is not
    /// attached to a terminal and is asked to; §24.4's stage machine parses that stream.
    #[must_use]
    pub fn argv(&self) -> Vec<OsString> {
        match self {
            Self::Clone { url, dest, depth } => {
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
            // as the subcommand.
            Self::VerifyRead { remote, step, .. } => {
                let mut argv: Vec<OsString> = match step {
                    VerifyStep::ResolveUrl => vec!["ls-remote".into(), "--get-url".into()],
                    VerifyStep::Advertise => vec!["ls-remote".into()],
                    // §47.3's pins, each exactly once. `--refmap=` empties the configured
                    // refspecs so no tracking ref moves; `--stdin` takes only the tips; the four
                    // `--no-*` keep the read to objects.
                    VerifyStep::Objects { .. } => vec![
                        "fetch".into(),
                        "--refmap=".into(),
                        "--stdin".into(),
                        "--no-prune".into(),
                        "--no-tags".into(),
                        "--no-recurse-submodules".into(),
                        "--no-write-fetch-head".into(),
                        "--no-write-commit-graph".into(),
                    ],
                };
                argv.push(OsString::from(remote.as_str()));
                argv
            }
        }
    }

    /// The `-c` pins this intent's child carries beyond the uniform ones, each exactly once.
    ///
    /// `protocol.file.allow=never` on every verifying step (PA1): a second net under
    /// `GIT_ALLOW_PROTOCOL`, which layer A can read off the argv.
    #[must_use]
    pub const fn config_pins(&self) -> &'static [&'static str] {
        match self {
            Self::Clone { .. } => &[],
            Self::VerifyRead { .. } => &["protocol.file.allow=never"],
        }
    }

    /// §47.3's per-intent pins — every token that must appear **exactly once** in this intent's
    /// child argv: the argv flags and the `-c` values.
    ///
    /// A clone's pins are its filter neutralisation, which is the effective config's and is
    /// asserted where it is enumerated.
    #[must_use]
    pub fn pins(&self) -> Vec<&'static str> {
        let flags: &[&'static str] = match self {
            Self::Clone { .. }
            | Self::VerifyRead {
                step: VerifyStep::ResolveUrl | VerifyStep::Advertise,
                ..
            } => &[],
            Self::VerifyRead {
                step: VerifyStep::Objects { .. },
                ..
            } => &[
                "--refmap=",
                "--stdin",
                "--no-prune",
                "--no-tags",
                "--no-recurse-submodules",
                "--no-write-fetch-head",
                "--no-write-commit-graph",
            ],
        };
        flags.iter().chain(self.config_pins()).copied().collect()
    }

    /// The bytes this intent's child reads on stdin, or `None` for `Stdio::null()`.
    ///
    /// Only the objects step has any: its tips, one [`AdvertisedRef`] per line. Stdin is argv by
    /// another channel, so it is built from the validated newtype alone (§47.2 rule 2).
    #[must_use]
    pub fn stdin_payload(&self) -> Option<Vec<u8>> {
        match self {
            Self::VerifyRead {
                step: VerifyStep::Objects { tips },
                ..
            } => {
                let mut payload = Vec::new();
                for tip in tips {
                    payload.extend_from_slice(tip.as_str().as_bytes());
                    payload.push(b'\n');
                }
                Some(payload)
            }
            Self::Clone { .. }
            | Self::VerifyRead {
                step: VerifyStep::ResolveUrl | VerifyStep::Advertise,
                ..
            } => None,
        }
    }

    /// What the write path does with this intent's stdout.
    #[must_use]
    pub const fn stdout_use(&self) -> StdoutUse {
        match self {
            Self::VerifyRead {
                step: VerifyStep::ResolveUrl | VerifyStep::Advertise,
                ..
            } => StdoutUse::Parse,
            Self::Clone { .. }
            | Self::VerifyRead {
                step: VerifyStep::Objects { .. },
                ..
            } => StdoutUse::Drain,
        }
    }

    /// The environment this intent's child gets **after** `neutralise_env` (§47.3).
    ///
    /// `GIT_ALLOW_PROTOCOL` always — the load-bearing transport pin. The objects step also gets
    /// `GIT_NO_REPLACE_OBJECTS=1` and `GIT_GRAFT_FILE` at a path inside the empty hooks directory
    /// that never exists: a graft or replace ref otherwise rewrites the graph the fetch walks.
    #[must_use]
    pub fn env_pins(&self, hooks_dir: &Path) -> Vec<(&'static str, OsString)> {
        let mut pins = vec![(
            "GIT_ALLOW_PROTOCOL",
            OsString::from(self.allowed_protocols()),
        )];
        match self {
            Self::VerifyRead {
                step: VerifyStep::Objects { .. },
                ..
            } => {
                pins.push(("GIT_NO_REPLACE_OBJECTS", OsString::from("1")));
                pins.push((
                    "GIT_GRAFT_FILE",
                    crate::git::absent_graft_path(hooks_dir).into_os_string(),
                ));
            }
            Self::Clone { .. }
            | Self::VerifyRead {
                step: VerifyStep::ResolveUrl | VerifyStep::Advertise,
                ..
            } => {}
        }
        pins
    }

    /// The repository this intent runs in, when it has one.
    ///
    /// `None` for a clone: its destination **does not exist yet**, which is §24.1's precondition,
    /// and rendering `-C` for it would name a directory git is about to create.
    #[must_use]
    pub fn work_dir(&self) -> Option<&Path> {
        match self {
            Self::Clone { .. } => None,
            Self::VerifyRead { repo, .. } => Some(repo.as_path()),
        }
    }

    /// How long this intent's child may run before the core kills its process group (§47.3).
    ///
    /// `None` for a clone: it reports progress to a user who can cancel it, and a large clone
    /// legitimately runs for minutes. Every step of the verifying read, which runs while a user
    /// waits for a verdict, carries [`GIT_INVOCATION_DEADLINE`].
    #[must_use]
    pub const fn deadline(&self) -> Option<Duration> {
        match self {
            Self::Clone { .. } => None,
            Self::VerifyRead { .. } => Some(GIT_INVOCATION_DEADLINE),
        }
    }

    /// The transports this intent's child may use, as `GIT_ALLOW_PROTOCOL` spells them (§47.2).
    ///
    /// **The load-bearing transport pin.** Argv pins lost twice: a user's
    /// `protocol.<helper>.allow=always` ran a helper past `-c protocol.allow=never`, and a
    /// user-global `protocol.file.allow=always` opened a file remote past it; the environment
    /// variable refused both (§47 M4). A clone is https only (§24.1c): a config that rewrites its
    /// URL to a path or to ssh now fails instead of cloning over that transport. The verifying
    /// read admits the user's ssh transport as well (PA1).
    #[must_use]
    pub const fn allowed_protocols(&self) -> &'static str {
        match self {
            Self::Clone { .. } => "https",
            Self::VerifyRead { .. } => "https:ssh",
        }
    }

    /// What this intent is declared to write (§47.1).
    #[must_use]
    pub const fn effect(&self) -> DeclaredEffect {
        match self {
            Self::Clone { .. } => DeclaredEffect::NewRepository,
            Self::VerifyRead {
                step: VerifyStep::ResolveUrl | VerifyStep::Advertise,
                ..
            } => DeclaredEffect::Nothing,
            Self::VerifyRead {
                step: VerifyStep::Objects { .. },
                ..
            } => DeclaredEffect::ObjectsOnly,
        }
    }

    /// Is this intent part of an act §45.1 governs, and so refused below the governed git floor
    /// (§47.8)? A clone is not: it runs on every git the app runs on.
    #[must_use]
    pub const fn is_governed(&self) -> bool {
        match self {
            Self::Clone { .. } => false,
            Self::VerifyRead { .. } => true,
        }
    }

    /// The discriminant of this intent, for an audit that reports which variant it rendered.
    #[must_use]
    pub const fn kind(&self) -> IntentKind {
        match self {
            Self::Clone { .. } => IntentKind::Clone,
            Self::VerifyRead { .. } => IntentKind::VerifyRead,
        }
    }

    /// The step a verifying read carries, or `None` for an intent that has no steps.
    #[must_use]
    pub const fn step(&self) -> Option<VerifyStepKind> {
        match self {
            Self::Clone { .. } => None,
            Self::VerifyRead { step, .. } => Some(step.kind()),
        }
    }

    /// One fully-populated intent per [`IntentKind`] **and per [`VerifyStepKind`]**, over a
    /// fixture carrying the sentinel credential the audit scans for.
    ///
    /// Both `match`es below are **exhaustive**, so a variant or a step added without a case here
    /// fails to compile. That is strictly stronger than the count in [`Intent::ALL`], and it is
    /// why both are asserted: the count alone is a number somebody can raise, and this is a
    /// build failure nobody can miss.
    #[cfg(feature = "testkit")]
    #[must_use]
    pub fn all_for_audit(fixture: &AuditFixture) -> Vec<Self> {
        let mut out = Vec::new();
        for kind in Self::ALL {
            match kind {
                IntentKind::Clone => out.push(Self::Clone {
                    url: fixture.url().clone(),
                    dest: fixture.dest().to_path_buf(),
                    depth: Some(1),
                }),
                IntentKind::VerifyRead => {
                    for step_kind in VerifyStep::ALL {
                        let step = match step_kind {
                            VerifyStepKind::ResolveUrl => VerifyStep::ResolveUrl,
                            VerifyStepKind::Advertise => VerifyStep::Advertise,
                            VerifyStepKind::Objects => VerifyStep::Objects {
                                tips: fixture.tips().to_vec(),
                            },
                        };
                        out.push(Self::VerifyRead {
                            repo: fixture.dest().to_path_buf(),
                            remote: fixture.remote().clone(),
                            step,
                        });
                    }
                }
            }
        }
        out
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
    tips: Vec<AdvertisedRef>,
    token: SecretToken,
}

#[cfg(feature = "testkit")]
impl AuditFixture {
    /// Build the fixture under `root`, which must be a directory the caller owns.
    ///
    /// The destination is a path under `root` that **does not exist**, which is §24.1's
    /// precondition for `clone` and what assertion 5 checks.
    ///
    /// # Errors
    /// Never in practice: the fixture's URL and remote name are literals that pass
    /// [`RemoteUrl::parse`] and [`RemoteName::parse`]; their refusals are propagated regardless.
    pub fn new(root: &Path, token: SecretToken) -> Result<Self, IntentRefusal> {
        Ok(Self {
            url: RemoteUrl::parse("https://forge.example/acme/widget.git")?,
            dest: root.join("widget"),
            remote: RemoteName::parse("origin")?,
            tips: vec![
                AdvertisedRef::parse("HEAD")?,
                AdvertisedRef::parse("refs/heads/main")?,
            ],
            token,
        })
    }

    /// The sentinel credential every variant is rendered with.
    #[must_use]
    pub const fn token(&self) -> &SecretToken {
        &self.token
    }

    /// The fixture's remote URL.
    #[must_use]
    pub const fn url(&self) -> &RemoteUrl {
        &self.url
    }

    /// The fixture's clone destination, which does not exist.
    #[must_use]
    pub fn dest(&self) -> &Path {
        &self.dest
    }

    /// The fixture's remote name.
    #[must_use]
    pub const fn remote(&self) -> &RemoteName {
        &self.remote
    }

    /// The tips the fixture's objects step carries on stdin.
    #[must_use]
    pub fn tips(&self) -> &[AdvertisedRef] {
        &self.tips
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

    use super::{
        AdvertisedRef, DeclaredEffect, Intent, IntentRefusal, ObjectId, RemoteName, RemoteUrl,
        StdoutUse, VerifyStep,
    };

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

    fn verify(step: VerifyStep) -> Intent {
        Intent::VerifyRead {
            repo: PathBuf::from("/srv/work/thing"),
            remote: RemoteName::parse("origin").unwrap(),
            step,
        }
    }

    fn strings(intent: &Intent) -> Vec<String> {
        intent
            .argv()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// §47.4's three steps, each its own argv. The repository is **not** in any of them: `-C` is
    /// a base argument and the audit reads `argv[0]` as the subcommand.
    #[test]
    fn each_verify_step_renders_its_own_argv_and_no_repository() {
        assert_eq!(
            strings(&verify(VerifyStep::ResolveUrl)),
            vec!["ls-remote", "--get-url", "origin"]
        );
        assert_eq!(
            strings(&verify(VerifyStep::Advertise)),
            vec!["ls-remote", "origin"]
        );
        let objects = verify(VerifyStep::Objects {
            tips: vec![AdvertisedRef::parse("refs/heads/main").unwrap()],
        });
        assert_eq!(
            strings(&objects),
            vec![
                "fetch",
                "--refmap=",
                "--stdin",
                "--no-prune",
                "--no-tags",
                "--no-recurse-submodules",
                "--no-write-fetch-head",
                "--no-write-commit-graph",
                "origin"
            ]
        );
        assert_eq!(objects.stdin_payload(), Some(b"refs/heads/main\n".to_vec()));
        assert_eq!(objects.effect(), DeclaredEffect::ObjectsOnly);
        assert_eq!(objects.stdout_use(), StdoutUse::Drain);
        assert_eq!(verify(VerifyStep::Advertise).stdout_use(), StdoutUse::Parse);
        assert!(objects.is_governed());
    }

    /// [p2-24b] The verifying read names the repository it runs in, by type; a clone does not.
    #[test]
    fn a_verify_read_names_the_repository_it_runs_in_and_a_clone_does_not() {
        assert_eq!(
            verify(VerifyStep::Advertise).work_dir(),
            Some(Path::new("/srv/work/thing"))
        );
        let clone = Intent::Clone {
            url: RemoteUrl::parse("https://forge.example/acme/widget.git").unwrap(),
            dest: PathBuf::from("/srv/work/new"),
            depth: None,
        };
        // A clone's destination does not exist yet, so `-C` would name a directory git is about
        // to create.
        assert_eq!(clone.work_dir(), None);
        assert!(!clone.is_governed());
        assert_eq!(clone.effect(), DeclaredEffect::NewRepository);
    }

    /// §47.2's grammar: a destination on a stdin line wrote a ref under every pin (M5).
    #[test]
    fn an_advertised_ref_is_head_or_a_well_formed_refs_name() {
        for good in [
            "HEAD",
            "refs/heads/main",
            "refs/tags/v1.0",
            "refs/changes/12/34/1",
        ] {
            assert!(AdvertisedRef::parse(good).is_ok(), "{good}");
        }
        for bad in [
            "refs/heads/main:refs/heads/injected",
            "+refs/heads/main",
            "refs/heads/*",
            "refs/tags/v1^{}",
            "refs/heads/a..b",
            "refs/heads/x@{1}",
            "refs/heads/.hidden",
            "refs/heads/x.lock",
            "refs/heads/",
            "refs/heads/x.",
            "refs//heads",
            "refs/heads/sp ace",
            "main",
            "-refs/heads/main",
        ] {
            assert_eq!(
                AdvertisedRef::parse(bad),
                Err(IntentRefusal::UnsafeRefName),
                "{bad:?} must be refused"
            );
        }
        assert!(AdvertisedRef::parse("refs/tags/v1")
            .unwrap()
            .is_branch_or_tag_tip());
        assert!(!AdvertisedRef::parse("refs/changes/1/1/1")
            .unwrap()
            .is_branch_or_tag_tip());
    }

    #[test]
    fn an_object_id_is_forty_or_sixty_four_lowercase_hex() {
        assert!(ObjectId::parse(&"a".repeat(40)).is_ok());
        assert!(ObjectId::parse(&"b".repeat(64)).is_ok());
        assert_eq!(ObjectId::parse("HEAD"), Err(IntentRefusal::NotAnObjectId));
    }
}
