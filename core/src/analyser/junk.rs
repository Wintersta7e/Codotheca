//! §45.4's junk set: **one owner, one direction**.
//!
//! [`JUNK_PATTERNS`] moved unchanged from the phase-2 uniqueness analyser, and it is **closed**:
//! there is no user-declared junk, because *"this is junk"* is `Remove anyway` through a side door
//! (G24). The rule reading it is §45.4's.

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

/// §45.4: is `rel`, relative to the repository root `root`, junk?
///
/// **Both must hold.** A directory component **strictly above** the path's own final name equals
/// an entry exactly, case-exact — so a *file* named `build` is precious and `Node_modules/` is
/// precious, which is the safe direction for a near-miss. And **no directory between the root and
/// the path holds a `.git`**: a directory holding `.git` is another repository, never junk, and
/// row 9 analyses it.
///
/// `is_dir` says `rel` names a whole directory, as `status` reports a wholly untracked or ignored
/// one: then its own name stands above everything it holds, and counts.
#[must_use]
pub fn is_junk(root: &Path, rel: &Path, is_dir: bool) -> bool {
    let names: Vec<&std::ffi::OsStr> = rel
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    let above = if is_dir {
        names.as_slice()
    } else {
        names.split_last().map_or(&[][..], |(_, above)| above)
    };
    if !above.iter().any(|name| {
        JUNK_PATTERNS
            .iter()
            .any(|junk| *name == std::ffi::OsStr::new(junk))
    }) {
        return false;
    }
    let mut here = root.to_path_buf();
    for name in above {
        here.push(name);
        if std::fs::symlink_metadata(here.join(".git")).is_ok() {
            return false;
        }
    }
    true
}
