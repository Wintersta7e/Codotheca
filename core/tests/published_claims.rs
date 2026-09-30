#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![cfg(feature = "testkit")]
//! §48.3 rows 5–7 and 9: what `README.md` and `SECURITY.md` say about git, read against the code
//! that decides it.
//!
//! §47 owns the intent set and the governed floor; this file owns the sentences. Each claim is
//! derived from the core's side — `Intent::ALL` through an exhaustive `match`, both floor
//! constants, the argv and environment every invocation is built with — so a variant added or
//! retired fails to compile here until its published phrase is written, and a changed pin or
//! scrub fails until the documents move with it.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::git::{base_args, neutralise_env, RepoHandle, StoreKey, GIT_FLOOR};
use codotheca_core::gitw::{
    write_base_args, AuditFixture, CredentialChannel, FilterDrivers, Intent, IntentKind, WriteEnv,
    GOVERNED_GIT_FLOOR,
};
use codotheca_core::mount::StoreClass;

const MARK_START: &str = "<!-- git-writes:start -->";
const MARK_END: &str = "<!-- git-writes:end -->";

/// The claims the tree before phase 4 published and the code no longer supports. Compared with
/// whitespace collapsed and case folded.
const RETIRED: &[&str] = &[
    "its only writes are `clone` and `fetch`",
    "`fetch` never prunes",
    "every git invocation is read-only",
    "`push`, `checkout` and `worktree add` arrive",
];

/// Each intent's published phrase. **Exhaustive, with no `_` arm**: a variant added to
/// `IntentKind` does not compile here until the documents name it.
const fn phrase(kind: IntentKind) -> &'static str {
    match kind {
        IntentKind::Clone => "a clone into a new folder",
        IntentKind::VerifyRead => "adds objects and moves no ref",
    }
}

