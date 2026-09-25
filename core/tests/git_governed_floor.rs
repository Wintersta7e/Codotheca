#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §47.8: the governed-git floor. Below it a governed intent spawns nothing but the version
//! probe and fails `TooOld`; `Clone` still runs; the features the floor exists for run on the git
//! under test; and a reftable repository reads correctly through git or yields unknown.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{
    parse_version, GitBackend as _, GitError, JobClass, JobContext, RefBackend, RepoHandle,
    StoreKey,
};
use codotheca_core::gitw::{
    meets_governed_floor, AdvertisedRef, Intent, MutatingGit as _, RemoteName, RemoteUrl,
    SystemMutatingGit, TransportFixture, VerifyStep, GOVERNED_GIT_FLOOR,
};
use codotheca_core::mount::StoreClass;
use support::git_world::{read_recordings, recording_git, test_git_version, World};

const fn ctx(cancel: &CancelToken) -> JobContext<'_> {
    JobContext::new(JobClass::Interactive, cancel, None)
}

/// The three verifying steps over `repo`'s `origin`.
fn verify_steps(repo: &Path) -> Vec<Intent> {
    [
        VerifyStep::ResolveUrl,
        VerifyStep::Advertise,
        VerifyStep::Objects {
            tips: vec![AdvertisedRef::parse("refs/heads/extra").expect("tip")],
        },
    ]
    .into_iter()
    .map(|step| Intent::VerifyRead {
        repo: repo.to_path_buf(),
        remote: RemoteName::parse("origin").expect("remote"),
        step,
    })
    .collect()
}

/// The stand-in under a name that selects its answer to `--version`.
///
/// A symbolic link on Unix, not a copy: a copy is a file this process holds open for writing,
/// and a child another test forks in that window inherits the handle, so exec'ing the copy fails
/// `ETXTBSY` (measured: *Text file busy*, once, under a parallel run). Windows has no such
/// failure and an unprivileged symbolic link there is refused, so it copies.
fn stand_in(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    });
    #[cfg(unix)]
    std::os::unix::fs::symlink(recording_git(), &path).expect("link the stand-in");
    #[cfg(not(unix))]
    std::fs::copy(recording_git(), &path).expect("copy the stand-in");
    path
}

/// **Below the floor a verifying read spawns nothing** — the stand-in records only the version
/// probe — and fails `TooOld` carrying the line git printed. **`Clone` still runs** (§47.8: its
/// pins are config keys an older git ignores).
#[test]
fn below_the_floor_a_verifying_read_spawns_nothing_and_a_clone_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hooks = temp.path().join("hooks-empty");
    std::fs::create_dir_all(&hooks).expect("hooks");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");
    let git = SystemMutatingGit::new(stand_in(temp.path(), "oldgit"), hooks.clone());

    let mut refused = 0;
    for intent in verify_steps(&repo) {
        let outcome = git.run(&intent, &CancelToken::new(), &mut |_| {});
        eprintln!("below the floor: {:?} -> {outcome:?}", intent.step());
        assert_eq!(
            outcome.map(|_| ()),
            Err(GitError::TooOld {
                found: "git version 2.28.0".to_owned()
            })
        );
        refused += 1;
    }
    assert!(
        !repo.with_file_name("repo.recorded").exists(),
        "a governed intent below the floor spawned: {:?}",
        read_recordings(&repo)
            .iter()
            .map(|r| r.argv.clone())
            .collect::<Vec<_>>()
    );
    let probes = read_recordings(&hooks);
    let probe_argv: Vec<Vec<String>> = probes.iter().map(|r| r.argv.clone()).collect();
    eprintln!("below the floor: the stand-in recorded {probe_argv:?}");
    assert_eq!(
        probe_argv,
        vec![vec!["--version".to_owned()]],
        "the version is probed once and is the only spawn"
    );

    let dest = temp.path().join("dest");
    let clone = Intent::Clone {
        url: RemoteUrl::parse("https://forge.example/acme/widget.git").expect("url"),
        dest: dest.clone(),
        depth: None,
    };
    let cloned = git.run(&clone, &CancelToken::new(), &mut |_| {});
    let clone_calls = read_recordings(&dest);
    eprintln!(
        "below the floor: clone -> {:?}, {} recorded",
        cloned.map(|_| ()),
        clone_calls.len()
    );
    assert_eq!(clone_calls.len(), 1, "a clone below the floor still runs");
    assert_eq!(
        clone_calls[0].argv.iter().find(|a| *a == "clone"),
        Some(&"clone".to_owned())
    );
    assert_eq!(refused, 3);
}

