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

mod support;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::cancel::CancelToken;
use codotheca_core::gitw::{
    write_base_args, AdvertisedRef, AuditFixture, CredentialChannel, FilterDrivers, Intent,
    IntentKind, MutatingGit, SystemMutatingGit, VerifyStep, VerifyStepKind, WriteEnv, WriteExec,
};

/// §47.9 A's first rule, replacing §24.2a's `WRITE_ALLOWED`: **each intent's own subcommands**,
/// by step. A variant that renders another intent's subcommand fails, which a flat list of
/// allowed verbs could not see.
const SUBCOMMANDS_BY_INTENT: &[(IntentKind, Option<VerifyStepKind>, &str)] = &[
    (IntentKind::Clone, None, "clone"),
    (
        IntentKind::VerifyRead,
        Some(VerifyStepKind::ResolveUrl),
        "ls-remote",
    ),
    (
        IntentKind::VerifyRead,
        Some(VerifyStepKind::Advertise),
        "ls-remote",
    ),
    (
        IntentKind::VerifyRead,
        Some(VerifyStepKind::Objects),
        "fetch",
    ),
];

/// §24.2a assertion 2's deny list — the eight tokens that turn an additive invocation into a
/// destructive one.
///
/// [p4] §47.9 A keeps the eight and adds sixteen, each matched as the token **or its `<token>=`
/// form**. It stays the list that names the crime; the per-intent allow-list below is the
/// load-bearing half.
const FLAG_FORBIDDEN: &[&str] = &[
    "--prune",
    "--force",
    "-f",
    "--hard",
    "--delete",
    "-d",
    "-D",
    "--mirror",
    "--prune-tags",
    "-P",
    "--update-head-ok",
    "-u",
    "--unshallow",
    "--deepen",
    "--update-shallow",
    "--refetch",
    "--tags",
    "--write-fetch-head",
    "--recurse-submodules",
    "--sign",
    "-s",
    "--local-user",
    "--force-with-lease",
    "--force-if-includes",
];

/// The one exception to [`FLAG_FORBIDDEN`] (R192): `CloneBundle` needs `--mirror` — `--bare`
/// leaves notes and the stash dangling. Declared here and **unexercised** until the intent lands
/// with its caller; any other intent carrying it fails.
const FORBIDDEN_EXCEPTIONS: &[(&str, &str)] = &[("CloneBundle", "--mirror")];

/// §47.9 A: the options every write child carries before its subcommand — `-C` for an intent that
/// runs in a repository, and `-c` for each config pin.
const BASE_FLAGS: &[&str] = &["-C", "--no-optional-locks", "-c"];

/// §47.9 A's **per-intent flag allow-list, default-deny**: exactly each row of §47.2 and §47.3.
/// A flag not on its intent's row fails, whether or not [`FLAG_FORBIDDEN`] names it.
const FLAGS_BY_INTENT: &[(IntentKind, Option<VerifyStepKind>, &[&str])] = &[
    (IntentKind::Clone, None, &["--progress", "--depth"]),
    (
        IntentKind::VerifyRead,
        Some(VerifyStepKind::ResolveUrl),
        &["--get-url"],
    ),
    (IntentKind::VerifyRead, Some(VerifyStepKind::Advertise), &[]),
    (
        IntentKind::VerifyRead,
        Some(VerifyStepKind::Objects),
        &[
            "--refmap=",
            "--stdin",
            "--no-prune",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            "--no-write-commit-graph",
        ],
    ),
];

/// §47.3's uniform pins, on every child.
const UNIFORM_PINS: &[&str] = &["fetch.bundleURI=", "transfer.bundleURI=false"];

