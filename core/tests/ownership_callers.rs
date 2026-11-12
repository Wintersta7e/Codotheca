#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Explicit selection skips git's ownership check, so the one thing that turns a WSL-spelled
//! pointer into an explicitly selected repository is `git::ownership`, whose probes run in the
//! resolution that then reads. These pin that: a new site cannot mark a handle owned silently.

use std::path::{Path, PathBuf};

fn core_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every `core/src` file holding `needle`, relative and `/`-separated, with its count.
fn holders(needle: &str) -> Vec<(String, usize)> {
    let root = core_src();
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(files.len() > 100, "scanned {} files", files.len());
    let mut found: Vec<(String, usize)> = files
        .iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(path).unwrap();
            let n = text.matches(needle).count();
            (n > 0).then(|| {
                let rel = path.strip_prefix(&root).unwrap();
                (rel.to_string_lossy().replace('\\', "/"), n)
            })
        })
        .collect();
    found.sort();
    found
}

#[test]
fn ownership_is_established_only_by_the_probing_function() {
    assert_eq!(
        holders("ownership_established("),
        vec![
            // The probing function's one call.
            ("git/ownership.rs".to_owned(), 1),
            // The definition, and two calls in its own unit test.
            ("git/repo.rs".to_owned(), 3),
        ]
    );
}

#[test]
fn nothing_outside_the_handle_spells_an_established_selection() {
    // The variant, `selects_explicitly`'s arm, `ownership_established`'s assignment, and the
    // unit test's two assertions.
    assert_eq!(
        holders("GitDirSelect::Established"),
        vec![("git/repo.rs".to_owned(), 4)]
    );
}