/// At or above the floor the verifying read runs, and the version is probed once per write path
/// however many intents it runs.
#[test]
fn above_the_floor_the_read_runs_and_the_version_is_probed_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hooks = temp.path().join("hooks-empty");
    std::fs::create_dir_all(&hooks).expect("hooks");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");
    let git = SystemMutatingGit::new(recording_git(), hooks.clone());
    for intent in verify_steps(&repo) {
        git.run(&intent, &CancelToken::new(), &mut |_| {})
            .expect("the stand-in answers above the floor");
    }
    let calls = read_recordings(&repo);
    let probes = read_recordings(&hooks);
    eprintln!(
        "above the floor: {} steps recorded, {} version probe(s)",
        calls.len(),
        probes.len()
    );
    assert_eq!(calls.len(), 3);
    assert_eq!(probes.len(), 1);
}

/// The floor compares all three components, and reads a vendor suffix the way `parse_version`
/// does.
#[test]
fn the_floor_compares_major_minor_and_patch() {
    let (major, minor, patch) = GOVERNED_GIT_FLOOR;
    let cases = [
        (format!("git version {major}.{minor}.{patch}"), true),
        (
            format!("git version {major}.{minor}.{patch}.windows.1"),
            true,
        ),
        (format!("git version {major}.{}.0", minor + 1), true),
        (format!("git version {}.0.0", major + 1), true),
        (format!("git version {major}.{}.9", minor - 1), false),
        ("git version 2.28.0".to_owned(), false),
    ];
    for (line, expected) in &cases {
        let version = parse_version(line.as_bytes()).expect("parses");
        assert_eq!(meets_governed_floor(&version), *expected, "{line}");
    }
    eprintln!(
        "floor {major}.{minor}.{patch}: {} version lines compared",
        cases.len()
    );
}

/// **Every floor feature, on the git under test** (`CODOTHECA_TEST_GIT`, or `PATH`'s): the three
/// verifying steps through the production write path — `ls-remote --get-url`, `ls-remote`, and
/// `fetch --refmap= --stdin --no-write-fetch-head --no-write-commit-graph` — and the two reads
/// the analyser leans on, `rev-parse --is-shallow-repository` and `status --ignored=matching`.
#[test]
fn every_floor_feature_runs_on_the_git_under_test() {
    let temp = tempfile::tempdir().expect("tempdir");
    let world = World::build(temp.path(), false);
    let hooks =
        codotheca_core::git::ensure_empty_hooks_dir(&world.root.join("app-data")).expect("hooks");
    let write_git = SystemMutatingGit::with_transport_fixture(
        support::test_git(),
        hooks.clone(),
        TransportFixture::new(&world.root),
    );
    let version = test_git_version();
    let mut features = Vec::new();
    for intent in verify_steps(&world.work) {
        let flags: Vec<String> = intent
            .argv()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .filter(|a| a.starts_with("--") || a == "ls-remote" || a == "fetch")
            .collect();
        write_git
            .run(&intent, &CancelToken::new(), &mut |_| {})
            .unwrap_or_else(|e| panic!("{version}: {flags:?} failed: {e:?}"));
        features.push(flags.join(" "));
    }
    let read_git = codotheca_core::git::SystemGit::new(
        std::sync::Arc::new(codotheca_core::git::GitExec::new(
            support::test_git(),
            hooks,
        )),
        std::sync::Arc::new(codotheca_core::git::GitSlots::for_machine()),
        std::sync::Arc::new(codotheca_core::clock::SystemClock::new()),
    );
    // D10's graft file makes git print a deprecation hint on every read, and the analyser's
    // reads refuse on any diagnostic output (§45.2). That refusal is the rule, not this test's
    // subject, so the reads here run without it.
    std::fs::remove_file(world.work.join(".git/info/grafts")).expect("grafts");
    let cancel = CancelToken::new();
    let handle =
        RepoHandle::resolve(&world.work, StoreKey::new("s"), StoreClass::Local).expect("resolves");
    let facts = read_git.repo_facts(&handle, &ctx(&cancel)).expect("facts");
    assert!(!facts.is_shallow);
    features.push("rev-parse --is-shallow-repository".to_owned());
    let scan = read_git
        .worktree_scan(&handle, &ctx(&cancel))
        .expect("status");
    assert!(!scan.entries.is_empty());
    features.push("status --ignored=matching".to_owned());
    eprintln!("{version}: floor features exercised: {features:?}");
    assert_eq!(features.len(), 5);
}

