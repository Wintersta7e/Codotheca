#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §25.3: the backup state is **one exported producer**, and §24's pre-flight imports it.
//!
//! *"Two declarations of one computation is R1's recorded shape, four times over."* The thing
//! that makes a second declaration possible is that the inputs — `ahead`, `stash_count` and
//! `fetch_head_at` — are on a row several modules already read, so a first-match table over them
//! is two lines away anywhere in the core.
//!
//! What is asserted is the **construction**: a second producer would have to build a
//! `BackupState` variant, and exactly two files may name one — this plan's producer, and the
//! generated protocol module that declares the enum.

use std::path::{Path, PathBuf};

/// Constructing a verdict. A file that only *matches* on one — as `locations.uninstall` will —
/// names the type and not a variant path.
const CONSTRUCTIONS: &[&str] = &[
    "BackupState::OnlyCopy",
    "BackupState::NotAnywhereElse",
    "BackupState::Verified",
];

/// The producer, and nothing else.
///
/// The generated `protocol.rs` declares the enum but constructs no variant, so it is not on this
/// list — which is the tell that the list is about **construction** and not about naming.
const ALLOWED: &[&str] = &["remote/backup.rs"];

fn core_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_sources(root: &Path) -> Vec<(String, String)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("{}: {error}", dir.display()),
        };
        for entry in entries {
            let entry = entry.expect("a readable entry");
            let path = entry.path();
            if entry.file_type().expect("a file type").is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                match std::fs::read_to_string(&path) {
                    Ok(text) => out.push((
                        path.strip_prefix(root)
                            .unwrap_or(&path)
                            .display()
                            .to_string()
                            .replace('\\', "/"),
                        text,
                    )),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => panic!("{}: {error}", path.display()),
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn the_backup_verdict_is_constructed_in_exactly_one_place() {
    let sources = rust_sources(&core_src());
    eprintln!(
        "backup_single_declaration: scanned {} core source file(s)",
        sources.len()
    );
    assert!(
        !sources.is_empty(),
        "the walk read no file, so it proved nothing"
    );

    let mut constructing: Vec<&str> = sources
        .iter()
        .filter(|(_, text)| CONSTRUCTIONS.iter().any(|needle| text.contains(needle)))
        .map(|(name, _)| name.as_str())
        .collect();
    constructing.sort_unstable();
    eprintln!("backup_single_declaration: {constructing:?} construct a verdict");
    // A filter that matched nothing would satisfy the assertion below while proving nothing.
    assert!(
        !constructing.is_empty(),
        "no file constructs a backup verdict, so this gate scanned nothing"
    );
    assert_eq!(
        constructing, ALLOWED,
        "a second producer of §25.3's verdict exists; there is one, and §24's pre-flight imports it"
    );
}