/// The floor `Intent::ALL` is held to. **Renamed once, in Lane 0** (PA36): two variants still,
/// one of them new. Two lanes raising it from one base resolve to the merged tree's
/// `Intent::ALL.len()`, never to a hand sum.
const INTENT_FLOOR: usize = 2;

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
    // **Both coverages, not one length**: every kind, and every step of the one that has steps.
    let kinds: Vec<IntentKind> = Intent::ALL
        .into_iter()
        .filter(|kind| intents.iter().any(|i| i.kind() == *kind))
        .collect();
    let steps: Vec<VerifyStepKind> = VerifyStep::ALL
        .into_iter()
        .filter(|step| intents.iter().any(|i| i.step() == Some(*step)))
        .collect();
    eprintln!(
        "git-write-audit: rendered {} intents — kinds {kinds:?}, steps {steps:?}",
        intents.len()
    );
    assert_eq!(
        kinds.len(),
        Intent::ALL.len(),
        "all_for_audit rendered {kinds:?} of {:?}; every assertion in this file loops over this \
         set, so a short one passes by not looking",
        Intent::ALL
    );
    assert_eq!(
        steps.len(),
        VerifyStep::ALL.len(),
        "all_for_audit rendered steps {steps:?} of {:?}",
        VerifyStep::ALL
    );
    assert!(
        !intents.is_empty(),
        "the write audit rendered zero variants"
    );
    intents
}

#[test]
fn every_rendered_subcommand_is_on_its_intents_row() {
    let intents = rendered();
    let mut tokens = 0;
    for intent in &intents {
        let argv = intent.argv();
        tokens += argv.len();
        let subcommand = argv.first().and_then(|arg| arg.to_str()).unwrap_or("");
        let row = SUBCOMMANDS_BY_INTENT
            .iter()
            .find(|(kind, step, _)| *kind == intent.kind() && *step == intent.step())
            .unwrap_or_else(|| panic!("{:?} {:?} has no row", intent.kind(), intent.step()));
        assert_eq!(
            subcommand,
            row.2,
            "{:?} {:?} rendered {subcommand:?}, not its own row's verb: {argv:?}",
            intent.kind(),
            intent.step()
        );
    }
    eprintln!(
        "git-write-audit: {} intents, {tokens} argv tokens, {} subcommand rows",
        intents.len(),
        SUBCOMMANDS_BY_INTENT.len()
    );
}

/// Every `-`-prefixed token of a child argv that is a flag, not a value: the token after `-c`,
/// `-C` or `--depth` is that option's value, and a lone `-` is a value (stdin), never a flag.
fn flags_of(argv: &[String]) -> Vec<&str> {
    let mut flags = Vec::new();
    let mut skip = false;
    for token in argv {
        if skip {
            skip = false;
            continue;
        }
        if matches!(token.as_str(), "-c" | "-C" | "--depth") {
            skip = true;
        }
        if token.starts_with('-') && token != "-" {
            flags.push(token.as_str());
        }
    }
    flags
}

/// The forbidden-flag check, as a function the planted proofs below feed directly.
fn forbidden_in(intent: &str, argv: &[String]) -> Result<(), String> {
    for flag in flags_of(argv) {
        for forbidden in FLAG_FORBIDDEN {
            let hit = flag == *forbidden || flag.starts_with(&format!("{forbidden}="));
            let excepted = FORBIDDEN_EXCEPTIONS
                .iter()
                .any(|(who, what)| *who == intent && what == forbidden);
            if hit && !excepted {
                return Err(format!(
                    "{intent} rendered forbidden flag {flag:?}: {argv:?}"
                ));
            }
        }
    }
    Ok(())
}

/// The default-deny allow-list check, as a function the planted proofs feed directly.
fn off_the_allow_list(
    kind: IntentKind,
    step: Option<VerifyStepKind>,
    argv: &[String],
) -> Result<(), String> {
    let row = FLAGS_BY_INTENT
        .iter()
        .find(|(k, s, _)| *k == kind && *s == step)
        .ok_or_else(|| format!("{kind:?} {step:?} has no flag row"))?;
    for flag in flags_of(argv) {
        if !BASE_FLAGS.contains(&flag) && !row.2.contains(&flag) {
            return Err(format!(
                "{kind:?} {step:?} rendered {flag:?}, which is not on its row: {argv:?}"
            ));
        }
    }
    Ok(())
}

/// Each pin exactly once — the check, as a function the planted proofs feed directly.
fn pins_once(pins: &[&str], argv: &[String]) -> Result<usize, String> {
    let mut counted = 0;
    for pin in pins {
        let seen = argv.iter().filter(|a| a.as_str() == *pin).count();
        if seen != 1 {
            return Err(format!("pin {pin:?} rendered {seen} times: {argv:?}"));
        }
        counted += 1;
    }
    Ok(counted)
}

