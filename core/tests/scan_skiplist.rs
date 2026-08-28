#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §4.3's exclusion list. It is rendered to the user verbatim as the privacy policy (§10.1b), so
//! the strings and their order are asserted, not just the matching behaviour.

use codotheca_core::scan::skiplist::{SkipList, SKIP_LIST};
use std::path::Path;

#[test]
fn the_list_is_the_twenty_nine_entries_of_the_spec_in_order() {
    assert_eq!(SKIP_LIST.len(), 29);
    assert_eq!(SKIP_LIST.first(), Some(&"node_modules"));
    assert_eq!(SKIP_LIST.get(5), Some(&".cargo/registry"));
    assert_eq!(SKIP_LIST.get(21), Some(&"$RECYCLE.BIN"));
    assert_eq!(SKIP_LIST.last(), Some(&"AppData"));
    // Not alphabetised: caches, then build outputs, then system paths.
    let mut sorted = SKIP_LIST.to_vec();
    sorted.sort_unstable();
    assert_ne!(sorted.as_slice(), SKIP_LIST.as_slice());
}

#[test]
fn a_bare_name_matches_the_final_component_at_any_depth() {
    let s = SkipList::default();
    assert!(s.skips(Path::new("/home/u/p/node_modules")));
    assert!(s.skips(Path::new("/home/u/p/a/b/node_modules")));
    assert!(!s.skips(Path::new("/home/u/p/node_modules/x")));
    assert!(!s.skips(Path::new("/home/u/node_modules_old")));
}

#[test]
fn a_relative_entry_matches_only_as_a_whole_path_suffix() {
    let s = SkipList::default();
    assert!(s.skips(Path::new("/home/u/.cargo/registry")));
    assert!(!s.skips(Path::new("/home/u/registry")));
    assert!(!s.skips(Path::new("/home/u/.cargo/registry2")));
    assert!(s.skips(Path::new("/home/u/go/pkg/mod")));
}

#[cfg(unix)]
#[test]
fn an_absolute_entry_matches_itself_and_its_subtree() {
    let s = SkipList::default();
    assert!(s.skips(Path::new("/proc")));
    assert!(s.skips(Path::new("/proc/1/fd")));
    assert!(s.skips(Path::new("/nix/store/abc")));
    assert!(!s.skips(Path::new("/home/u/proc")));
}

#[test]
fn user_entries_are_appended_after_the_twenty_nine_and_match_the_same_way() {
    let s = SkipList::with_user_entries(&["Archive".to_owned()]);
    assert_eq!(s.entries().len(), 30);
    assert_eq!(
        s.entries().first().map(String::as_str),
        Some("node_modules")
    );
    assert_eq!(s.entries().last().map(String::as_str), Some("Archive"));
    assert!(s.skips(Path::new("/home/u/Archive")));
}

/// §10.1b renders `entries()` directly, so it must be the source strings and not a normalised
/// copy — a privacy policy showing `$recycle.bin` is a different claim from the one it matches.
#[test]
fn the_rendered_entries_are_verbatim_and_never_the_normalised_form() {
    let s = SkipList::default();
    assert_eq!(s.entries().len(), SKIP_LIST.len());
    for (rendered, source) in s.entries().iter().zip(SKIP_LIST.iter()) {
        assert_eq!(rendered, source);
    }
}

/// §4.3 is applied to descendants; the caller is what excludes a root. Asserted here because the
/// rule lives in the walk and a reader of this type would otherwise assume the opposite.
#[test]
fn a_trailing_separator_does_not_change_the_verdict() {
    let s = SkipList::default();
    assert!(s.skips(Path::new("/home/u/p/node_modules/")));
    assert!(s.skips(Path::new("/home/u/.cargo/registry/")));
}

/// §4.6 asks a different question from the walk's. The walk stops at `…/node_modules`; presence
/// classifies a `location` row that names something *inside* it, and `skips` on that row answers
/// "is `dep` excluded" — no — so the location would read `missing` rather than `unscanned`.
#[test]
fn covers_tests_every_ancestor_below_the_root_and_skips_tests_only_the_directory() {
    let s = SkipList::default();
    let root = Path::new("/r");
    assert!(!s.skips(Path::new("/r/node_modules/pkg/dep")));
    assert!(s.covers(Path::new("/r/node_modules/pkg/dep"), root));
    assert!(s.covers(Path::new("/r/node_modules"), root));
    assert!(!s.covers(Path::new("/r/p"), root));
}

/// §4.3: the list is never applied to a scan root itself, so a root named `build` stops the walk
/// up rather than excluding everything beneath it.
#[test]
fn covers_never_excludes_the_root_itself() {
    let s = SkipList::default();
    let build_root = Path::new("/home/u/build");
    assert!(s.skips(build_root), "the name is on the list");
    assert!(!s.covers(build_root, build_root));
    assert!(!s.covers(Path::new("/home/u/build/y"), build_root));
}
