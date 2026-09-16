#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![cfg(feature = "testkit")]
//! §24.2a: the git **write** audit, asserted over `Intent::ALL` rather than over source text.
//!
//! This file is the sibling of `core/tests/git_readonly.rs` and makes the assertion that one
//! structurally cannot: **the flag denylist**. The read audit skips every literal beginning with
//! `-` except `--version`, and *the difference between an additive `fetch` and a destructive one
//! is a flag*.
//!
//! Every assertion here loops over a rendered set, so an empty set would pass all of them while
//! looking at nothing. [`rendered`] therefore checks the floor **once, before any caller sees
//! the set**, rather than leaving each test to remember.

use std::path::{Path, PathBuf};

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::cancel::CancelToken;
use codotheca_core::gitw::{
    write_base_args, AuditFixture, CredentialChannel, FilterDrivers, Intent, MutatingGit,
    SystemMutatingGit, WriteEnv,
};

/// §24.2a assertion 1's allow list.
const WRITE_ALLOWED: &[&str] = &["clone", "fetch"];

/// §24.2a assertion 2's deny list — the eight tokens that turn an additive invocation into a
/// destructive one.
const FLAG_FORBIDDEN: &[&str] = &[
    "--prune", "--force", "-f", "--hard", "--delete", "-d", "-D", "--mirror",
];

/// The sentinel the audit scans for. It is not a real credential and never reaches a forge.
const SENTINEL: &str = "credential-sentinel-do-not-leak";

/// Every variant, rendered over a fixture carrying the sentinel credential.
///
/// The fixture's temporary root is dropped before this returns, and that is deliberate: §24.1
/// requires a clone destination that **does not exist** at call time, so the audit renders
/// against a path guaranteed to be absent.
///
/// **The floor is checked here rather than in each test.** A rendered set shorter than
/// `Intent::ALL` would let every loop below pass by not looking — §26.2's *a gate whose passing
/// run scans zero files is a failing gate*, arriving inside the gate written to prevent it.
fn rendered() -> Vec<Intent> {
    let temp = tempfile::tempdir().expect("audit tempdir");
    let fixture =
        AuditFixture::new(temp.path(), SecretToken::new(SENTINEL.to_owned())).expect("fixture");
    let intents = Intent::all_for_audit(&fixture);
    assert_eq!(
        intents.len(),
        Intent::ALL.len(),
        "all_for_audit rendered {} of {} variants; every assertion in this file loops over this \
         set, so a short one passes by not looking",
        intents.len(),
        Intent::ALL.len()
    );
    assert!(
        !intents.is_empty(),
        "the write audit rendered zero variants"
    );
    intents
}

#[test]
fn every_rendered_subcommand_is_write_allowed() {
    let intents = rendered();
    let mut tokens = 0;
    for intent in &intents {
        let argv = intent.argv();
        tokens += argv.len();
        let subcommand = argv.first().and_then(|arg| arg.to_str()).unwrap_or("");
        assert!(
            WRITE_ALLOWED.contains(&subcommand),
            "{:?} rendered forbidden subcommand {subcommand:?}: {argv:?}",
            intent.kind()
        );
    }
    eprintln!(
        "git-write-audit: {} variants, {tokens} argv tokens, {} allowed subcommands",
        intents.len(),
        WRITE_ALLOWED.len()
    );
}

#[test]
fn no_rendered_argv_contains_a_forbidden_flag() {
    let intents = rendered();
    let mut scanned = 0;
    for intent in &intents {
        let argv = intent.argv();
        for token in &argv {
            scanned += 1;
            let token = token.to_string_lossy();
            assert!(
                !FLAG_FORBIDDEN.contains(&token.as_ref()),
                "{:?} rendered forbidden flag {token:?}: {argv:?}",
                intent.kind()
            );
        }
    }
    assert!(
        scanned > 0,
        "the flag denylist scanned zero argv tokens, so it asserts nothing"
    );
    eprintln!(
        "git-write-audit: {scanned} argv tokens checked against {} forbidden flags",
        FLAG_FORBIDDEN.len()
    );
}

#[test]
fn the_exhaustive_intent_list_has_two_renderable_variants() {
    let intents = rendered();
    assert_eq!(
        Intent::ALL.len(),
        2,
        "Intent::ALL changed; review every write-boundary assertion before accepting a new variant"
    );
    assert_eq!(
        intents.len(),
        Intent::ALL.len(),
        "all_for_audit did not render every Intent::ALL discriminant"
    );
}

