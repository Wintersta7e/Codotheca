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

/// Collect every literal `OsStr::new("…")` argument; computed arguments are audited separately.
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

#[derive(Debug)]
struct NonLiteralOsStrAllowance {
    file: &'static str,
    call: &'static str,
    reason: &'static str,
}

const NON_LITERAL_OS_STR_ALLOWLIST: &[NonLiteralOsStrAllowance] = &[
    NonLiteralOsStrAllowance {
        file: "refstate.rs",
        call: "OsStr::new(&range)",
        reason: "the value is an upstream-versus-HEAD revision range built from resolved OIDs",
    },
    NonLiteralOsStrAllowance {
        file: "history.rs",
        call: "OsStr::new(o.as_str())",
        reason: "each value is a commit OID already selected by the bounded root-history read",
    },
    NonLiteralOsStrAllowance {
        file: "history.rs",
        call: "OsStr::new(&count)",
        reason: "the value is the computed -n limit flag, not a git subcommand",
    },
    NonLiteralOsStrAllowance {
        file: "status.rs",
        call: "OsStr::new(untracked)",
        reason: "the closed status-mode enum produces one of two read-only untracked-file flags",
    },
    NonLiteralOsStrAllowance {
        file: "history.rs",
        call: "OsStr::new(&name)",
        reason: "[p2-24b] the value is a ref name this repository's own refs/ directory and \
                 packed-refs yielded, never a caller's string, and it sits after `rev-list` as \
                 a revision rather than as a subcommand",
    },
];

/// Collect `OsStr::new(...)` calls whose argument is not exactly one string literal.
fn non_literal_os_str_calls(source: &str) -> Vec<String> {
    const START: &str = "OsStr::new(";

    let mut calls = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = source[cursor..].find(START) {
        let call_start = cursor + relative_start;
        let argument_start = call_start + START.len();
        let mut depth = 1_u32;
        let mut in_string = false;
        let mut escaped = false;
        let mut call_end = None;

        for (relative, ch) in source[argument_start..].char_indices() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }
            match ch {
                '"' => in_string = true,
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        call_end = Some(argument_start + relative);
                        break;
                    }
                }
                _ => {}
            }
        }

        let Some(call_end) = call_end else {
            break;
        };
        let argument = source[argument_start..call_end].trim();
        if !(argument.starts_with('"') && argument.ends_with('"')) {
            calls.push(source[call_start..=call_end].to_owned());
        }
        cursor = call_end + 1;
    }
    calls
}

fn rejected_non_literal_os_str_calls(file: &str, source: &str) -> Vec<String> {
    non_literal_os_str_calls(source)
        .into_iter()
        .filter(|call| {
            !NON_LITERAL_OS_STR_ALLOWLIST.iter().any(|allowed| {
                assert!(
                    !allowed.reason.trim().is_empty(),
                    "{file}: empty allowlist reason"
                );
                allowed.file == file && allowed.call == call
            })
        })
        .collect()
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

#[test]
fn every_non_literal_os_str_call_is_explicitly_justified() {
    let files = source_files();
    let mut scanned = 0;
    let mut rejected = Vec::new();
    for (name, source) in &files {
        scanned += non_literal_os_str_calls(source).len();
        rejected.extend(
            rejected_non_literal_os_str_calls(name, source)
                .into_iter()
                .map(|call| format!("{name}: {call}")),
        );
    }
    eprintln!(
        "git read-only dynamic argv audit scanned {} files and {scanned} non-literal calls",
        files.len()
    );
    assert!(
        !files.is_empty(),
        "the dynamic argv audit scanned zero source files"
    );
    assert!(
        rejected.is_empty(),
        "non-literal OsStr::new arguments require an exact justified allowlist entry:\n{}",
        rejected.join("\n")
    );
}

#[test]
fn a_non_literal_os_str_fixture_is_rejected() {
    let source = r#"let sub = "push"; OsStr::new(sub)"#;
    assert_eq!(
        rejected_non_literal_os_str_calls("fixture.rs", source),
        ["OsStr::new(sub)"],
        "a computed argv element must not pass the git read-only audit"
    );
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

/// Every file in the two **mutating** modules, each with its own floor and its own count.
///
/// **A second collector, deliberately not a widening of `source_files`.** AC-P2-24-1 freezes
/// `ALLOWED`, `FORBIDDEN` and `source_files` — widening that glob to `core/src/git*` would have
/// changed what the read-only audit audits, which is the one thing that criterion exists to
/// prevent. `core/src/git/` stays provably read-only by being scanned alone.
fn mutating_module_files() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    for (module, floor) in [("gitw", 4_usize), ("removal", 3), ("uninstall", 2)] {
        let dir = root.join(module);
        let mut found = 0;
        for entry in std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{module} must exist by now: {e}"))
            .flatten()
        {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "rs") {
                let name = format!("{module}/{}", path.file_name().unwrap().to_string_lossy());
                out.push((name, std::fs::read_to_string(&path).unwrap()));
                found += 1;
            }
        }
        // A collector reading an empty directory is a gate asserting over nothing. The floor is
        // per directory so one module cannot cover for the other being absent.
        assert!(
            found >= floor,
            "core/src/{module}/ yielded {found} file(s), below its floor of {floor}"
        );
        eprintln!("git_readonly: core/src/{module}/ — {found} file(s)");
    }
    out
}

// The token `FORGET` appears in no rendered string, accessible name or command anywhere in the
// product; the git layer is the place it would most plausibly leak in as a subcommand name.
//
// **[p2] Read across all three modules.** `core/src/removal/` is the more likely of the two new
// ones: `FORGET` was the name of a *removal* affordance, not a git one, so the module that
// removes things is exactly where it would next appear (AC-P2-24-5).
/// The predicate the scan applies, named so the discrimination test below can apply the **same**
/// one rather than a restatement of it — a fixture checked against a second copy of the rule
/// proves the copy, not the rule.
fn carries_forbidden_token(source: &str) -> bool {
    source.contains("FORGET")
}

#[test]
fn the_forbidden_token_appears_nowhere_in_the_module() {
    let mut scanned = 0;
    for (name, source) in source_files().into_iter().chain(mutating_module_files()) {
        assert!(
            !carries_forbidden_token(&source),
            "{name} contains the forbidden token"
        );
        scanned += 1;
    }
    assert!(
        scanned >= 19,
        "the token scan read {scanned} file(s); a gate that scans nothing is a failing gate"
    );
    eprintln!("git_readonly: the forbidden token scan read {scanned} file(s)");
}

/// The widened scan is **discriminating**: a planted token in either new module is caught.
///
/// Fixture-shaped, in `a_non_literal_os_str_fixture_is_rejected`'s style, so no probe file is
/// left behind for another gate to trip over mid-life.
#[test]
fn a_planted_token_in_either_mutating_module_would_be_caught() {
    for module in ["gitw", "removal", "uninstall"] {
        let planted = format!("// a {module} file\npub const LABEL: &str = \"FORGET THIS\";\n");
        assert!(
            carries_forbidden_token(&planted),
            "the scan must reject a {module} file carrying the token"
        );
        let clean = format!("// a {module} file\npub const LABEL: &str = \"RELOCATE\";\n");
        assert!(
            !carries_forbidden_token(&clean),
            "and must accept one that does not — a scan that rejects everything proves nothing"
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
