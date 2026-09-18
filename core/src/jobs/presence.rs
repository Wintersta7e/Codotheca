//! §29.4's four presence predicates, over the **unfiltered** HEAD enumeration.
//!
//! They are **path** predicates and read no file contents, so they cost nothing beyond the
//! `ls-tree` call — and they are evaluated **before §29.2's rule 3**. That ordering is
//! load-bearing: a `LICENSE` has no extension and `.github/workflows/ci.yml` carries a
//! `markup(...)` one, so filtering to programming extensions first would make all four
//! permanently `absent` while every other criterion still passed.
//!
//! **Each answer is a tri-state, never a bare boolean.** A budget exceedance is `not_read`, never
//! `absent`: a timeout looks exactly like a missing file. The **row's** absence is a fourth and
//! different fact — J7 has never run — and is not a value here.

use crate::git::TreeEntry;

use super::j6_content::README_NAMES;

/// `project_content_scan`'s four tri-states. **The spellings are §29.4's and this enum produces
/// them**; the DDL CHECK mirrors [`PresenceState::slug`] character for character (R26).
///
/// A plain Rust enum is not R31 here: the schema declares no presence type, and §31 maps these
/// three spellings onto its own wire vocabulary rather than putting them on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceState {
    /// The enumeration holds a matching path.
    Present,
    /// The enumeration was read and holds none. **A known false.**
    Absent,
    /// The enumeration failed, was cancelled, timed out, or the repository was unreadable,
    /// offline or untrusted. **Unknown, and never rendered as a false.**
    NotRead,
}

impl PresenceState {
    /// Every state, so a test can walk the vocabulary without restating it.
    pub const ALL: [PresenceState; 3] = [
        PresenceState::Present,
        PresenceState::Absent,
        PresenceState::NotRead,
    ];

    /// The stored form.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            PresenceState::Present => "present",
            PresenceState::Absent => "absent",
            PresenceState::NotRead => "not_read",
        }
    }

    /// `slug`'s inverse. `None` for a value a newer build wrote.
    #[must_use]
    pub fn from_slug(s: &str) -> Option<PresenceState> {
        match s {
            "present" => Some(PresenceState::Present),
            "absent" => Some(PresenceState::Absent),
            "not_read" => Some(PresenceState::NotRead),
            _ => None,
        }
    }
}

/// One enumeration pass's four answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresenceAnswers {
    /// A `README_NAMES` entry at the tree root.
    pub readme: PresenceState,
    /// A licence file at the tree root, or a `LICENSES/` directory.
    pub license: PresenceState,
    /// A test directory or a test-named file, at any depth.
    pub tests: PresenceState,
    /// A CI configuration, at any depth. **Not `ciGreen`** — that is the forge's latest
    /// conclusion (§25) and says nothing about whether a configuration exists, nor the reverse.
    pub ci: PresenceState,
}

impl PresenceAnswers {
    /// The answer set for an enumeration that did not happen. **All four `not_read`, never
    /// `absent`** — §29.4's single most likely place to render unknown as zero.
    #[must_use]
    pub fn not_read() -> Self {
        Self {
            readme: PresenceState::NotRead,
            license: PresenceState::NotRead,
            tests: PresenceState::NotRead,
            ci: PresenceState::NotRead,
        }
    }
}

/// What bumping this invalidates: **the enumeration**, never the blob cache.
///
/// `predicate_version` is not `scanner_version` and the two are bumped independently. Widening
/// the `tests` list must not invalidate a library-wide blob cache, and widening the marker set
/// must not force a re-enumeration.
pub const PREDICATE_VERSION: i64 = 1;

/// §29.4's licence basenames, **root-only**. Each may carry a `-<suffix>` (`LICENSE-MIT`) and an
/// extension from [`LICENSE_EXTENSIONS`].
///
/// Root-only because a committed `vendor/<dep>/LICENSE` would otherwise close `missing_license`
/// for a project that declares none, and vendored trees are tracked in plenty of repositories.
const LICENSE_STEMS: &[&str] = &["license", "licence", "copying", "copyright", "unlicense"];

/// The extensions a licence basename may carry. §29.4 owns the list.
const LICENSE_EXTENSIONS: &[&str] = &["md", "txt", "rst"];

/// §29.4's test **path components**, matched at any depth — a root-only rule is wrong for the
/// majority of real layouts.
const TEST_COMPONENTS: &[&str] = &["test", "tests", "spec", "specs", "__tests__", "testing"];