fn doc(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Whitespace collapsed and `**` emphasis dropped, so a phrase wrapped across lines still matches.
fn normalised(text: &str) -> String {
    text.replace("**", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The text between the markers, and the whole line the start marker sits on through the end
/// marker (where a stated count of ways may precede the marker).
fn marked(text: &str) -> Option<(String, String)> {
    let (before, rest) = text.split_once(MARK_START)?;
    let (block, _) = rest.split_once(MARK_END)?;
    if rest.contains(MARK_START) {
        return None;
    }
    let line_head = before.rsplit_once('\n').map_or(before, |(_, head)| head);
    Some((
        normalised(block),
        normalised(&format!("{line_head} {block}")),
    ))
}

/// `one` … `thirty-nine`, the only numbers these documents spell out.
fn number(word: &str) -> Option<usize> {
    const UNITS: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    let word = word.to_ascii_lowercase();
    if let Some(n) = UNITS.iter().position(|u| *u == word) {
        return Some(n);
    }
    let (tens, unit) = word.split_once('-').unwrap_or((word.as_str(), "zero"));
    let tens = match tens {
        "twenty" => 20,
        "thirty" => 30,
        _ => return None,
    };
    let unit = UNITS.iter().take(10).position(|u| *u == unit)?;
    Some(tens + unit)
}

/// The word immediately before `marker` in `text`, if `marker` occurs.
fn word_before<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let (before, _) = text.split_once(marker)?;
    before.split_whitespace().last()
}

#[test]
fn ac_p4_48_7_readme_and_security_name_exactly_the_git_write_intents() {
    let n = Intent::ALL.len();
    let mut named = Vec::new();
    for name in ["README.md", "SECURITY.md"] {
        let text = doc(name);
        let (block, line) = marked(&text)
            .unwrap_or_else(|| panic!("{name}: no single {MARK_START} … {MARK_END} block"));
        let mut found = 0;
        for kind in Intent::ALL {
            let wanted = phrase(kind);
            assert!(
                block.contains(wanted),
                "{name}: the git-writes block does not name {kind:?} (\"{wanted}\"): {block}"
            );
            found += 1;
        }
        named.push(found);
        if let Some(word) = word_before(&line, " ways") {
            assert_eq!(
                number(word),
                Some(n),
                "{name}: the block says \"{word} ways\" and Intent::ALL holds {n}"
            );
        }
        let whole = normalised(&text).to_lowercase();
        for retired in RETIRED {
            assert!(
                !whole.contains(retired),
                "{name}: still claims \"{retired}\""
            );
        }
    }
    eprintln!(
        "intents: {n}; README: {}/{n}; SECURITY: {}/{n}",
        named[0], named[1]
    );
    assert!(n > 0, "Intent::ALL is empty, so nothing was checked");
}

#[test]
fn the_readme_states_both_git_floors() {
    let readme = doc("README.md");
    let requirements = readme
        .split("\n\n")
        .map(normalised)
        .find(|p| p.starts_with("Requires "))
        .expect("the README's requirements paragraph");
    let app = format!("`git` {}.{} or newer", GIT_FLOOR.0, GIT_FLOOR.1);
    let (major, minor, patch) = GOVERNED_GIT_FLOOR;
    let governed = format!("git {major}.{minor}.{patch} or newer");
    eprintln!("floors: app {app:?}, governed {governed:?}");
    assert!(
        requirements.contains(&app),
        "the requirements do not state {app}: {requirements}"
    );
    assert!(
        requirements.contains(&governed),
        "the requirements do not state {governed}: {requirements}"
    );
}

fn strings(argv: &[OsString]) -> Vec<String> {
    argv.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

fn handle(dir: &Path, trusted: bool) -> RepoHandle {
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let mut repo = RepoHandle::resolve(dir, StoreKey::new("s0"), StoreClass::Local).unwrap();
    repo.trusted = trusted;
    repo
}

/// Whether `argv` carries `-c <pin>`, where a `<…>` value stands for any value.
fn carries(argv: &[String], pin: &str) -> bool {
    let (key, value) = pin.split_once('=').unwrap_or((pin, ""));
    argv.windows(2).any(|w| {
        w[0] == "-c"
            && if value.starts_with('<') {
                w[1].starts_with(&format!("{key}="))
            } else {
                w[1] == pin
            }
    })
}

#[test]
fn security_md_names_only_what_every_git_invocation_carries() {
    let security = normalised(&doc("SECURITY.md"));
    let (_, paragraph) = security
        .split_once("A scanned repository is nonetheless treated as untrusted data.")
        .expect("SECURITY.md's untrusted-data paragraph");
    let spans: Vec<&str> = paragraph.split('`').skip(1).step_by(2).collect();

    // Every invocation shape: the read path, trusted and not, and every write intent rendered as
    // production renders it — anonymous credential, filters enumerated (`gitw/backend.rs`).
    let tmp = tempfile::tempdir().unwrap();
    let hooks = tmp.path().join("hooks-empty");
    let mut shapes: Vec<(String, Vec<String>)> = vec![
        (
            "read".to_owned(),
            strings(&base_args(&handle(&tmp.path().join("a"), false), &hooks)),
        ),
        (
            "read, trusted".to_owned(),
            strings(&base_args(&handle(&tmp.path().join("b"), true), &hooks)),
        ),
    ];
    let fixture = AuditFixture::new(tmp.path(), SecretToken::new("unused".to_owned())).unwrap();
    let env = WriteEnv {
        work_dir: None,
        hooks_dir: hooks,
        credential: CredentialChannel::anonymous(),
        filters: FilterDrivers::enumerated(Vec::new()),
    };
    for intent in Intent::all_for_audit(&fixture) {
        shapes.push((
            format!("{:?} {:?}", intent.kind(), intent.step()),
            strings(&write_base_args(&intent, &env)),
        ));
    }

    let pins: Vec<&str> = spans.iter().filter_map(|s| s.strip_prefix("-c ")).collect();
    eprintln!("pins named: {pins:?}; invocation shapes: {}", shapes.len());
    assert!(!pins.is_empty(), "SECURITY.md names no -c pin");
    for pin in &pins {
        for (shape, argv) in &shapes {
            assert!(
                carries(argv, pin),
                "{shape} does not carry -c {pin}: {argv:?}"
            );
        }
    }

    // The scrub also removes whole families it finds in this process's environment. Clearing
    // them here leaves the fixed set, which is what a published count can state. This binary's
    // other tests read no environment variable.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            std::env::remove_var(&name);
        }
    }
    let mut cmd = Command::new("git");
    neutralise_env(&mut cmd);
    let envs: Vec<(String, Option<String>)> = cmd
        .get_envs()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.map(|v| v.to_string_lossy().into_owned()),
            )
        })
        .collect();
    let removed: Vec<&str> = envs
        .iter()
        .filter(|(k, v)| v.is_none() && k.starts_with("GIT_"))
        .map(|(k, _)| k.as_str())
        .collect();
    eprintln!("GIT_* removed: {} {removed:?}", removed.len());
    assert!(
        !removed.is_empty(),
        "neutralise_env removed no GIT_* variable"
    );

    for span in &spans {
        let Some((name, value)) = span.split_once('=') else {
            continue;
        };
        if !name.starts_with("GIT_") || name.contains(' ') {
            continue;
        }
        assert!(
            envs.iter()
                .any(|(k, v)| k == name && v.as_deref() == Some(value)),
            "SECURITY.md says {name}={value}, and neutralise_env does not set it"
        );
    }
    if let Some(word) = word_before(paragraph, " other `GIT_*` variables removed") {
        assert_eq!(
            number(word),
            Some(removed.len()),
            "SECURITY.md quotes \"{word}\" removed GIT_* variables; the scrub removes {}",
            removed.len()
        );
    }
}
