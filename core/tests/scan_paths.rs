#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §1.3's three path forms as free functions, plus the one comparison the scanner needs.
//!
//! The canonical-key rule itself belongs to `index::path` (plan 04) and is tested in
//! `index_paths.rs`; these tests pin the free-function surface plan 07 consumes and, above all,
//! that the two agree byte for byte.

use codotheca_core::index::path::{PathPlatform, StoredPath};
use codotheca_core::paths::{is_under, path_bytes, path_from_bytes, path_key};
use std::path::{Path, PathBuf};

/// R2: every key in these tests names its platform. The Unix cases would previously have folded
/// on a Windows host and the Windows case could not run on Linux at all.
fn ukey(p: &str) -> Vec<u8> {
    path_key(Path::new(p), PathPlatform::Unix)
}

#[test]
fn path_bytes_round_trips() {
    let p = PathBuf::from("/home/u/a b/c");
    assert_eq!(path_from_bytes(&path_bytes(&p)), p);
}

/// The encoding is stated once, in `index::path`, or `location.path_bytes` holds two encodings.
/// A `Discovered` built here and a `StoredPath` written by plan 08 describe the same row.
#[test]
fn the_free_function_and_stored_path_agree_on_the_byte_encoding() {
    for raw in ["/home/u/p", "a b/c", "/"] {
        let p = Path::new(raw);
        let stored = StoredPath::from_os(p.as_os_str(), PathPlatform::Unix);
        assert_eq!(path_bytes(p), stored.bytes(), "one encoding for path_bytes");
        assert_eq!(
            path_key(p, PathPlatform::Unix),
            stored.key(),
            "one key rule"
        );
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_bytes_survive_the_round_trip() {
    use codotheca_core::paths::path_display;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;
    let raw = b"/tmp/\xff\xfe/repo";
    let p = PathBuf::from(OsStr::from_bytes(raw));
    assert_eq!(path_bytes(&p), raw.to_vec());
    assert_eq!(path_from_bytes(raw), p);
    // path_display is lossy on purpose and is never used to open anything.
    assert!(path_display(&p).contains('\u{fffd}'));
}

/// Measured in the shipped app: a scan root stored with forward slashes, joined with a child,
/// rendered with both separators in one string — which reads as a corrupt value rather than a
/// location, and was reported as a defect against a path that was perfectly valid. `Path::join`
/// appends the host's separator whatever the root was spelled with, so the mix is made at display
/// time and has to be resolved there.
#[test]
fn a_drive_lettered_path_is_displayed_with_one_separator() {
    use codotheca_core::paths::path_display;
    assert_eq!(path_display(&PathBuf::from(r"D:/Work\0")), r"D:\Work\0");
    assert_eq!(
        path_display(&PathBuf::from("E:/Shelf/Widget/widget-source")),
        r"E:\Shelf\Widget\widget-source"
    );
    // Already consistent, and unchanged.
    assert_eq!(
        path_display(&PathBuf::from(r"D:\Work\alpha")),
        r"D:\Work\alpha"
    );
}

/// The rewrite is keyed on the drive letter and nothing else. A POSIX path keeps its forward
/// slashes, and §4bis.4's display-only `\\wsl.localhost\…` form already carries backslashes —
/// rewriting either by the host's convention is how a Linux path gets shown as a Windows one.
#[test]
fn a_path_with_no_drive_letter_is_left_exactly_as_it_is() {
    use codotheca_core::paths::path_display;
    assert_eq!(
        path_display(&PathBuf::from("/srv/work/alpha")),
        "/srv/work/alpha"
    );
    assert_eq!(
        path_display(&PathBuf::from(r"\\wsl.localhost\Ubuntu\home\x")),
        r"\\wsl.localhost\Ubuntu\home\x"
    );
    assert_eq!(
        path_display(&PathBuf::from("relative/child")),
        "relative/child"
    );
}

#[test]
fn path_key_strips_a_trailing_separator_but_never_the_root() {
    assert_eq!(ukey("/a/b/"), ukey("/a/b"));
    assert_eq!(ukey("/"), b"/".to_vec());
}

/// R2's whole point: both behaviours are testable on either host, because neither is selected by
/// `#[cfg]`. A Unix key is case-sensitive even when the test runs on Windows; a Windows key folds
/// case and `\` even when the test runs on Linux.
#[test]
fn the_platform_argument_decides_folding_not_the_host() {
    assert_ne!(ukey("/a/Repo"), ukey("/a/repo"));
    assert_eq!(
        path_key(Path::new(r"C:\A\Repo"), PathPlatform::Windows),
        path_key(Path::new("c:/a/repo"), PathPlatform::Windows)
    );
}

/// A backslash is a legal character in a Unix filename, so folding it to `/` there would merge
/// a directory called `a\b` with a directory `b` inside `a`.
#[test]
fn a_backslash_is_a_separator_on_windows_and_a_filename_character_on_unix() {
    assert_eq!(
        path_key(Path::new(r"c:\a\b"), PathPlatform::Windows),
        b"c:/a/b".to_vec()
    );
    assert_eq!(ukey(r"/a\b"), br"/a\b".to_vec());
    assert!(!is_under(&ukey(r"/a\b"), &ukey("/a")));
}

#[test]
fn is_under_matches_on_component_boundaries_only() {
    let parent = ukey("/a/b");
    assert!(is_under(&ukey("/a/b"), &parent));
    assert!(is_under(&ukey("/a/b/c"), &parent));
    assert!(!is_under(&ukey("/a/bc"), &parent));
    assert!(!is_under(&ukey("/a"), &parent));
}

#[test]
fn is_under_handles_a_root_parent() {
    assert!(is_under(&ukey("/a"), &ukey("/")));
    assert!(is_under(&ukey("/"), &ukey("/")));
}
