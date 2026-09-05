//! §22.2 — the provider-declared host-alias fold.
//!
//! §1.1 names *"host aliases"* among the combinations v2's rule mishandled and supplies no fix.
//! This is the fix. [`canonical_remote_key`](super::remote::canonical_remote_key) lowercases a
//! host and strips a numeric port and does nothing else, so a clone taken over an alias host and
//! a listing published on the canonical one are two keys for one repository.
//!
//! **The fold is a comparison form and writes nothing.** No stored `remote_key` is rewritten: the
//! column holds what `git config` will produce again on the next scan, and a rewritten one would
//! disagree with the repository on disk.
//!
//! **Provider-declared, never pattern-guessed** (§22.2). The set's contents belong to the
//! adapter — including the exclusion of gist hosts, because a gist is not a repository — and an
//! undeclared host folds to itself and so does not participate in matching. An SSH-config alias
//! is unresolvable without reading `~/.ssh/config`, which this app does not read; it reaches
//! §22.6's suppression rather than producing a duplicate.

use crate::provider::Provider;

/// One provider's canonical host and the finite set of spellings that fold to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostAliases {
    canonical: String,
    aliases: Vec<String>,
}

impl HostAliases {
    /// Read the set off the adapter that declares it.
    #[must_use]
    pub fn from_provider(provider: &dyn Provider) -> Self {
        Self::declared(provider.canonical_host(), provider.host_aliases())
    }

    /// The same set from an adapter's **declaration** rather than from an instance of it.
    ///
    /// A scan folds a clone's key with no account and no transport in hand; the alias set is
    /// static per adapter, so it reads the declaration. `core::provider::declared_host_aliases`
    /// is the only intended caller — this is not a door for inventing a set.
    #[must_use]
    pub fn declared(canonical: &str, aliases: &[&str]) -> Self {
        Self {
            canonical: canonical.to_owned(),
            aliases: aliases.iter().map(|host| (*host).to_owned()).collect(),
        }
    }

    /// Whether this host folds to the canonical one.
    ///
    /// The canonical host is always a member of its own set. An Enterprise install declares no
    /// aliases, and answering `false` for the only host it has would make `contains` mean
    /// something different for that provider than for every other.
    #[must_use]
    pub fn contains(&self, host: &str) -> bool {
        host.eq_ignore_ascii_case(&self.canonical)
            || self
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(host))
    }

    #[must_use]
    pub fn canonical(&self) -> &str {
        &self.canonical
    }

    /// Every spelling that folds to the canonical host, the canonical one included and listed
    /// once. A caller that has to narrow a stored column by index runs one equality per spelling
    /// rather than a pattern over the host segment.
    #[must_use]
    pub fn spellings(&self) -> Vec<&str> {
        let mut out = vec![self.canonical.as_str()];
        for alias in &self.aliases {
            if !out.iter().any(|seen| seen.eq_ignore_ascii_case(alias)) {
                out.push(alias.as_str());
            }
        }
        out
    }
}

/// The host segment's comparison form. A declared host folds to the canonical one; an undeclared
/// host folds to itself.
#[must_use]
pub fn fold_host(host: &str, aliases: &HostAliases) -> String {
    if aliases.contains(host) {
        return aliases.canonical().to_owned();
    }
    host.to_owned()
}

/// A whole `remote_key`'s comparison form: the host segment folded, the rest byte-identical.
///
/// It splits on the **first** `/` and rejoins. It never re-parses the path and never assembles a
/// key from parts — a second `<host>/<owner>/<name>` assembler is a defect whatever it computes
/// (§22.1), and the path may carry more than two segments.
///
/// `None` for anything that is not a key: a value with no `/` names no repository, and folding it
/// would produce a bare host that compares equal to every other bare host.
#[must_use]
pub fn fold_key(key: &str, aliases: &HostAliases) -> Option<String> {
    let (host, path) = key.split_once('/')?;
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{}/{path}", fold_host(host, aliases)))
}