/// Every intent and step, driven through the **production** `WriteExec` into the recording
/// stand-in, paired with what each child actually received. A clone is keyed on its destination
/// and a verifying read on its `-C` directory — the fixture's one path — so the recordings come
/// back in the order the intents ran.
fn children() -> Vec<(Intent, support::git_world::Recording)> {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture =
        AuditFixture::new(temp.path(), SecretToken::new(SENTINEL.to_owned())).expect("fixture");
    let intents = Intent::all_for_audit(&fixture);
    let env = authenticated_env(temp.path(), fixture.url().host(), None);
    std::fs::create_dir_all(&env.hooks_dir).expect("hooks dir");
    let exec = WriteExec::new(recording_git());
    for intent in &intents {
        exec.run(intent, &env, &CancelToken::new(), &mut |_| {})
            .expect("the recording stand-in exits 0");
    }
    let calls = support::git_world::read_recordings(fixture.dest());
    assert_eq!(calls.len(), intents.len(), "one child per intent and step");
    intents.into_iter().zip(calls).collect()
}

/// **AC-P4-47-2 — the flag denylist**, over every child's real argv: no `FLAG_FORBIDDEN` token and
/// no `<token>=` form, except the one printed exception.
#[test]
fn no_rendered_argv_contains_a_forbidden_flag() {
    let children = children();
    let mut scanned = 0;
    for (intent, call) in &children {
        scanned += flags_of(&call.argv).len();
        if let Err(finding) = forbidden_in(&format!("{:?}", intent.kind()), &call.argv) {
            panic!("{finding}");
        }
    }
    assert!(
        scanned > 0,
        "the flag denylist scanned zero flags, so it asserts nothing"
    );
    eprintln!(
        "git-write-audit: {scanned} flags in {} children checked against {} forbidden flags; \
         exceptions {FORBIDDEN_EXCEPTIONS:?}",
        children.len(),
        FLAG_FORBIDDEN.len()
    );
}

/// **AC-P4-47-2 — the per-intent allow-list, default-deny**: every flag each child carries is
/// on its own intent's row or is a base option.
#[test]
fn ac_p4_47_2() {
    let children = children();
    let mut checked = 0;
    for (intent, call) in &children {
        checked += flags_of(&call.argv).len();
        if let Err(finding) = off_the_allow_list(intent.kind(), intent.step(), &call.argv) {
            panic!("{finding}");
        }
    }
    assert!(checked > 0, "the allow-list checked zero flags");
    eprintln!(
        "git-write-audit: {checked} flags in {} children on their rows",
        children.len()
    );
}

/// **AC-P4-47-4 — pins and environment, from the child.** Every pin §47.3 names for the intent,
/// and both uniform pins, appear **exactly once**; `GIT_ALLOW_PROTOCOL` equals the intent's
/// list; the objects step carries the no-replace and absent-graft pins.
#[test]
fn ac_p4_47_4() {
    let children = children();
    let mut pins = 0;
    for (intent, call) in &children {
        let mut expected: Vec<&str> = intent.pins();
        expected.extend(UNIFORM_PINS);
        pins += pins_once(&expected, &call.argv).unwrap_or_else(|f| panic!("{f}"));
        assert_eq!(
            call.env_value("GIT_ALLOW_PROTOCOL"),
            Some(intent.allowed_protocols()),
            "{:?} {:?}",
            intent.kind(),
            intent.step()
        );
        let objects = intent.step() == Some(VerifyStepKind::Objects);
        assert_eq!(
            call.env_value("GIT_NO_REPLACE_OBJECTS").is_some(),
            objects,
            "the no-replace pin is the objects step's alone"
        );
        let graft = call.env_value("GIT_GRAFT_FILE");
        assert_eq!(
            graft.is_some(),
            objects,
            "the graft pin is the objects step's alone"
        );
        if let Some(path) = graft {
            assert!(
                !Path::new(path).exists(),
                "the graft file must never exist: {path}"
            );
        }
    }
    assert!(pins > 0, "zero pins counted");
    eprintln!(
        "git-write-audit: {pins} pins counted once each across {} children",
        children.len()
    );
}

