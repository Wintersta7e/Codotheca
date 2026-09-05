//! §20.10 audit 1 — the scope audit. **AC-P2-20-2 says "no source file in *the core*"**, so it
//! walks every `.rs` under `core/src/`, not one module.
//!
//! Scoping the walk to `core/src/provider/` would leave the one function that *takes* scope
//! literals outside the gate: the device-flow request takes `scopes: &[&str]` and the upgrade
//! passes `SCOPES_PRIVATE` from the command layer. A gate that misses the caller is the shape
//! this project keeps paying for.
//!
//! Modelled on `core/tests/git_readonly.rs`: an allowlist that may not grow without a spec
//! change, plus **a denylist so the failure names the crime**. It prints the number of files
//! scanned and fails at zero.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::provider::scopes::{SCOPES_PRIVATE, SCOPES_PUBLIC};
use std::collections::BTreeSet;

fn core_src() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` under a directory, with its text and comment lines blanked.
///
/// Comments are stripped because this file's own neighbours document the ban, and a gate that
/// greps the prose *about* a rule reports the documentation rather than the code.
fn sources(root: &std::path::Path) -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let entry = entry.expect("a readable entry");
            // The kind comes from readdir, never a second stat: a file that vanishes between
            // the two would take the gate down instead of being skipped.
            let kind = entry.file_type().expect("an entry kind");
            let path = entry.path();
            if kind.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        let code = text
                            .lines()
                            .filter(|line| !line.trim_start().starts_with("//"))
                            .collect::<Vec<_>>()
                            .join("\n");
                        let name = path
                            .strip_prefix(root)
                            .unwrap_or(&path)
                            .display()
                            .to_string();
                        out.push((name, code));
                    }
                    // Skipped BEFORE it is counted, so "scanned nothing" keeps meaning what it
                    // says while another gate's probe file races this walk.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => panic!("{}: {e}", path.display()),
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

/// `sources`, plus the count and the zero guard both audits open with.
///
/// The guard lives here and nowhere else so that `the_audit_fails_when_it_scans_nothing` can run
/// **this** predicate against an empty directory. A zero-guard test that restates the predicate in
/// its own words tests its own words: delete the guard and it still reads green.
fn scanned(root: &std::path::Path, label: &str) -> Vec<(String, String)> {
    let files = sources(root);
    eprintln!(
        "provider_scope_audit: {label} scanned {} file(s)",
        files.len()
    );
    assert!(
        !files.is_empty(),
        "{label} read no file, so the audit proved nothing"
    );
    files
}

/// A double-quoted string literal is the only way a scope reaches the wire from this core.
fn string_literals(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '"' {
            let mut j = i + 1;
            let mut value = String::new();
            while j < bytes.len() && bytes[j] != '"' {
                if bytes[j] == '\\' {
                    j += 1;
                }
                if j < bytes.len() {
                    value.push(bytes[j]);
                }
                j += 1;
            }
            out.push(value);
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// The one file allowed to write a scope literal at all.
const SCOPE_LITERAL_HOME: &str = "provider/scopes.rs";

/// §20.3's two tier sets are the only scope literals the core may send, and
/// `core/src/provider/scopes.rs` is the only file allowed to write one.
///
/// This is stated as *where a literal may be written* rather than as *which literals look like
/// scopes*. A shape heuristic over the whole core is not decidable: the query AST alone writes
/// `era:live`, `lang:rust` and `session:42`, none of which is a scope, and an audit that reports
/// them earns an exemption list that grows until it reports nothing. Every other file must
/// reference `SCOPES_PUBLIC` or `SCOPES_PRIVATE`, which is also what stops a second copy drifting.
#[test]
fn every_scope_literal_is_on_a_tier_list() {
    let files = scanned(&core_src(), "the allowlist over core/src/");

    let allowed: BTreeSet<&str> = SCOPES_PUBLIC
        .iter()
        .chain(SCOPES_PRIVATE)
        .copied()
        .collect();
    assert!(
        !allowed.is_empty(),
        "the tier constants are empty, so the audit would allow nothing and prove nothing"
    );

    let Some((_, home)) = files
        .iter()
        .find(|(name, _)| name.replace('\\', "/") == SCOPE_LITERAL_HOME)
    else {
        panic!("{SCOPE_LITERAL_HOME} is where the tier constants live")
    };
    let declared: BTreeSet<String> = string_literals(home).into_iter().collect();
    for scope in &allowed {
        assert!(
            declared.contains(*scope),
            "{scope:?} is in a tier constant but is not written in {SCOPE_LITERAL_HOME}"
        );
    }

    // Every other file must reach a scope through the constants. A second copy of `read:org`
    // somewhere else is R12's drift shape on a value the user consents to.
    let mut offenders: Vec<String> = Vec::new();
    for (name, code) in &files {
        if name.replace('\\', "/") == SCOPE_LITERAL_HOME {
            continue;
        }
        for literal in string_literals(code) {
            if allowed.contains(literal.as_str()) {
                offenders.push(format!("{name}: {literal:?}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a scope literal written outside {SCOPE_LITERAL_HOME}: {offenders:?}"
    );
}

/// The denylist, so a failure names the crime rather than only the count.
///
/// It walks the **whole** core, which is the widening AC-P2-20-2 asks for: the function that
/// *takes* scope literals lives in the accounts module, not the provider one, so a gate scoped
/// to `core/src/provider/` would let a stray `"gist"` in a caller through unnoticed.
#[test]
fn the_core_never_names_a_destructive_or_unrequested_scope() {
    let files = scanned(&core_src(), "the denylist over core/src/");

    let mut offenders: Vec<String> = Vec::new();
    for (name, code) in &files {
        for literal in string_literals(code) {
            let banned = ["delete_repo", "gist", "notifications"].contains(&literal.as_str())
                || literal == "workflow"
                || literal.starts_with("workflow:")
                || literal.starts_with("admin:")
                || literal.starts_with("write:");
            if banned {
                offenders.push(format!("{name}: {literal:?}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the core names a scope it must never request: {offenders:?}"
    );
}

/// A gate whose passing run scans nothing is a failing gate. Pointing the walk at a directory
/// with no `.rs` in it must fail rather than read green.
///
/// It calls `scanned` — the function both audits above open with — and asserts it panics. Nothing
/// short of that proves the guard is load-bearing: assert the fixture is empty and the guard can
/// be deleted with every test still green.
#[test]
fn the_audit_fails_when_it_scans_nothing() {
    let empty = tempfile::tempdir().expect("tmp");
    let outcome = std::panic::catch_unwind(|| scanned(empty.path(), "the zero-guard fixture"));
    let files = outcome.expect_err("scanning a directory with no Rust in it must fail the audit");
    let message = files
        .downcast_ref::<String>()
        .map_or("<not a string>", String::as_str);
    assert!(
        message.contains("proved nothing"),
        "the zero guard failed for some other reason: {message:?}"
    );
}

/// `PRIVATE_TIER_SCOPE` is what separates the two tiers, so it must be a member of the private
/// set and absent from the public one. Without both halves it is a third opinion about the
/// tiers rather than a name for the difference between them.
#[test]
fn the_tier_separator_is_the_scope_the_private_set_adds() {
    use codotheca_core::provider::scopes::PRIVATE_TIER_SCOPE;
    assert!(
        SCOPES_PRIVATE.contains(&PRIVATE_TIER_SCOPE),
        "the separator is not in the private tier"
    );
    assert!(
        !SCOPES_PUBLIC.contains(&PRIVATE_TIER_SCOPE),
        "the separator is in the public tier, so it separates nothing"
    );
}
