#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! R53's gate: the README asset fetch is **not** scheduled work and spends **no** account's rate
//! allowance.
//!
//! The reason is not convenience. The hosts are arbitrary badge and image services, not the
//! forge, so mirroring their headers into a budget keyed `(account_id, resource)` would key a
//! forge pool by a stranger's `x-ratelimit-resource` — a wrong value in the row that decides
//! whether the app burns an account's allowance. A later author adding this module to the sync
//! runner is exactly what this file exists to stop, and it fails on the **name** because that is
//! what such a change would introduce first.

use std::path::{Path, PathBuf};

/// The four names that would mean this module had joined the sync runner.
const FORBIDDEN: [&str; 4] = ["SyncTask", "sync_budget", "JobKind", "project_job_state"];

/// Everything but the whole-line comments.
///
/// **Grepping a declaration matches prose about it** — this project has produced two wrong
/// rulings that way, and this gate produced a third on its first run: `fetch.rs`'s header states
/// R53 in terms, naming `SyncTask` and `sync_budget` to say it uses neither. Dropping lines whose
/// first non-space characters are `//` keeps the rule ("no *code* here names the sync runner")
/// while letting the module carry the reason it does not. A trailing comment is **not** stripped,
/// because `//` also appears inside every URL literal in this module.
fn code_only(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_readme_module_names_nothing_from_the_sync_runner() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/readme");
    let mut files = Vec::new();
    rust_files(&root, &mut files);

    let mut scanned = 0_usize;
    let mut hits: Vec<String> = Vec::new();
    for file in &files {
        // A file that vanished between the walk and the read is skipped **before** it is
        // counted, or the zero-scan guard below stops meaning what it says.
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        scanned += 1;
        let code = code_only(&text);
        for name in FORBIDDEN {
            if code.contains(name) {
                hits.push(format!("{}: {name}", file.display()));
            }
        }
    }

    eprintln!(
        "readme_no_sync_budget: scanned {scanned} file(s) for {} names",
        FORBIDDEN.len()
    );
    assert!(
        scanned > 0,
        "a gate whose passing run scans zero files is a failing gate"
    );
    assert!(
        hits.is_empty(),
        "R53: the README asset fetch is not scheduled work and draws on no rate budget: {hits:?}"
    );
}
