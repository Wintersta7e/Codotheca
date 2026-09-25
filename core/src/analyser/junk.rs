//! §45.4's junk set: **one owner, one direction**.
//!
//! Moved unchanged from the phase-2 uniqueness analyser; §45.4's rule — a directory component
//! strictly above the path, case-exact, no `.git` between — replaces [`is_junk`]'s with the
//! worktree step that reads it.

use std::path::Path;

/// Paths whose presence says nothing worth keeping.
///
/// **One owner for the junk set**, and the rule is stated in exactly one direction: **an ignored
/// path is precious unless it matches something here, never the reverse.** Inverting it would make
/// every unrecognised ignored file junk, which is the direction that loses work — and a `.env`, a
/// local database or an editor scratch file is exactly what nothing here matches.
pub const JUNK_PATTERNS: [&str; 12] = [
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    "__pycache__",
    ".venv",
    "venv",
    "vendor",
    ".gradle",
    ".terraform",
    "Pods",
];

/// Is this path junk — and therefore not worth blocking a removal over?
#[must_use]
pub fn is_junk(rel: &Path) -> bool {
    rel.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        JUNK_PATTERNS.iter().any(|junk| name == *junk)
    })
}