/// `git <args>` from the git under test, isolated from the developer's config.
fn under_test(cwd: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(support::test_git());
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            cmd.env_remove(key);
        }
    }
    cmd.current_dir(cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", cwd.join("absent.gitconfig"))
        .env("HOME", cwd)
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("git under test")
}

/// **A reftable repository reads correctly through git or yields unknown — never safe** (§47.8:
/// reftable is a test term, not a version term). Where the git under test cannot create one the
/// test prints why and counts as not run (R207).
#[test]
fn a_reftable_repository_reads_correctly_or_yields_unknown() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical");
    let repo = root.join("reftable");
    let version = test_git_version();
    let init = under_test(
        &root,
        &[
            "init",
            "-q",
            "-b",
            "main",
            "--ref-format=reftable",
            "reftable",
        ],
    );
    if !init.status.success() {
        eprintln!(
            "skipped: {version} has no reftable ({})",
            String::from_utf8_lossy(&init.stderr).trim()
        );
        return;
    }
    std::fs::write(repo.join("a.txt"), b"one\n").expect("file");
    for args in [
        &["add", "a.txt"][..],
        &["commit", "-q", "-m", "one"],
        &["branch", "feature"],
        &["tag", "-a", "v1", "-m", "v1"],
    ] {
        let out = under_test(&repo, args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let oid = |name: &str| {
        String::from_utf8_lossy(&under_test(&repo, &["rev-parse", name]).stdout)
            .trim()
            .to_owned()
    };
    let expected = [
        ("refs/heads/feature", oid("refs/heads/feature")),
        ("refs/heads/main", oid("refs/heads/main")),
        ("refs/tags/v1", oid("refs/tags/v1")),
    ];

    let hooks = codotheca_core::git::ensure_empty_hooks_dir(&root.join("app-data")).expect("hooks");
    let read_git = codotheca_core::git::SystemGit::new(
        std::sync::Arc::new(codotheca_core::git::GitExec::new(
            support::test_git(),
            hooks,
        )),
        std::sync::Arc::new(codotheca_core::git::GitSlots::for_machine()),
        std::sync::Arc::new(codotheca_core::clock::SystemClock::new()),
    );
    let cancel = CancelToken::new();
    let handle =
        RepoHandle::resolve(&repo, StoreKey::new("s"), StoreClass::Local).expect("resolves");
    match read_git.enumerate_refs(&handle, &ctx(&cancel)) {
        Ok(listing) => {
            assert_eq!(listing.backend, RefBackend::Reftable);
            for (name, want) in &expected {
                let found = listing.refs.iter().find(|r| r.name == *name);
                assert_eq!(
                    found.map(|r| r.oid.as_str()),
                    Some(want.as_str()),
                    "{version}: {name} misread from a reftable repository"
                );
            }
            eprintln!(
                "{version}: reftable read correctly, {} refs, {} checked",
                listing.refs.len(),
                expected.len()
            );
        }
        Err(e) => eprintln!("{version}: reftable yields unknown: {e:?}"),
    }
}
