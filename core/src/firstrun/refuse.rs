//! §10.1a's refuse-list. Every refusal carries a reason; there is no silent no.

use std::path::Path;

use crate::protocol::RootRefusal;
use crate::scan::skiplist::SkipList;

/// Above this many estimated directories a root is refused until confirmed.
///
/// The reason given is **not** slowness: the walk measured six figures of directories a second,
/// so half a million of them is seconds. A root that large is nearly always a mis-pick.
pub const DIRECTORY_CEILING: i64 = 500_000;

/// True for `/`, a drive root, and a bare UNC share.
#[must_use]
pub fn is_filesystem_root(path: &Path) -> bool {
    let text = path.to_string_lossy().replace('\\', "/");
    let trimmed = text.trim_end_matches('/');
    if trimmed.is_empty() {
        return true;
    }
    // `C:` with nothing after it.
    if trimmed.len() == 2 && trimmed.ends_with(':') {
        return true;
    }
    // `//server/share` and nothing below it.
    if let Some(rest) = trimmed.strip_prefix("//") {
        return rest.split('/').filter(|s| !s.is_empty()).count() <= 2;
    }
    false
}

/// True when the path is the home directory itself, with or without a trailing separator.
#[must_use]
pub fn is_bare_home(path: &Path, home: &Path) -> bool {
    fn normalise(p: &Path) -> String {
        p.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    }
    !normalise(home).is_empty() && normalise(path) == normalise(home)
}

/// The refusal a path earns from its shape alone, or `None`.
#[must_use]
pub fn shape_refusal(path: &Path, home: &Path) -> Option<RootRefusal> {
    if is_filesystem_root(path) {
        return Some(RootRefusal::FilesystemRoot);
    }
    if is_bare_home(path, home) {
        return Some(RootRefusal::HomeWithoutNarrowing);
    }
    None
}

/// Directories under `root`, counting `root` itself, stopping the moment the ceiling is
/// reached.
///
/// This is a real directory read, which is why §10.1b permits it **only** on a path the user
/// just chose in the shell's dialog and never on a suggested row, where it would break the
/// standfirst's promise that nothing is read until the user says so.
///
/// An excluded directory is counted and not descended into, which is what
/// [`crate::scan::walk`] does with the same list: it increments its walked count for every
/// directory entry it sees and only then decides whether to enter. §4.3 is about the *contents*
/// — a single entry is never what pushes a root over half a million, and a tree of them is.
#[must_use]
pub fn estimate_directories(root: &Path, ceiling: i64, skip: &SkipList) -> i64 {
    if !root.is_dir() {
        return 0;
    }
    let mut count: i64 = 1;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if count >= ceiling {
            return ceiling;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let child = entry.path();
            count = count.saturating_add(1);
            if count >= ceiling {
                return ceiling;
            }
            if skip.skips(&child) {
                continue;
            }
            stack.push(child);
        }
    }
    count
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::protocol::RootRefusal;
    use crate::scan::skiplist::SkipList;
    use std::path::{Path, PathBuf};

    // §10.1a: roots.add refuses `/`, a drive root, a home directory without a narrowing
    // subdirectory, and anything over the ceiling without explicit confirmation — with an
    // explanation, never a silent no.
    #[test]
    fn the_three_absolute_refusals_are_decided_from_the_shape_alone() {
        let home = Path::new("/home/u");
        assert_eq!(
            shape_refusal(Path::new("/"), home),
            Some(RootRefusal::FilesystemRoot)
        );
        assert_eq!(
            shape_refusal(Path::new("C:\\"), home),
            Some(RootRefusal::FilesystemRoot)
        );
        assert_eq!(
            shape_refusal(Path::new("D:/"), home),
            Some(RootRefusal::FilesystemRoot)
        );
        assert_eq!(
            shape_refusal(Path::new("/home/u"), home),
            Some(RootRefusal::HomeWithoutNarrowing)
        );
        assert_eq!(shape_refusal(Path::new("/home/u/dev"), home), None);
        assert_eq!(shape_refusal(Path::new("/srv/work"), home), None);
    }

    #[test]
    fn a_trailing_separator_does_not_smuggle_a_home_past_the_refusal() {
        assert!(is_bare_home(Path::new("/home/u/"), Path::new("/home/u")));
        assert!(is_bare_home(
            Path::new("C:\\Users\\u\\"),
            Path::new("C:\\Users\\u")
        ));
        assert!(!is_bare_home(
            Path::new("/home/u/dev"),
            Path::new("/home/u")
        ));
    }

    #[test]
    fn the_estimate_stops_at_the_ceiling_rather_than_walking_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..7 {
            std::fs::create_dir_all(dir.path().join(format!("d{i}")).join("inner")).unwrap();
        }
        let skip = SkipList::default();
        assert_eq!(estimate_directories(dir.path(), 4, &skip), 4);
        assert_eq!(estimate_directories(dir.path(), 1_000, &skip), 15);
    }

    // §4.3 is the privacy policy, and it is the privacy policy for the estimate too: a
    // directory that will never be scanned must not push a root over the ceiling. The excluded
    // directory is itself one entry, exactly as the walk counts it; what it contains is not.
    #[test]
    fn the_estimate_honours_the_exclusion_list() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules").join("a").join("b")).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let skip = SkipList::default();
        assert_eq!(estimate_directories(dir.path(), 1_000, &skip), 3);
    }

    #[test]
    fn a_missing_directory_estimates_zero_and_never_panics() {
        let skip = SkipList::default();
        assert_eq!(
            estimate_directories(&PathBuf::from("/nonexistent/x"), 100, &skip),
            0
        );
    }
}
