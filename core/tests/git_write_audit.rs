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

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::cancel::CancelToken;
use codotheca_core::gitw::{
    write_base_args, AuditFixture, CredentialChannel, FilterDrivers, Intent, MutatingGit,
    SystemMutatingGit, WriteEnv, WriteExec,
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
    fn read(dest: &Path) -> Self {
        let mut path = dest.to_path_buf();
        let mut name = path
            .file_name()
            .expect("destination has a file name")
            .to_os_string();
        name.push(".recorded");
        path.set_file_name(name);
        Self::read_path(&path)
    }

    fn read_path(path: &Path) -> Self {
        let blob = std::fs::read(path).unwrap_or_else(|e| {
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
        Self { argv, env }
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

fn authenticated_env(root: &Path, host: &str, work_dir: Option<PathBuf>) -> WriteEnv {
    WriteEnv {
        work_dir,
        hooks_dir: root.join("hooks-empty"),
        credential: CredentialChannel::one_shot(&SecretToken::new(SENTINEL.to_owned()), host)
            .expect("one-shot credential channel"),
        filters: FilterDrivers::enumerated(Vec::new()),
    }
}

/// Drive one variant into the recording stand-in and read what the child received.
///
/// **Only `Clone` can be spawn-recorded, and that is a property of the variant rather than a gap
/// in the audit.** The recorder keys its output on the last argv element, which must be an
/// absolute path; `Intent::Fetch` renders its **remote name** last, so there is nothing to key on
/// and the stand-in refuses. An earlier version of this helper pointed the `Fetch` case at a file
/// name it invented, which is a test reading a recording no child wrote.
fn drive_recorded_clone(root: &Path, intent: &Intent, host: &str) -> Recorded {
    let Intent::Clone { dest, .. } = intent else {
        panic!("only Intent::Clone can be spawn-recorded; Fetch renders no path to key on");
    };
    let env = authenticated_env(root, host, None);
    std::fs::create_dir_all(&env.hooks_dir).expect("hooks dir");
    let cancel = CancelToken::new();
    WriteExec::new(recording_git())
        .run(intent, &env, &cancel, &mut |_| {})
        .expect("the recording stand-in exits 0");
    Recorded::read(dest)
}

/// The sentinel must appear in none of these, whichever way the argv was obtained.
fn assert_no_sentinel(kind: codotheca_core::gitw::IntentKind, argv: &[String], env: &[String]) {
    let mut urls = 0;
    for arg in argv {
        assert!(
            !arg.contains(SENTINEL),
            "credential sentinel appeared in argv for {kind:?}: {arg:?}"
        );
        if let Some(url) = arg.strip_prefix("https://") {
            urls += 1;
            let authority = url.split(['/', '?', '#']).next().unwrap_or("");
            assert!(
                !authority.contains('@'),
                "credential sentinel URL userinfo reached argv for {kind:?}: {arg:?}"
            );
        }
    }
    for config in argv
        .windows(2)
        .filter_map(|pair| (pair.first().map(String::as_str) == Some("-c")).then_some(&pair[1]))
    {
        assert!(
            !config.contains(SENTINEL),
            "credential sentinel appeared in -c config for {kind:?}: {config:?}"
        );
    }
    for entry in env {
        assert!(
            !entry.contains(SENTINEL),
            "credential sentinel appeared in the environment for {kind:?}: {entry:?}"
        );
    }
    assert!(urls <= 1, "a variant rendered more than one https URL");
}

/// **Assertion 3, for `Clone`: over what the spawned child actually received.**
///
/// The channel is built from the sentinel, and the helper value is required to be **non-empty** —
/// an anonymous channel would make every absence assertion below vacuously true.
#[test]
fn the_sentinel_reaches_no_child_argv_config_url_or_environment() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture =
        AuditFixture::new(temp.path(), SecretToken::new(SENTINEL.to_owned())).expect("fixture");
    let intents = Intent::all_for_audit(&fixture);
    assert_eq!(intents.len(), Intent::ALL.len(), "audit variant floor");

    let clone = intents
        .iter()
        .find(|i| matches!(i, Intent::Clone { .. }))
        .expect("the exhaustive set contains a Clone");
    let recorded = drive_recorded_clone(temp.path(), clone, fixture.url().host());
    assert!(
        recorded
            .argv
            .iter()
            .any(|arg| arg.starts_with("credential.helper=") && arg != "credential.helper="),
        "Clone was not rendered with the authenticated sentinel channel: {:?}",
        recorded.argv
    );
    assert_no_sentinel(clone.kind(), &recorded.argv, &recorded.env);
}

/// **Assertion 3, for `Fetch`: over the rendered argv, and stated as such.**
///
/// `Intent::Fetch` **cannot be spawn-recorded**: it renders a remote name rather than a path, so
/// the stand-in has nothing to key its recording on and refuses. It also has no production caller
/// in this plan — the in-session fetch is p2-24b's §24.7C — and `SystemMutatingGit::run` refuses
/// it outright rather than guessing a repository.
///
/// So this half asserts over `write_base_args`'s output rather than over a child's. **That is a
/// weaker claim than the `Clone` half and it is labelled weaker**: claiming it observed a spawn
/// would be the bar written past its defect. What it does prove is that the credential never
/// enters the rendering, which is the only layer `Fetch` has.
#[test]
fn the_sentinel_reaches_no_rendered_fetch_argv_although_fetch_never_spawns() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture =
        AuditFixture::new(temp.path(), SecretToken::new(SENTINEL.to_owned())).expect("fixture");
    let fetch = Intent::all_for_audit(&fixture)
        .into_iter()
        .find(|i| matches!(i, Intent::Fetch { .. }))
        .expect("the exhaustive set contains a Fetch");

    let env = authenticated_env(
        temp.path(),
        fixture.url().host(),
        Some(temp.path().to_path_buf()),
    );
    let mut argv: Vec<String> = write_base_args(&fetch, &env)
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    argv.extend(
        fetch
            .argv()
            .iter()
            .map(|a| a.to_string_lossy().into_owned()),
    );
    assert!(
        argv.iter()
            .any(|arg| arg.starts_with("credential.helper=") && arg != "credential.helper="),
        "Fetch was not rendered with the authenticated sentinel channel: {argv:?}"
    );
    assert_no_sentinel(fetch.kind(), &argv, &[]);
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("spawn fixture git");
    assert!(
        output.status.success(),
        "fixture git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A `file://` URL for a local fixture repository.
///
/// **`canonicalize` returns a VERBATIM path on Windows** — `\\?\C:\…` — and joining that naively
/// produced `file:////?/C:/…`, which git rejected with *"does not appear to be a git repository"*
/// against a repository that was plainly there. It failed **only** on Windows and **only** in the
/// one test that drives a real clone; every WSL gate was green over it. The `\\?\` prefix is an
/// API-level escape from the 260-character limit rather than part of the path's identity, so it
/// is stripped before the URL is built.
fn file_url(path: &Path) -> String {
    let canonical = path.canonicalize().expect("canonical fixture path");
    let text = canonical.to_string_lossy();
    // `\\?\UNC\server\share` is the other verbatim form. These fixtures are always local, so the
    // drive form is the only one reachable here and a UNC path would need a different URL shape.
    let raw = text
        .strip_prefix(r"\\?\")
        .unwrap_or(&text)
        .replace('\\', "/");
    let mut encoded = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    if cfg!(windows) {
        format!("file:///{encoded}")
    } else {
        format!("file://{encoded}")
    }
}

fn clone_url_arg(intent: &Intent) -> String {
    intent
        .argv()
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .find(|arg| arg.starts_with("https://"))
        .expect("clone argv has an https URL")
}

fn origin_url(config: &str) -> Option<&str> {
    let mut in_origin = false;
    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_origin = trimmed == r#"[remote "origin"]"#;
        } else if in_origin {
            if let Some(value) = trimmed.strip_prefix("url = ") {
                return Some(value);
            }
        }
    }
    None
}

/// **Assertion 7**, over the bytes Git persisted rather than the argv that asked it to clone.
#[test]
fn a_real_clone_persists_no_sentinel_and_an_origin_without_userinfo() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = temp.path().join("fixture remote");
    std::fs::create_dir_all(&source).expect("fixture directory");
    run_git(&source, &["init", "--quiet"]);
    std::fs::write(source.join("README.md"), "fixture\n").expect("fixture file");
    run_git(&source, &["add", "README.md"]);
    run_git(
        &source,
        &[
            "-c",
            "user.name=Fixture Author",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );

    let dest = temp.path().join("produced clone");
    let intent = Intent::Clone {
        url: codotheca_core::gitw::RemoteUrl::parse(
            "https://fixture.example.invalid/acme/widget.git",
        )
        .expect("fixture URL"),
        dest: dest.clone(),
        depth: Some(1),
    };
    let rendered_url = clone_url_arg(&intent);
    let env = authenticated_env(temp.path(), "fixture.example.invalid", None);
    std::fs::create_dir_all(&env.hooks_dir).expect("hooks dir");

    let mut command = Command::new("git");
    command.args(write_base_args(&intent, &env));
    command.args([
        "-c",
        &format!("url.{}.insteadOf={rendered_url}", file_url(&source)),
    ]);
    command.args(intent.argv());
    codotheca_core::git::neutralise_env(&mut command);
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn real clone");
    assert!(
        output.status.success(),
        "real fixture clone failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let config = std::fs::read_to_string(dest.join(".git/config")).expect("produced .git/config");
    assert!(
        !config.contains(SENTINEL),
        "produced clone's .git/config contains credential sentinel {SENTINEL}: {config}"
    );
    let persisted = origin_url(&config).expect("origin URL in produced .git/config");
    let authority = persisted
        .strip_prefix("https://")
        .expect("origin URL stays https")
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    assert!(
        !authority.contains('@'),
        "produced clone's origin URL carries userinfo: {persisted}"
    );
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
    // The recording is what this returns; the child's stderr lines are not part of it.
    backend
        .run(&intent, &cancel, &mut |_stderr_line| {})
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

/// **AC-P2-24-22's second half: an invalid credential RETURNS, it does not hang.**
///
/// This is what `GIT_TERMINAL_PROMPT=0` buys, and it is asserted rather than assumed. Without it
/// git blocks on a TTY prompt forever against a remote that refuses the credential, and the clone
/// never comes back — a hang is worse than a failure, because a failure has a rendering.
///
/// **Forced, not raced** (R72). The remote is a `file://` URL naming a path that does not exist,
/// so git fails on a real remote it can evaluate immediately, and the deadline is a wall-clock
/// bound this test would blow through by seconds rather than milliseconds if the prompt fired.
/// Nothing here sleeps and nothing waits for a timing window to open.
#[test]
fn an_unusable_remote_returns_rather_than_blocking_on_a_prompt() {
    let temp = tempfile::tempdir().expect("tempdir");
    let env = authenticated_env(temp.path(), "fixture.example.invalid", None);
    std::fs::create_dir_all(&env.hooks_dir).expect("hooks dir");

    // The URL git actually contacts is rewritten to a path that is not there; the intent's own
    // URL stays https, because a non-https scheme cannot be built at all.
    let dest = temp.path().join("never-arrives");
    let intent = Intent::Clone {
        url: codotheca_core::gitw::RemoteUrl::parse("https://fixture.example.invalid/acme/w.git")
            .expect("fixture URL"),
        dest: dest.clone(),
        depth: Some(1),
    };
    // A directory that exists and is not a repository. `file_url` canonicalises, so a path that
    // is simply absent cannot be named; an empty directory fails the clone for a reason git
    // evaluates locally and immediately, which is what keeps this deterministic.
    let not_a_repo = temp.path().join("not-a-repository");
    std::fs::create_dir_all(&not_a_repo).expect("empty directory");
    let missing = file_url(&not_a_repo);
    let rendered_url = clone_url_arg(&intent);

    let started = std::time::Instant::now();
    let mut command = Command::new("git");
    command.args(write_base_args(&intent, &env));
    command.args(["-c", &format!("url.{missing}.insteadOf={rendered_url}")]);
    command.args(intent.argv());
    codotheca_core::git::neutralise_env(&mut command);
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("git runs");
    let elapsed = started.elapsed();

    assert!(
        !output.status.success(),
        "a remote that is not there must fail rather than succeed"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "the clone took {elapsed:?}; with GIT_TERMINAL_PROMPT unset git blocks on a prompt and \
         never returns at all"
    );
    assert!(
        // `is_ok_and` rather than `map(..).unwrap_or(..)`: an unreadable destination is not the
        // same as an empty one, and treating it as empty would make this pass on a failure it
        // cannot see.
        !dest.exists() || std::fs::read_dir(&dest).is_ok_and(|d| d.count() == 0),
        "a failed clone must leave no populated destination behind"
    );
    eprintln!("git-write-audit: an unusable remote returned in {elapsed:?}");
}

/// [p2-24b] §24.1: *"An invocation may create bytes and may never remove or overwrite one."*
///
/// **`git fetch` spawns `git maintenance run --auto`, and that prunes.** Measured rather than
/// reasoned about: `GIT_TRACE=1 git -c gc.auto=1 fetch origin` shows
/// `run_command: git maintenance run --auto --no-quiet` as a child, and the same fetch carrying
/// the three options below renders no such line. `gc.auto`'s default threshold is 6,700 loose
/// objects, so without them §24.1's invariant holds by luck about a repository's shape rather
/// than by construction — and §24.7C's pre-flight runs a real fetch against a user's working copy.
///
/// Asserted on the **child's own argv**, not on `Intent::argv()`: these are base arguments, and a
/// test reading the intent's rendering would not see them at all.
#[test]
fn the_child_disables_the_maintenance_that_would_prune() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (recorded, _dest) = drive_a_clone(temp.path());

    for option in ["gc.auto=0", "gc.autoDetach=false", "maintenance.auto=false"] {
        assert!(
            recorded.argv.iter().any(|a| a == option),
            "{option} must be in the child's argv, or a fetch may spawn a pruning child: {:?}",
            recorded.argv
        );
    }
}

/// The same three, rendered for **every** variant rather than for the one that happens to spawn.
///
/// A clone is the variant the recorder can drive; a fetch is the one that actually triggers
/// maintenance. Asserting only the driveable one would leave the variant that matters uncovered,
/// which is the shape of a bar written past its own subject.
#[test]
fn every_variant_renders_the_maintenance_options() {
    let temp = tempfile::tempdir().expect("tempdir");
    let intents = rendered();
    let env = authenticated_env(temp.path(), "forge.example", None);
    let mut checked = 0;
    for intent in &intents {
        let argv: Vec<String> = write_base_args(intent, &env)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        for option in ["gc.auto=0", "gc.autoDetach=false", "maintenance.auto=false"] {
            assert!(
                argv.iter().any(|a| a == option),
                "{:?} renders no {option}: {argv:?}",
                intent.kind()
            );
        }
        checked += 1;
    }
    assert_eq!(
        checked,
        Intent::ALL.len(),
        "the maintenance check looked at {checked} of {} variants",
        Intent::ALL.len()
    );
}