/// §29.4's test **basename** shapes, as `(prefix, suffix)` pairs matched against the whole
/// case-folded basename. An empty prefix is a plain suffix match.
const TEST_BASENAME_SHAPES: &[(&str, &str)] = &[
    ("", "_test.go"),
    ("", "_test.rs"),
    ("test_", ".py"),
    ("", "_test.py"),
    ("", ".test.ts"),
    ("", ".test.tsx"),
    ("", ".test.js"),
    ("", ".test.jsx"),
    ("", ".test.mjs"),
    ("", ".test.cjs"),
    ("", ".spec.ts"),
    ("", ".spec.tsx"),
    ("", ".spec.js"),
    ("", ".spec.jsx"),
    ("", "test.java"),
    ("", "tests.cs"),
    ("", "_spec.rb"),
];

/// §29.4's CI directories, matched as a **path component** at any depth.
const CI_DIRECTORIES: &[&str] = &[".circleci", ".buildkite", ".woodpecker"];

/// The two components `.github/workflows/` is, matched in sequence at any depth.
const CI_WORKFLOW_DIRS: [&str; 2] = [".github", "workflows"];

/// The extensions a `.github/workflows/` entry must carry to be a workflow.
const CI_WORKFLOW_EXTENSIONS: &[&str] = &["yml", "yaml"];

/// §29.4's CI basenames, matched at any depth.
const CI_BASENAMES: &[&str] = &[
    ".gitlab-ci.yml",
    "jenkinsfile",
    "azure-pipelines.yml",
    "azure-pipelines.yaml",
    ".travis.yml",
    ".drone.yml",
    "bitbucket-pipelines.yml",
];

/// Answer all four over the **unfiltered** records of one `ls-tree` pass.
#[must_use]
pub fn presence_for(entries: &[TreeEntry]) -> PresenceAnswers {
    let mut answers = PresenceAnswers {
        readme: PresenceState::Absent,
        license: PresenceState::Absent,
        tests: PresenceState::Absent,
        ci: PresenceState::Absent,
    };
    for entry in entries {
        let path = folded(&entry.path);
        let root = !path.contains('/');
        let base = path.rsplit('/').next().unwrap_or(&path).to_owned();
        if root && README_NAMES.iter().any(|n| n.to_ascii_lowercase() == base) {
            answers.readme = PresenceState::Present;
        }
        if is_license(&path, root, &base) {
            answers.license = PresenceState::Present;
        }
        if is_test(&path, &base) {
            answers.tests = PresenceState::Present;
        }
        if is_ci(&path, &base) {
            answers.ci = PresenceState::Present;
        }
    }
    answers
}

/// ASCII-fold a raw path. Non-ASCII bytes survive unchanged — a path is arbitrary bytes and the
/// lists are all ASCII, so nothing is lost and nothing is mangled.
fn folded(path: &[u8]) -> String {
    String::from_utf8_lossy(&path.to_ascii_lowercase()).into_owned()
}

fn is_license(path: &str, root: bool, base: &str) -> bool {
    if path.starts_with("licenses/") {
        return true;
    }
    if !root {
        return false;
    }
    // `LICENSE-MIT.md` is a stem, a suffix and an extension; each half is optional.
    let stem = base
        .rsplit_once('.')
        .filter(|(_, ext)| LICENSE_EXTENSIONS.contains(ext))
        .map_or(base, |(head, _)| head);
    let stem = stem.split_once('-').map_or(stem, |(head, _)| head);
    LICENSE_STEMS.contains(&stem)
}

fn is_test(path: &str, base: &str) -> bool {
    if path
        .split('/')
        .any(|component| TEST_COMPONENTS.contains(&component))
    {
        return true;
    }
    TEST_BASENAME_SHAPES.iter().any(|(prefix, suffix)| {
        base.len() > prefix.len() + suffix.len()
            && base.starts_with(prefix)
            && base.ends_with(suffix)
    })
}

fn is_ci(path: &str, base: &str) -> bool {
    // Every directory the path passes through — the basename is a file, not a directory it is
    // under, so it is excluded from both directory rules.
    let dirs: Vec<&str> = path.split('/').collect();
    let Some((_, dirs)) = dirs.split_last() else {
        return false;
    };
    let extension = base.rsplit_once('.').map(|(_, ext)| ext);
    if dirs.windows(2).any(|pair| pair == CI_WORKFLOW_DIRS)
        && extension.is_some_and(|ext| CI_WORKFLOW_EXTENSIONS.contains(&ext))
    {
        return true;
    }
    if dirs.iter().any(|dir| CI_DIRECTORIES.contains(dir)) {
        return true;
    }
    CI_BASENAMES.contains(&base)
}