/// **AC-P4-47-3 — layer A: stdin.** Every line the objects step's child reads parses as an
/// `AdvertisedRef`; no other step reads any.
#[test]
fn ac_p4_47_3() {
    let children = children();
    let mut lines = 0;
    for (intent, call) in &children {
        let text = String::from_utf8(call.stdin.clone()).expect("stdin is text");
        if intent.step() != Some(VerifyStepKind::Objects) {
            assert!(
                text.is_empty(),
                "{:?} {:?} read stdin",
                intent.kind(),
                intent.step()
            );
            continue;
        }
        for line in text.lines() {
            assert!(
                AdvertisedRef::parse(line).is_ok(),
                "a stdin line is not an AdvertisedRef: {line:?}"
            );
            lines += 1;
        }
    }
    assert!(
        lines > 0,
        "zero stdin lines parsed, so the grammar was never exercised"
    );
    eprintln!("git-write-audit: {lines} stdin lines parsed as AdvertisedRef");
}

/// **The planted proofs**: each checker, fed a shape it exists to refuse, refuses it — so a
/// passing run above is the product's, not a checker that accepts everything.
#[test]
fn the_checkers_refuse_what_they_exist_to_refuse() {
    let argv = |tokens: &[&str]| tokens.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>();
    let prune_tags = argv(&["fetch", "--prune-tags", "origin"]);
    assert!(forbidden_in("VerifyRead", &prune_tags).is_err());
    assert!(forbidden_in("TagArchived", &argv(&["tag", "-f", "archived"])).is_err());
    assert!(forbidden_in("VerifyRead", &argv(&["fetch", "--tags=x", "origin"])).is_err());
    assert!(forbidden_in("VerifyRead", &argv(&["clone", "--mirror", "b", "d"])).is_err());
    assert!(
        forbidden_in("CloneBundle", &argv(&["clone", "--mirror", "b", "d"])).is_ok(),
        "the one printed exception"
    );
    assert!(
        off_the_allow_list(
            IntentKind::VerifyRead,
            Some(VerifyStepKind::Objects),
            &argv(&["fetch", "--stdin", "--tags", "origin"]),
        )
        .is_err(),
        "a flag off the row fails whether or not the denylist names it"
    );
    assert!(pins_once(&["--stdin"], &argv(&["fetch", "--stdin", "--stdin"])).is_err());
    assert!(
        flags_of(&argv(&["tag", "-F", "-", "archived"])).contains(&"-F")
            && !flags_of(&argv(&["tag", "-F", "-"])).contains(&"-"),
        "a lone `-` is a value"
    );
    eprintln!("git-write-audit: 8 planted shapes refused, 1 exception admitted");
}

/// `Intent::ALL` is held to its floor. **Renamed once in Lane 0** from
/// `the_exhaustive_intent_list_has_two_renderable_variants` (PA36): the floor is still two, and
/// one of the two is new — `VerifyRead` replaced the retired `Fetch`.
#[test]
fn the_exhaustive_intent_list_matches_its_floor() {
    // `rendered` asserts every kind and every step was rendered.
    let intents = rendered();
    assert_eq!(
        INTENT_FLOOR,
        Intent::ALL.len(),
        "Intent::ALL changed; review every write-boundary assertion before accepting a new variant"
    );
    assert!(intents.len() >= Intent::ALL.len());
}

/// **AC-P2-24-4, in the form that is expressible here.**
///
/// No scheduler retries a write intent, so the criterion's *"under every scheduler retry path"*
/// cannot be exercised by driving one. What **is** provable, and is the property the criterion
/// rests on, is that `argv()` is a **pure function of the (variant, step)**: rendering the same
/// intent N times yields byte-identical argv, so no retry can produce a token the first attempt
/// did not. [p4] Its argv half stays true and **is not the claim** — §47.9 C's differential
/// layer is (`AC-P4-47-6`).
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

