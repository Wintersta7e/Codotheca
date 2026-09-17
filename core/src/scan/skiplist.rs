//! §4.3's exclusion list. It is **shown verbatim to the user as the privacy policy** (§10.1b),
//! so the strings and their order are the spec's and are not to be tidied, sorted or corrected.
//! §10.1b also notes that the design prototype misspells one of them: the rendered string is
//! `$RECYCLE.BIN`, because a privacy policy that misspells what it matches is false.
//!
//! Two rules that are **not** entries and must not be added to the list, because the list is
//! rendered: a directory named `.git` is never descended into (that is the walk's rule, and it
//! is what stops every repository being found twice), and the list is never applied to a scan
//! root itself — a user who adds a root named `build` gets their root scanned.

use std::path::Path;

use crate::paths::path_display;

/// §4.3, in §4.3's order: caches, then build outputs, then system paths, and last this app's own
/// staging directory.
///
/// `.codotheca-installing` is appended rather than filed among the caches so that every existing
/// index is unmoved — `EXCLUSION_CHIPS_BEFORE_EXPANDER` still draws the same first eleven — and
/// because a reader of the rendered policy meets the third-party caches first and this app's own
/// scratch last, which is the order they will recognise it in.
pub const SKIP_LIST: [&str; 30] = [
    "node_modules",
    ".venv",
    "venv",
    "target",
    "vendor",
    ".cargo/registry",
    "go/pkg/mod",
    ".terraform/modules",
    ".local/share/nvim/lazy",
    ".vim/bundle",
    ".oh-my-zsh",
    "Pods",
    ".m2",
    ".gradle",
    ".pyenv",
    ".rustup",
    ".nvm",
    "dist",
    "build",
    ".next",
    "__pycache__",
    "$RECYCLE.BIN",
    "System Volume Information",
    "/nix/store",
    "/var/lib/docker",
    "/proc",
    "/sys",
    "/snap",
    "AppData",
    // §24.3b: a partial clone lives here, so the next scan cannot discover a half-written
    // repository — by the value the scanner already reads, rather than a second matcher.
    // `install::staging::STAGING_DIR_NAME` is where this string is owned.
    ".codotheca-installing",
];

/// The three shapes the list has. The spec states none of them, so this type states them:
///
/// | Shape | Example | Rule |
/// |---|---|---|
/// | bare component name | `node_modules` | the directory's **final component** equals it |
/// | relative path suffix | `.cargo/registry` | the key **ends with `/` + the entry** |
/// | absolute path | `/proc` | that path or anything beneath it, **on Unix only** |
#[derive(Debug, Clone, PartialEq, Eq)]
enum Rule {
    Name(String),
    Suffix(String),
    Absolute(String),
}

/// The 29 entries plus whatever the user added in settings' `EXCLUDED FROM EVERY SCAN` panel.
///
/// Rules are compiled once at construction rather than re-normalised per directory: `skips` is
/// called for every directory the walk visits, and the probe measured 101k of those a second.
#[derive(Debug, Clone)]
pub struct SkipList {
    entries: Vec<String>,
    rules: Vec<Rule>,
}

impl Default for SkipList {
    fn default() -> Self {
        Self::from_entries(SKIP_LIST.iter().map(|s| (*s).to_owned()).collect())
    }
}

impl SkipList {
    /// User entries are appended, never interleaved: §10.1b renders the 29 first.
    #[must_use]
    pub fn with_user_entries(extra: &[String]) -> Self {
        let mut entries: Vec<String> = SKIP_LIST.iter().map(|s| (*s).to_owned()).collect();
        entries.extend(extra.iter().cloned());
        Self::from_entries(entries)
    }

    fn from_entries(entries: Vec<String>) -> Self {
        let rules = entries
            .iter()
            .map(|entry| {
                let normalised = normalise(entry);
                if let Some(absolute) = normalised.strip_prefix('/') {
                    Rule::Absolute(format!("/{absolute}"))
                } else if normalised.contains('/') {
                    Rule::Suffix(format!("/{normalised}"))
                } else {
                    Rule::Name(normalised)
                }
            })
            .collect();
        Self { entries, rules }
    }

    /// Render order, verbatim, for the consent screen and the settings panel. Never the
    /// normalised form — the policy shown and the policy applied must read the same.
    #[must_use]
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// True when `dir` **or any ancestor of it below `root`** is excluded.
    ///
    /// This is the question §4.6 asks and [`skips`](Self::skips) is not it. The walk tests each
    /// directory as it descends, so stopping at `…/Archive` is enough to keep `…/Archive/old`
    /// out; but presence classifies a stored `location` row directly, and that row names the
    /// repository *inside* the excluded directory. Asking `skips` there answers "is `old`
    /// excluded", which is no, and the location would be reported `missing` — a claim that a
    /// repository is gone when it was merely never looked at.
    ///
    /// `root` is never excluded by its own list (§4.3): a user who adds a root named `build`
    /// gets their root scanned.
    #[must_use]
    pub fn covers(&self, dir: &Path, root: &Path) -> bool {
        let mut current = Some(dir);
        while let Some(path) = current {
            if path == root {
                return false;
            }
            if self.skips(path) {
                return true;
            }
            current = path.parent();
        }
        false
    }

    /// True when `dir` itself must not be read. Callers must not apply this to a scan root
    /// itself, and must use [`covers`](Self::covers) when the question is about a descendant.
    #[must_use]
    pub fn skips(&self, dir: &Path) -> bool {
        let hay = normalise(&path_display(dir));
        let name_start = hay.rfind('/').map_or(0, |i| i + 1);
        let name = hay.get(name_start..).unwrap_or_default();
        self.rules.iter().any(|rule| match rule {
            Rule::Name(entry) => name == entry,
            Rule::Suffix(entry) => hay.ends_with(entry.as_str()),
            Rule::Absolute(entry) => {
                cfg!(unix) && (hay == *entry || hay.starts_with(&format!("{entry}/")))
            }
        })
    }
}

/// Separators unified; case folded on Windows only.
///
/// R2 note: this is a *host* rule and stays `cfg!(windows)`, unlike `paths::path_key`. The
/// exclusion list is matched against directory names the walk is reading on this machine right
/// now, so there is no other platform in play — and unlike a `path_key`, nothing here is stored
/// or compared against a row written elsewhere. Do not "align" it by threading a `PathPlatform`
/// through: R2 changed `path_key` because keys outlive the host, and these strings do not.
fn normalise(raw: &str) -> String {
    let unified = raw.replace('\\', "/");
    let mut s = if cfg!(windows) {
        unified.to_lowercase()
    } else {
        unified
    };
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::SKIP_LIST;
    use crate::install::staging::STAGING_DIR_NAME;

    /// The list carries a **literal** rather than the const, because `app/test/skipList.test.ts`
    /// mirrors this array by parsing its string literals out of this file — an identifier would
    /// be invisible to it and the rendered privacy policy would silently lose an entry.
    ///
    /// So the value has one owner (`STAGING_DIR_NAME`) and this asserts the literal still equals
    /// it. Without this, the skip list and the staging directory could drift to two different
    /// names and a partial clone would become discoverable by the very next scan.
    #[test]
    fn the_staging_directory_is_skipped_under_the_name_it_is_actually_created_with() {
        assert!(
            SKIP_LIST.contains(&STAGING_DIR_NAME),
            "SKIP_LIST must carry {STAGING_DIR_NAME:?} verbatim"
        );
    }
}
