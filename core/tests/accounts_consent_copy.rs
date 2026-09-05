//! **AC-P2-20-13's build-side half, and it is not `deferred`.**
//!
//! §20.3's four claims are *documentation knowledge* against a live API nobody here has called,
//! so the criterion itself waits (R56). What does **not** wait is the rule that makes waiting
//! safe: **no consent string asserting a scope may ship before the live check runs.**
//!
//! So this asserts that no rendered string in the settings surface names a scope, by reading the
//! renderer's own source and **printing its file count**. The scope chips the `CONNECTED` state
//! draws come from `accounts.list`'s `grantedScopes` — a read-back fact — and there is nothing
//! for a live observation to contradict.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::provider::scopes::{SCOPES_PRIVATE, SCOPES_PUBLIC};

fn settings_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../app/src/renderer/settings")
}

/// Every non-test source in the settings surface, with `//` comment lines removed.
///
/// Comments are stripped because this file's neighbours document the ban, and a gate that greps
/// the prose *about* a rule reports the documentation rather than the code. Test files are
/// skipped for the same reason: the bar that proves the chips are payload-driven necessarily
/// contains scope-shaped fixtures.
fn settings_sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let dir = settings_dir();
    for entry in std::fs::read_dir(&dir).expect("the settings surface is readable") {
        let entry = entry.expect("a readable entry");
        // The kind comes from readdir, never a second stat.
        if !entry.file_type().expect("an entry kind").is_file() {
            continue;
        }
        let path = entry.path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        // `Path::extension` rather than `ends_with`: clippy refuses a case-sensitive extension
        // comparison, and this asks the path what its extension is instead of matching text.
        let is_ts = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ts") || e.eq_ignore_ascii_case("tsx"));
        if !is_ts || name.to_ascii_lowercase().contains(".test.") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let code = text
                    .lines()
                    .filter(|line| {
                        let t = line.trim_start();
                        !t.starts_with("//") && !t.starts_with('*') && !t.starts_with("/*")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                out.push((name, code));
            }
            // Skipped BEFORE it is counted, so "scanned nothing" keeps meaning what it says.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("{}: {e}", path.display()),
        }
    }
    out
}

/// No rendered string in the settings surface names a scope.
#[test]
fn no_rendered_string_names_a_scope() {
    let files = settings_sources();
    eprintln!(
        "accounts_consent_copy: scanned {} settings source file(s)",
        files.len()
    );
    assert!(
        !files.is_empty(),
        "the walk read no file, so this proved nothing"
    );

    let mut offenders: Vec<String> = Vec::new();
    for (name, code) in &files {
        for scope in SCOPES_PUBLIC.iter().chain(SCOPES_PRIVATE) {
            // **The bare string `repo` is not bannable**, being an unavoidable English word —
            // it is a substring of `repoCount`, `repositories` and half this surface's prose.
            // §20.10 states that limitation as part of the rule, and it is *precisely why* the
            // displayed list must come from `grantedScopes` rather than from a grep's silence.
            // A scope with a `:` or a `_` is unambiguous; a bare word is not.
            if !scope.contains(':') && !scope.contains('_') {
                continue;
            }
            if code.contains(scope) {
                offenders.push(format!("{name}: {scope:?}"));
            }
        }
        // The two that were shipped on this very screen and are not scope names at all.
        for impossible in ["repo:read", "workflow:read", "delete_repo"] {
            if code.contains(impossible) {
                offenders.push(format!("{name}: {impossible:?}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a consent string names a scope, which may not ship before the live check runs: \
         {offenders:?}"
    );
}

/// The other half of the same rule, stated positively: the surface **does** render a grant, and
/// it takes it from the payload. Without this the test above passes on a screen that shows
/// nothing at all, which would satisfy the ban and defeat its purpose.
#[test]
fn the_connected_state_renders_the_grant_it_was_given() {
    let files = settings_sources();
    let drawn = files.iter().any(|(_, code)| code.contains("grantedScopes"));
    assert!(
        drawn,
        "no settings source reads grantedScopes, so the ban above is satisfied by an empty screen"
    );
}