/// Drive a clone into the recording stand-in and read what the child received.
///
/// The recorder keys a clone on its destination, the absolute last argv element; an intent that
/// runs in a repository is keyed on its `-C` directory instead (see
/// `the_sentinel_reaches_no_verify_read_child`).
fn drive_recorded_clone(root: &Path, intent: &Intent, host: &str) -> Recorded {
    let Intent::Clone { dest, .. } = intent else {
        panic!("this helper drives a clone; the verifying read is keyed on its -C directory");
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
fn assert_no_sentinel(kind: IntentKind, argv: &[String], env: &[String]) {
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
    assert!(intents.len() >= Intent::ALL.len(), "audit variant floor");

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

/// **Assertion 3, for `VerifyRead`: over what each step's spawned child actually received.**
///
/// [p4] Renamed from `the_sentinel_reaches_no_rendered_fetch_argv_although_fetch_never_spawns`:
/// `Intent::Fetch` is retired, and the verifying read that replaces it **does** spawn in
/// production, so this now reads every step's child — keyed on its `-C` directory — rather than
/// a rendered argv (AC-P4-47-4).
#[test]
fn the_sentinel_reaches_no_verify_read_child() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture =
        AuditFixture::new(temp.path(), SecretToken::new(SENTINEL.to_owned())).expect("fixture");
    let steps: Vec<Intent> = Intent::all_for_audit(&fixture)
        .into_iter()
        .filter(|i| matches!(i, Intent::VerifyRead { .. }))
        .collect();
    assert_eq!(steps.len(), VerifyStep::ALL.len(), "every step is rendered");

    let env = authenticated_env(temp.path(), fixture.url().host(), None);
    std::fs::create_dir_all(&env.hooks_dir).expect("hooks dir");
    let exec = WriteExec::new(recording_git());
    for intent in &steps {
        exec.run(intent, &env, &CancelToken::new(), &mut |_| {})
            .expect("the recording stand-in exits 0");
    }
    let calls = support::git_world::read_recordings(fixture.dest());
    assert_eq!(calls.len(), steps.len(), "one child per step");
    for (intent, call) in steps.iter().zip(&calls) {
        assert!(
            call.argv
                .iter()
                .any(|arg| arg.starts_with("credential.helper=") && arg != "credential.helper="),
            "{:?} was not rendered with the authenticated sentinel channel: {:?}",
            intent.step(),
            call.argv
        );
        assert_no_sentinel(intent.kind(), &call.argv, &call.env);
        assert!(
            !String::from_utf8_lossy(&call.stdin).contains(SENTINEL),
            "the sentinel reached {:?}'s stdin",
            intent.step()
        );
    }
    eprintln!(
        "git-write-audit: {} verify-read children, sentinel in none",
        calls.len()
    );
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
        intents.len(),
        "the maintenance check looked at {checked} of {} rendered intents",
        intents.len()
    );
}

/// §47.3: **no scrub variable reaches a write child**, whatever the parent's environment holds.
///
/// `GIT_CONFIG_COUNT` and its keys delivered the same branch and tag deletions as repository
/// config (§47 M2), and `neutralise_env` is shared by both paths, so the write path is proved
/// separately rather than inferred from the read one. The child drives the production
/// `SystemMutatingGit` — filter enumeration and clone both — at the recording stand-in.
#[test]
fn no_scrub_variable_reaches_a_write_child_under_a_hostile_parent() {
    use support::git_world::{
        child_dir, hostile_parent_env, is_child, read_recording, run_in_child, scrub_report,
    };

    if is_child() {
        let dir = child_dir();
        let hooks = dir.join("hooks-empty");
        std::fs::create_dir_all(&hooks).expect("hooks dir");
        let intent = Intent::Clone {
            url: codotheca_core::gitw::RemoteUrl::parse("https://forge.example/acme/widget.git")
                .expect("fixture url"),
            dest: dir.join("widget"),
            depth: None,
        };
        SystemMutatingGit::new(recording_git(), hooks)
            .run(&intent, &CancelToken::new(), &mut |_| {})
            .expect("the recording stand-in exits 0");
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let hostile = hostile_parent_env(tmp.path());
    run_in_child(
        "no_scrub_variable_reaches_a_write_child_under_a_hostile_parent",
        tmp.path(),
        &hostile.all(),
    );
    let recorded = read_recording(&tmp.path().join("widget"));
    let report = scrub_report(&hostile, &recorded.env);
    eprintln!(
        "write-path scrub: {} planted, {} scrubbed variables reached the child {:?}, {} kept \
         variables lost {:?}",
        report.planted,
        report.leaked.len(),
        report.leaked,
        report.lost.len(),
        report.lost
    );
    assert!(
        report.planted > 0,
        "nothing was planted, so nothing was proved"
    );
    assert!(
        report.leaked.is_empty(),
        "scrubbed variables reached the write child: {:?}",
        report.leaked
    );
    assert!(
        report.lost.is_empty(),
        "the user's own config route must be kept: {:?}",
        report.lost
    );
}