/// **AC-P2-24-4, in the form that is expressible here.**
///
/// `Intent::Fetch` has no product caller in this plan — the in-session fetch is p2-24b's §24.7C —
/// so the criterion's *"under every scheduler retry path"* cannot be exercised by driving a
/// scheduler that does not exist. What **is** provable, and is the property the criterion rests
/// on, is that `argv()` is a **pure function of the variant**: rendering the same intent N times
/// yields byte-identical argv, so no retry can produce a token the first attempt did not.
///
/// Stated plainly because claiming otherwise would be a bar written past its defect: **this is
/// the property, not an exercised scheduler.**
#[test]
fn rendering_an_intent_repeatedly_is_byte_identical_and_never_grows_a_flag() {
    for intent in &rendered() {
        let first = intent.argv();
        for attempt in 1..8 {
            let again = intent.argv();
            assert_eq!(
                again,
                first,
                "{:?} rendered differently on attempt {attempt}; argv must be a pure function of \
                 the variant, or a retry can carry what the first attempt did not",
                intent.kind()
            );
            for token in &again {
                let token = token.to_string_lossy();
                assert!(
                    !FLAG_FORBIDDEN.contains(&token.as_ref()),
                    "{:?} grew forbidden flag {token:?} on attempt {attempt}",
                    intent.kind()
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// §24.2a assertions 4 and 5, read from what the CHILD received.
//
// R88: an assertion over the argv a builder returns is an assertion about an *intention*.
// Between `Intent::argv()` and the child sit `write_base_args`, `WriteExec` and the `Command`
// builder — exactly where `--prune`, a re-inherited `credential.helper` or a restored
// `GIT_ASKPASS` would enter unseen. So these drive the **production** `SystemMutatingGit` at a
// recording stand-in for git and assert over the file it writes.
// ---------------------------------------------------------------------------

/// One recorded child invocation: the argv it received and the environment it ran in.
struct Recorded {
    argv: Vec<String>,
    env: Vec<String>,
}

impl Recorded {
    fn read(dest: &Path) -> Recorded {
        let mut path = dest.to_path_buf();
        let mut name = path
            .file_name()
            .expect("destination has a file name")
            .to_os_string();
        name.push(".recorded");
        path.set_file_name(name);
        let blob = std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "no recording at {}: the write path never spawned the child ({e})",
                path.display()
            )
        });
        let mut argv = Vec::new();
        let mut env = Vec::new();
        for record in blob.split(|b| *b == 0) {
            let text = String::from_utf8_lossy(record);
            if let Some(rest) = text.strip_prefix("ARGV\t") {
                argv.push(rest.to_owned());
            } else if let Some(rest) = text.strip_prefix("ENV\t") {
                env.push(rest.to_owned());
            }
        }
        assert!(
            !argv.is_empty(),
            "the recording holds no argv, so every assertion over it would be vacuous"
        );
        Recorded { argv, env }
    }

    fn env_value(&self, key: &str) -> Option<&str> {
        self.env
            .iter()
            .find_map(|entry| entry.strip_prefix(&format!("{key}=")))
    }

    fn has_env(&self, key: &str) -> bool {
        self.env
            .iter()
            .any(|entry| entry.starts_with(&format!("{key}=")))
    }
}

fn recording_git() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_codotheca-recording-git"))
}

/// Drive the real `SystemMutatingGit` at the recording stand-in and return what the child got.
fn drive_a_clone(root: &Path) -> (Recorded, PathBuf) {
    let hooks = root.join("hooks-empty");
    std::fs::create_dir_all(&hooks).expect("hooks dir");
    let dest = root.join("widget");
    let intent = Intent::Clone {
        url: codotheca_core::gitw::RemoteUrl::parse("https://forge.example/acme/widget.git")
            .expect("fixture url"),
        dest: dest.clone(),
        depth: Some(1),
    };
    assert!(
        !dest.exists(),
        "§24.1: a clone's destination must not exist before the call"
    );
    let backend = SystemMutatingGit::new(recording_git(), hooks);
    let cancel = CancelToken::new();
    let mut seen = Vec::new();
    backend
        .run(&intent, &cancel, &mut |line| seen.push(line.to_owned()))
        .expect("the recording stand-in exits 0");
    (Recorded::read(&dest), dest)
}

/// **Assertion 4**, over the child's real argv and environment.
///
/// The credential option is **counted**, not searched for: a test that only rejects a wrong value
/// passes on *absence*, and absence is inheritance — git would fall back to the user's configured
/// helper, which is the whole hazard §24.1c exists to close.
#[test]
fn the_child_receives_exactly_one_credential_helper_and_no_askpass() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (recorded, _dest) = drive_a_clone(temp.path());

    let helpers = recorded
        .argv
        .iter()
        .filter(|a| a.starts_with("credential.helper="))
        .count();
    assert_eq!(
        helpers, 1,
        "expected exactly one credential.helper= in the child's argv, found {helpers}: {:?}",
        recorded.argv
    );
    assert!(
        recorded.argv.iter().any(|a| a == "credential.helper="),
        "the default tier clones anonymously, so the value is empty: {:?}",
        recorded.argv
    );
    assert!(
        recorded.argv.iter().any(|a| a == "core.askPass="),
        "core.askPass must be empty in the child's argv: {:?}",
        recorded.argv
    );
    assert_eq!(
        recorded.env_value("GIT_TERMINAL_PROMPT"),
        Some("0"),
        "without this an auth failure blocks on a TTY prompt instead of returning"
    );
    for banned in ["GIT_ASKPASS", "SSH_ASKPASS"] {
        assert!(
            !recorded.has_env(banned),
            "{banned} must be absent from the write path's environment"
        );
    }
    eprintln!(
        "git-write-audit: child received {} argv elements and {} environment entries",
        recorded.argv.len(),
        recorded.env.len()
    );
}

