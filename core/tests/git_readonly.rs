#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §17: phase 1 has no destructive operation at all, audited rather than promised.

use std::collections::BTreeSet;

/// Every non-flag argv literal this module is permitted to emit: eight read-only subcommands
/// plus the one revision name that is spelled out. §17 forbids the rest for the whole of
/// phase 1, and this list may not grow without a spec change.
const ALLOWED: &[&str] = &[
    "--version",
    "cat-file",
    "check-ignore",
    "log",
    "ls-files",
    "rev-list",
    "rev-parse",
    "show",
    "status",
    "HEAD",
];

/// Anything that writes. Present as an explicit denylist so the failure message names the crime.
const FORBIDDEN: &[&str] = &[
    "checkout",
    "clean",
    "commit",
    "fetch",
    "gc",
    "merge",
    "prune",
    "pull",
    "push",
    "rebase",
    "reset",
    "restore",
    "rm",
    "stash",
    "switch",
    "update-ref",
    "worktree",
    "write-tree",
];

fn source_files() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("git");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, std::fs::read_to_string(&path).unwrap()));
        }
    }
    assert!(
        out.len() >= 10,
        "expected the whole git module, found {}",
        out.len()
    );
    out
}

/// Collect every `OsStr::new("…")` literal in the module: an argv element can enter no other way.
fn os_str_literals(source: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = source;
    while let Some(at) = rest.find("OsStr::new(\"") {
        let after = &rest[at + "OsStr::new(\"".len()..];
        if let Some(end) = after.find('"') {
            found.insert(after[..end].to_owned());
            rest = &after[end..];
        } else {
            break;
        }
    }
    found
}

#[test]
fn no_source_file_names_a_destructive_subcommand() {
    for (name, source) in source_files() {
        for literal in os_str_literals(&source) {
            assert!(
                !FORBIDDEN.contains(&literal.as_str()),
                "{name} names the destructive subcommand `{literal}`; phase 1 has no destructive \
                 operation at all (§17)"
            );
        }
    }
}

#[test]
fn every_subcommand_literal_is_on_the_read_only_allow_list() {
    // A subcommand is the first literal that is not a flag, so anything not starting with `-`
    // and not a format string must be an allowed subcommand.
    for (name, source) in source_files() {
        for literal in os_str_literals(&source) {
            if literal.starts_with('-') && literal != "--version" {
                continue; // a flag, not a subcommand
            }
            assert!(
                ALLOWED.contains(&literal.as_str()),
                "{name} runs `{literal}`, which is not on the read-only allow list"
            );
        }
    }
}

// The audit has to see something. A run that scanned no literals would pass both assertions
// above while proving nothing — the failure shape this project keeps hitting.
#[test]
fn the_audit_actually_reads_argv_literals() {
    let total: usize = source_files()
        .iter()
        .map(|(_, source)| os_str_literals(source).len())
        .sum();
    assert!(
        total >= 20,
        "the audit found only {total} argv literals; a gate that scans nothing is a failing gate"
    );
}

// The token `FORGET` appears in no rendered string, accessible name or command anywhere in the
// product; the git layer is the place it would most plausibly leak in as a subcommand name.
#[test]
fn the_forbidden_token_appears_nowhere_in_the_module() {
    for (name, source) in source_files() {
        assert!(
            !source.contains("FORGET"),
            "{name} contains the forbidden token"
        );
    }
}

/// The one argv the git seam runs that is **built outside** `core/src/git/`.
///
/// `identity::remote::remote_urls_argv` holds the tokens, so the literal scan above cannot see
/// them — and until this plan nothing ran it, so §17's audit had never had to. Asserting the
/// value it returns is the same guarantee reached the other way round.
#[test]
fn the_remote_url_argv_is_a_read() {
    let argv = codotheca_core::identity::remote::remote_urls_argv();
    assert_eq!(
        argv.first().copied(),
        Some("config"),
        "the subcommand moved: {argv:?}"
    );
    for token in &argv {
        assert!(
            !FORBIDDEN.contains(token),
            "the remote read names the destructive subcommand `{token}`"
        );
    }
    // `git config` writes when it is given a value or a mutating flag. Neither is here, and the
    // token count is what makes that mechanical: `--get-regexp <pattern>` plus `--null` is four,
    // and a fifth token would be the value to store.
    for writing in [
        "--add",
        "--edit",
        "--remove-section",
        "--rename-section",
        "--replace-all",
        "--unset",
        "--unset-all",
    ] {
        assert!(
            !argv.contains(&writing),
            "the remote read carries the mutating flag `{writing}`"
        );
    }
    assert_eq!(argv.len(), 4, "an extra token would be a value: {argv:?}");
}
