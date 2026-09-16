#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §25.4's Rust gate: **phase 2 displays a remote fact and may not judge it.**
//!
//! `concept.md` names CI among *health checks* — scored, feeding rank and the needs-attention
//! feed — while the design names *latest CI runs*, a record with a workflow, a conclusion, a
//! branch and a run number. Two objects sharing a word (R15's shape). Phase 2 has the record.
//!
//! In the shape of `core/tests/git_readonly.rs`, and with the same guard: **a gate whose passing
//! run scans zero files is a failing gate**, so the walk prints its count and fails at zero.

use std::path::{Path, PathBuf};

/// The three tables and the module. A file that names one of these is a file with the forge's
/// facts in its hands, which is exactly the set §25.4 is about.
const REMOTE_MARKERS: &[&str] = &[
    "remote_repo",
    "remote_topic",
    "remote_ci_run",
    "crate::remote",
    "codotheca_core::remote",
];

/// The two columns §1.10 gives to completion, which nothing in phase 2 writes. Phase 3's health
/// surface is what earns them.
const COMPLETION_COLUMNS: &[&str] = &["completion_lit", "completion_applicable"];

fn core_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_sources(root: &Path) -> Vec<(String, String)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            // A directory that vanished between the walk and the read carries nothing to check,
            // and it is skipped BEFORE it is counted.
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
fn no_remote_path_writes_a_completion_column() {
    let sources = rust_sources(&core_src());
    eprintln!(
        "remote_no_completion_writer: walked {} core source file(s)",
        sources.len()
    );
    assert!(
        !sources.is_empty(),
        "the walk read no file, so it proved nothing"
    );

    let remote_files: Vec<&(String, String)> = sources
        .iter()
        .filter(|(_, text)| REMOTE_MARKERS.iter().any(|marker| text.contains(marker)))
        .collect();
    eprintln!(
        "remote_no_completion_writer: {} of them name a remote fact",
        remote_files.len()
    );
    // The same guard one level down: a filter that matched nothing would pass the assertion
    // below while proving nothing about any remote path.
    assert!(
        !remote_files.is_empty(),
        "no file names a remote fact, so this gate scanned nothing"
    );

    let mut offenders = Vec::new();
    for (name, text) in &remote_files {
        for column in COMPLETION_COLUMNS {
            if text.contains(column) {
                offenders.push(format!("{name} names {column}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a remote path reaches a completion column, which is phase 3's: {offenders:?}"
    );
}

/// **There is no "CI green" boolean anywhere in phase 2** — the aggregate *is* the check, and
/// the check is phase 3. A `ConditionSignal` derived from a run set is the same thing wearing
/// §5.4's vocabulary.
#[test]
fn no_remote_path_produces_an_aggregate_over_a_run_set() {
    let sources = rust_sources(&core_src());
    let remote_files: Vec<&(String, String)> = sources
        .iter()
        .filter(|(_, text)| REMOTE_MARKERS.iter().any(|marker| text.contains(marker)))
        .collect();
    eprintln!(
        "remote_no_completion_writer: aggregate census read {} file(s)",
        remote_files.len()
    );
    assert!(
        !remote_files.is_empty(),
        "the aggregate census read no file, so it proved nothing"
    );

    let banned = [
        "ci_green",
        "ciGreen",
        "ci_ok",
        "ci_passing",
        "ciPassing",
        "ci_status",
        "ConditionSignal",
    ];
    let mut offenders = Vec::new();
    for (name, text) in &remote_files {
        for needle in banned {
            if text.contains(needle) {
                offenders.push(format!("{name} names {needle}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a remote path produces an aggregate or a condition, both phase 3's: {offenders:?}"
    );
}