/// **Assertion 5**, asserted against the production entry point and never a fake.
///
/// The filter enumeration is the one git invocation in this lane that **neither audit covers**:
/// it is not in `core/src/git/`, so the read audit's directory scan misses it, and it is not an
/// `Intent` variant, so this file's enumeration of `Intent::ALL` misses it too. If it silently
/// returned nothing, every clone would render unfiltered and both audits would stay green. So it
/// is asserted here, behaviourally, from the child's own argv.
#[test]
fn a_configured_filter_driver_is_neutralised_in_the_child_argv() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (recorded, _dest) = drive_a_clone(temp.path());

    for attribute in ["process", "clean", "smudge"] {
        let wanted = format!("filter.auditdriver.{attribute}=");
        assert!(
            recorded.argv.contains(&wanted),
            "the driver the effective config declares must be neutralised: {wanted} missing from \
             {:?}",
            recorded.argv
        );
    }
    let scheme_ok = recorded
        .argv
        .iter()
        .any(|a| a.starts_with("https://") && !a.contains('@'));
    assert!(
        scheme_ok,
        "the clone URL must be https and carry no userinfo: {:?}",
        recorded.argv
    );
}

/// The negative half of the same question, and the reason [`FilterDrivers`] has one constructor.
///
/// With no driver configured the prefix carries **no** `filter.*` option — and the enumeration
/// still ran, which the type is what proves: `FilterDrivers` has no `Default` and no empty
/// constructor, so a `WriteEnv` can only hold drivers that came from a read which completed.
/// *Enumerated and found none* and *never enumerated* cannot collapse into one another.
#[test]
fn no_configured_driver_renders_no_filter_option_and_still_proves_the_read_ran() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hooks = temp.path().join("hooks-empty");
    std::fs::create_dir_all(&hooks).expect("hooks dir");

    // A copy of the stand-in whose name selects the matched-nothing answer, so the mode is
    // per-test rather than a process-global environment variable two parallel tests would race.
    let quiet = temp.path().join(if cfg!(windows) {
        "nofilters-git.exe"
    } else {
        "nofilters-git"
    });
    std::fs::copy(recording_git(), &quiet).expect("copy the stand-in");

    let dest = temp.path().join("quiet");
    let intent = Intent::Clone {
        url: codotheca_core::gitw::RemoteUrl::parse("https://forge.example/acme/widget.git")
            .expect("fixture url"),
        dest: dest.clone(),
        depth: None,
    };
    let backend = SystemMutatingGit::new(quiet, hooks);
    let cancel = CancelToken::new();
    backend
        .run(&intent, &cancel, &mut |_| {})
        .expect("a config that matches nothing is not a failure");

    let recorded = Recorded::read(&dest);
    assert!(
        !recorded.argv.iter().any(|a| a.starts_with("filter.")),
        "no driver is configured, so no filter option may be rendered: {:?}",
        recorded.argv
    );
    assert_eq!(
        recorded
            .argv
            .iter()
            .filter(|a| a.starts_with("credential.helper="))
            .count(),
        1,
        "the credential option is rendered whether or not a filter is"
    );
}

/// A clone whose filters cannot be enumerated is **refused, never run unfiltered** (§24.1b).
#[test]
fn a_failed_filter_enumeration_refuses_the_clone() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hooks = temp.path().join("hooks-empty");
    std::fs::create_dir_all(&hooks).expect("hooks dir");
    let dest = temp.path().join("never");
    let intent = Intent::Clone {
        url: codotheca_core::gitw::RemoteUrl::parse("https://forge.example/acme/widget.git")
            .expect("fixture url"),
        dest: dest.clone(),
        depth: None,
    };
    // A git that does not exist: the enumeration cannot complete.
    let backend = SystemMutatingGit::new(temp.path().join("no-such-git"), hooks);
    let cancel = CancelToken::new();
    let outcome = backend.run(&intent, &cancel, &mut |_| {});
    assert!(
        outcome.is_err(),
        "an unenumerable filter set must refuse rather than clone unfiltered"
    );
    assert!(
        !dest.exists(),
        "the refusal must happen before anything is created on disk"
    );
}

/// `write_base_args` renders the credential option once for **every** variant, including the one
/// with no production caller — so the property is not an accident of the clone path.
#[test]
fn every_variant_renders_exactly_one_credential_option() {
    let temp = tempfile::tempdir().expect("tempdir");
    let env = WriteEnv {
        work_dir: None,
        hooks_dir: temp.path().join("hooks-empty"),
        credential: CredentialChannel::anonymous(),
        filters: FilterDrivers::enumerated(Vec::new()),
    };
    let intents = rendered();
    for intent in &intents {
        let argv = write_base_args(intent, &env);
        let rendered: Vec<String> = argv
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let helpers = rendered
            .iter()
            .filter(|a| a.starts_with("credential.helper="))
            .count();
        assert_eq!(
            helpers,
            1,
            "{:?} rendered {helpers} credential.helper options: {rendered:?}",
            intent.kind()
        );
    }
}
