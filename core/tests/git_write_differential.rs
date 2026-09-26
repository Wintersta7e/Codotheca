#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![cfg(feature = "testkit")]
//! §47.9 C — the differential layer: what an intent **writes**, compared on disk, under config
//! that is trying to make it write more.
//!
//! The argv audit (`git_write_audit.rs`) passed while a user's config made the audited fetch
//! delete a checked-out branch (§47.1). This file therefore reads the repository before and after
//! and asserts the difference, rather than asserting what was asked for.

mod support;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::neutralise_env;
use codotheca_core::gitw::{
    write_base_args, AdvertisedRef, AuditFixture, CredentialChannel, FilterDrivers, Intent,
    MutatingGit, RemoteName, RemoteUrl, SystemMutatingGit, VerifyStep, WriteEnv,
};
use support::git_world::{read_recording, recording_git};
use support::TestRepo;

/// The two uniform pins §47.3 renders on every intent. `fetch.bundleURI` wrote `refs/bundles/*`
/// and `.git/config` under every other pin, and only the empty value stopped it (M3).
const BUNDLE_PINS: [&str; 2] = ["fetch.bundleURI=", "transfer.bundleURI=false"];

fn anonymous_env(hooks: &Path) -> WriteEnv {
    WriteEnv {
        work_dir: None,
        hooks_dir: hooks.to_path_buf(),
        credential: CredentialChannel::anonymous(),
        filters: FilterDrivers::enumerated(Vec::new()),
    }
}

/// **The child each intent spawns carries each bundle pin exactly once**, and the transport list
/// its intent declares — read off what the recording stand-in received, not off `argv()`.
#[test]
fn every_intent_renders_the_bundle_uri_pins_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hooks = temp.path().join("hooks-empty");
    std::fs::create_dir_all(&hooks).expect("hooks dir");
    let fixture = AuditFixture::new(
        temp.path(),
        codotheca_core::accounts::keychain::SecretToken::new("unused".to_owned()),
    )
    .expect("fixture");
    let intents = Intent::all_for_audit(&fixture);
    assert!(!intents.is_empty(), "no intent rendered");
    let backend = SystemMutatingGit::new(recording_git(), hooks);
    let mut checked = 0;
    for intent in &intents {
        backend
            .run(intent, &CancelToken::new(), &mut |_| {})
            .expect("the recording stand-in exits 0");
        // A clone is keyed on its destination; an intent that runs in a repository on its `-C`
        // directory. Both are the fixture's one path.
        let recorded = read_recording(fixture.dest());
        for pin in BUNDLE_PINS {
            let count = recorded.argv.iter().filter(|a| *a == pin).count();
            assert_eq!(
                count,
                1,
                "{:?} rendered {pin} {count} times: {:?}",
                intent.kind(),
                recorded.argv
            );
        }
        let allowed = recorded
            .env
            .iter()
            .find_map(|e| e.strip_prefix("GIT_ALLOW_PROTOCOL="));
        assert_eq!(
            allowed,
            Some(intent.allowed_protocols()),
            "{:?}'s child must carry its own transport list",
            intent.kind()
        );
        checked += 1;
        eprintln!(
            "bundle pins: {:?} {:?} — {} argv tokens, GIT_ALLOW_PROTOCOL={allowed:?}",
            intent.kind(),
            intent.step(),
            recorded.argv.len()
        );
    }
    assert_eq!(
        checked,
        intents.len(),
        "every rendered intent and step is checked"
    );
    assert!(
        checked > Intent::ALL.len(),
        "the verifying read's steps were rendered"
    );
}

fn run_fixture_git(cwd: &Path, home: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(cwd)
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .args(args)
        .output()
        .expect("fixture git");
    assert!(
        out.status.success(),
        "fixture git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `refs/bundles/*` in `repo`, through git.
fn bundle_refs(repo: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["for-each-ref", "--format=%(refname)", "refs/bundles"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("for-each-ref");
    assert!(out.status.success(), "for-each-ref failed");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// §47.9 C's bundle fixture: a source repository, a bundle of it, and a creation-token bundle
/// list pointing at that bundle.
struct BundleWorld {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    src: PathBuf,
    bundle: PathBuf,
    token_list: PathBuf,
}

impl BundleWorld {
    fn new() -> Self {
        let src = TestRepo::init();
        src.write("a.txt", b"one");
        src.commit("one");
        src.write("b.txt", b"two");
        src.commit("two");
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        let home = root.join("home");
        std::fs::create_dir_all(&home).expect("home");
        let src_path = root.join("src");
        run_fixture_git(
            &root,
            &home,
            &[
                "clone",
                "-q",
                &src.path().to_string_lossy(),
                &src_path.to_string_lossy(),
            ],
        );
        let bundle = root.join("all.bundle");
        run_fixture_git(
            &src_path,
            &home,
            &["bundle", "create", &bundle.to_string_lossy(), "--all"],
        );
        let token_list = root.join("list.cfg");
        std::fs::write(
            &token_list,
            format!(
                "[bundle]\n\tversion = 1\n\tmode = all\n\theuristic = creationToken\n\
                 [bundle \"one\"]\n\turi = {}\n\tcreationToken = 1\n",
                bundle.to_string_lossy().replace('\\', "/")
            ),
        )
        .expect("bundle list");
        Self {
            _dir: dir,
            root,
            home,
            src: src_path,
            bundle,
            token_list,
        }
    }

    /// A clone of the source whose own config names `bundle_uri` — the repository-config route.
    fn work_repo(&self, name: &str, bundle_uri: &Path) -> PathBuf {
        let work = self.root.join(name);
        run_fixture_git(
            &self.root,
            &self.home,
            &[
                "clone",
                "-q",
                &self.src.to_string_lossy(),
                &work.to_string_lossy(),
            ],
        );
        run_fixture_git(
            &work,
            &self.home,
            &["config", "fetch.bundleURI", &bundle_uri.to_string_lossy()],
        );
        work
    }
}

/// The verifying read's objects step in `repo`, fetching the source's branch — the one step that
/// runs `fetch`, and so the one `fetch.bundleURI` reaches.
fn objects_intent(repo: &Path) -> Intent {
    Intent::VerifyRead {
        repo: repo.to_path_buf(),
        remote: RemoteName::parse("origin").expect("remote"),
        step: VerifyStep::Objects {
            tips: vec![AdvertisedRef::parse("refs/heads/main").expect("tip")],
        },
    }
}

/// The production intent's child, minus exactly the listed `-c` pins — the **bite**, rendered in
/// the test so the product is never built without them — run to completion with its stdin
/// payload and `extra` environment set after the intent's own pins.
fn run_stripped(
    intent: &Intent,
    env: &WriteEnv,
    strip: &[&str],
    extra: &[(&str, OsString)],
) -> std::process::Output {
    use std::io::Write as _;

    let base: Vec<OsString> = write_base_args(intent, env);
    let mut kept: Vec<OsString> = Vec::with_capacity(base.len());
    let mut i = 0;
    while i < base.len() {
        if base[i] == "-c"
            && base
                .get(i + 1)
                .is_some_and(|v| strip.iter().any(|s| v == *s))
        {
            i += 2;
            continue;
        }
        kept.push(base[i].clone());
        i += 1;
    }
    let mut cmd = Command::new(support::test_git());
    cmd.args(kept);
    cmd.args(intent.argv());
    neutralise_env(&mut cmd);
    for (key, value) in intent.env_pins(&env.hooks_dir) {
        cmd.env(key, value);
    }
    for (key, value) in extra {
        cmd.env(key, value);
    }
    let payload = intent.stdin_payload();
    cmd.stdin(if payload.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("fixture child");
    if let Some(bytes) = payload {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(&bytes).expect("stdin written");
    }
    child.wait_with_output().expect("fixture child ends")
}

/// **AC-P4-47-8 (M3), the fetch half, through the production backend.** A repository whose own
/// config points `fetch.bundleURI` at a local bundle, and separately at a creation-token list:
/// the fetch — refused on its transport, as M3's was — writes no `refs/bundles/*` and not one
/// byte of `.git/config`.
///
/// **The hazard is proved live in the same run**: the same child with `fetch.bundleURI=` stripped
/// does write the ref, so a fixture that stopped reproducing M3 fails here instead of passing.
#[test]
fn a_bundle_uri_in_repo_config_writes_no_bundle_ref_and_no_config_byte() {
    if !support::git_world::test_git_lists_key("fetch.bundleURI") {
        eprintln!(
            "skipped: {} has no fetch.bundleURI, so M3's fetch half cannot occur",
            support::git_world::test_git_version()
        );
        return;
    }
    let world = BundleWorld::new();
    let hooks = world.root.join("hooks-empty");
    std::fs::create_dir_all(&hooks).expect("hooks dir");
    let backend = SystemMutatingGit::new(support::test_git(), hooks.clone());
    let mut cases = 0;
    for (name, uri) in [("plain", &world.bundle), ("token-list", &world.token_list)] {
        let work = world.work_repo(name, uri);
        let config_before = std::fs::read(work.join(".git").join("config")).expect("config");
        let intent = objects_intent(&work);
        let outcome = backend.run(&intent, &CancelToken::new(), &mut |_| {});
        let refs = bundle_refs(&work);
        let config_after = std::fs::read(work.join(".git").join("config")).expect("config");
        eprintln!("M3 {name}: fetch ended {outcome:?}; refs/bundles {refs:?}");
        assert!(refs.is_empty(), "{name}: the fetch wrote {refs:?}");
        assert_eq!(
            config_before, config_after,
            "{name}: the fetch wrote a byte of .git/config"
        );

        // The bite, live: without the pin the same child writes the ref.
        let control = world.work_repo(&format!("{name}-control"), uri);
        let control_intent = objects_intent(&control);
        let mut env = anonymous_env(&hooks);
        env.work_dir = Some(control.clone());
        let _ = run_stripped(&control_intent, &env, &["fetch.bundleURI="], &[]);
        let control_refs = bundle_refs(&control);
        eprintln!("M3 {name}: without fetch.bundleURI= the child wrote {control_refs:?}");
        assert!(
            !control_refs.is_empty(),
            "{name}: the fixture no longer reproduces M3, so this test proves nothing"
        );
        cases += 1;
    }
    assert_eq!(cases, 2, "both bundle shapes are exercised");
}

/// **AC-P4-47-8 (M3), the clone half.** An origin that advertises bundle URIs, under a user's
/// `transfer.bundleURI=true`: the new repository gains no `refs/bundles/*`.
///
/// Rendered from the production `write_base_args` with the transport list widened to `file` for
/// this local fixture — what `TransportFixture` does for a `SystemMutatingGit`, and the only
/// difference from the production child (the production child refuses this transport outright,
/// which would make the bundle half vacuous). The hazard is proved live the same way: without
/// `transfer.bundleURI=false` the clone writes the ref.
#[test]
fn a_bundle_advertising_origin_gives_a_clone_no_bundle_ref() {
    if !support::git_world::test_git_lists_key("transfer.bundleURI") {
        eprintln!(
            "skipped: {} has no transfer.bundleURI, so M3's clone half cannot occur",
            support::git_world::test_git_version()
        );
        return;
    }
    let world = BundleWorld::new();
    run_fixture_git(
        &world.src,
        &world.home,
        &["config", "uploadpack.advertiseBundleURIs", "true"],
    );
    run_fixture_git(&world.src, &world.home, &["config", "bundle.version", "1"]);
    run_fixture_git(&world.src, &world.home, &["config", "bundle.mode", "all"]);
    run_fixture_git(
        &world.src,
        &world.home,
        &["config", "bundle.one.uri", &world.bundle.to_string_lossy()],
    );
    let hooks = world.root.join("hooks-empty");
    std::fs::create_dir_all(&hooks).expect("hooks dir");
    let url = RemoteUrl::parse("https://forge.invalid/acme/widget.git").expect("url");
    let src_url = format!(
        "file://{}{}",
        if cfg!(windows) { "/" } else { "" },
        world.src.to_string_lossy().replace('\\', "/")
    );
    let user = support::git_world::user_global(
        &world.root,
        &format!(
            "[transfer]\n\tbundleURI = true\n[url \"{src_url}\"]\n\tinsteadOf = {}\n",
            url.as_str()
        ),
    );

    let clone_with = |dest: &Path, strip: &[&str]| {
        let intent = Intent::Clone {
            url: url.clone(),
            dest: dest.to_path_buf(),
            depth: None,
        };
        let out = run_stripped(
            &intent,
            &anonymous_env(&hooks),
            strip,
            &user
                .iter()
                .map(|(key, value)| (key.as_str(), value.clone()))
                .chain([(
                    "GIT_ALLOW_PROTOCOL",
                    OsString::from(format!("{}:file", intent.allowed_protocols())),
                )])
                .collect::<Vec<_>>(),
        );
        assert!(
            out.status.success(),
            "the fixture clone failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        bundle_refs(dest)
    };
    let pinned = clone_with(&world.root.join("pinned"), &[]);
    let unpinned = clone_with(&world.root.join("unpinned"), &["transfer.bundleURI=false"]);
    eprintln!("M3 clone: pinned {pinned:?}; without transfer.bundleURI=false {unpinned:?}");
    assert!(pinned.is_empty(), "the clone wrote {pinned:?}");
    assert!(
        !unpinned.is_empty(),
        "the fixture no longer reproduces the bundle write, so this test proves nothing"
    );
}

/// **A feature the git under test has is never skipped (R246).**
///
/// Every not-run above is keyed on a probe, so a probe wrongly answering *absent* would print a
/// skip over a live hazard. Each probe is checked against a witness that does not go through it:
/// the listing must name `fetch.prune`, which every supported git has; M3's unpinned control runs
/// whatever the listing says, and a bundle ref it writes means `fetch.bundleURI` is present; and
/// the Count route delivering the hostile profile means `GIT_CONFIG_COUNT` is read.
#[test]
fn a_feature_probe_never_reads_a_present_feature_as_absent() {
    let version = support::git_world::test_git_version();
    assert!(
        support::git_world::test_git_lists_key("fetch.prune"),
        "{version}: `git help --config` does not list fetch.prune, so every absent it answers is \
         unproven"
    );

    let world = BundleWorld::new();
    let hooks = world.root.join("hooks-empty");
    std::fs::create_dir_all(&hooks).expect("hooks dir");
    let control = world.work_repo("control", &world.bundle);
    let mut env = anonymous_env(&hooks);
    env.work_dir = Some(control.clone());
    let _ = run_stripped(&objects_intent(&control), &env, &["fetch.bundleURI="], &[]);
    let bundle_written = !bundle_refs(&control).is_empty();
    let bundle_listed = support::git_world::test_git_lists_key("fetch.bundleURI");

    let dir = tempfile::tempdir().expect("tempdir");
    let layer_c = support::git_world::World::build(dir.path(), true);
    let count_env = layer_c.apply(support::git_world::Route::Count);
    let count_delivered = layer_c.route_delivers(&count_env);
    let count_probed = support::git_world::test_git_reads_config_env();

    eprintln!(
        "{version}: fetch.bundleURI written {bundle_written}, listed {bundle_listed}; \
         GIT_CONFIG_COUNT delivered {count_delivered}, probed {count_probed}"
    );
    assert!(
        !bundle_written || bundle_listed,
        "the unpinned fetch wrote a bundle ref, yet the listing says {version} has no \
         fetch.bundleURI"
    );
    assert!(
        !count_delivered || count_probed,
        "the Count route delivered the profile, yet the probe says {version} does not read \
         GIT_CONFIG_COUNT"
    );
}

/// An event sink that drops everything: the install's stage stream is not this test's subject.
#[derive(Debug)]
struct DropEvents;

impl codotheca_core::proto::pubsub::EventSink for DropEvents {
    fn emit(&self, _topic: &str, _event: &str, _payload: serde_json::Value) {}
}

/// The case the re-executed child runs, for its printed line.
const CASE_VAR: &str = "CODOTHECA_INSTALL_REWRITE_CASE";
const INSTALL_TEST: &str =
    "an_install_rewritten_to_a_path_or_ssh_fails_network_and_runs_no_ssh_command";

/// Drive one production `run_install` in this process with the real git, and assert it ended
/// `network`.
fn child_install_ends_network() {
    use std::sync::{Arc, Mutex};

    use codotheca_core::install::queue::InstallRequest;
    use codotheca_core::install::run::{paths_for, run_install, InstallCtx, RootFacts};
    use codotheca_core::install::state::InstallStateStore;
    use codotheca_core::mount::{MountFacts, StoreClass};
    use codotheca_core::protocol::{InstallDestination, InstallRunId, ProjectId, RootId};
    use codotheca_core::testing::{FakeGitBackend, FakeMountResolver};

    let dir = support::git_world::child_dir();
    let root = dir.join("library");
    std::fs::create_dir_all(&root).expect("root");
    let hooks = dir.join("hooks-empty");
    std::fs::create_dir_all(&hooks).expect("hooks");
    let index = Arc::new(Mutex::new(
        codotheca_core::index::Index::open(&dir.join("index")).expect("index"),
    ));
    {
        let _guard = codotheca_core::proto::txguard::TxGuard::enter();
        index
            .lock()
            .expect("lock")
            .conn()
            .execute(
                "INSERT INTO project (id, name, seed_basename, created_at, updated_at)
                 VALUES (1, 'widget', 'widget', 0, 0)",
                [],
            )
            .expect("project");
    }
    let git = SystemMutatingGit::new(support::test_git(), hooks);
    let probe = FakeGitBackend::new();
    let mounts = FakeMountResolver::new();
    mounts.map(
        root.clone(),
        MountFacts {
            store_key: "store".to_owned(),
            volume_key: Some("vol".to_owned()),
            class: StoreClass::Local,
        },
    );
    let jobs = codotheca_core::jobs::NullJobSink;
    let cancel = CancelToken::new();
    let stages = InstallStateStore::new();
    let ctx = InstallCtx {
        git: &git,
        probe: &probe,
        index: &index,
        jobs: &jobs,
        mounts: &mounts,
        stages: &stages,
        events: &DropEvents,
        cancel: &cancel,
        now: 100,
    };
    let request = InstallRequest {
        project: ProjectId(1),
        root: RootId(1),
        destination: InstallDestination {
            root_id: RootId(1),
            seed_basename: "widget".to_owned(),
            display: "<root>/widget".to_owned(),
        },
    };
    let facts = RootFacts {
        root_id: 1,
        path: root.clone(),
        kind: "linux".to_owned(),
        distro: String::new(),
    };
    let paths = paths_for(&root, "widget").expect("paths");
    let outcome = run_install(
        &ctx,
        InstallRunId(1),
        &request,
        &facts,
        &paths,
        "https://forge.invalid/acme/widget.git",
    );
    eprintln!(
        "install under the {} rewrite ended {outcome:?}",
        std::env::var(CASE_VAR).unwrap_or_default()
    );
    assert_eq!(
        outcome,
        Err(codotheca_core::protocol::InstallFailure::Network),
        "a clone config rewrote off https must fail `network`"
    );
}

/// **AC-P4-47-9's install clause (§47.10's item 3, D-3).** A user whose config rewrites the https
/// clone URL to a local path, and separately to `ssh://`, cloned over that transport before —
/// against §24.1c's https-only rule. Now the install ends `network`, and the ssh program the
/// user's environment names never runs.
///
/// The rewrite is the user's global config, delivered to a re-executed child through
/// `GIT_CONFIG_GLOBAL` (which §47.3 keeps); `GIT_SSH_COMMAND` is a marker that would create a
/// file if it ran.
#[test]
fn an_install_rewritten_to_a_path_or_ssh_fails_network_and_runs_no_ssh_command() {
    if support::git_world::is_child() {
        child_install_ends_network();
        return;
    }

    let source = TestRepo::init();
    source.write("a.txt", b"one");
    source.commit("one");
    let source_path = source.path().to_string_lossy().replace('\\', "/");
    let mut cases = 0;
    for (case, target) in [
        ("path", source_path),
        ("ssh", "ssh://forge.invalid/acme/widget.git".to_owned()),
    ] {
        let tmp = tempfile::tempdir().expect("tempdir");
        let marker = tmp.path().join("ssh-ran");
        let mut env = support::git_world::user_global(
            tmp.path(),
            &format!("[url \"{target}\"]\n\tinsteadOf = https://forge.invalid/acme/widget.git\n"),
        );
        let ssh = format!("touch '{}' #", marker.to_string_lossy().replace('\\', "/"));
        env.push(("GIT_SSH_COMMAND".to_owned(), OsString::from(ssh)));
        env.push((CASE_VAR.to_owned(), OsString::from(case)));
        support::git_world::run_in_child(INSTALL_TEST, tmp.path(), &env);
        assert!(
            !marker.exists(),
            "{case}: the ssh program the environment names ran"
        );
        assert!(
            !tmp.path().join("library").join("widget").exists(),
            "{case}: a destination exists after a refused clone"
        );
        cases += 1;
    }
    assert_eq!(cases, 2, "both rewrites are exercised");
}

// ---------------------------------------------------------------------------
// §47.4 — the verifying read that replaces the retired fetch.
// ---------------------------------------------------------------------------

/// A bare origin and a work clone of it, under one temporary root the transport fixture admits.
struct OriginWorld {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    origin: PathBuf,
    work: PathBuf,
}

impl OriginWorld {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = support::canonical(dir.path());
        let home = root.join("home");
        std::fs::create_dir_all(&home).expect("home");
        let seed = root.join("seed");
        run_fixture_git(
            &root,
            &home,
            &["init", "-q", "-b", "main", &seed.to_string_lossy()],
        );
        std::fs::write(seed.join("a.txt"), b"one\n").expect("seed file");
        run_fixture_git(&seed, &home, &["add", "a.txt"]);
        run_fixture_git(&seed, &home, &["commit", "-q", "-m", "one"]);
        let origin = root.join("origin.git");
        run_fixture_git(
            &root,
            &home,
            &[
                "clone",
                "-q",
                "--bare",
                &seed.to_string_lossy(),
                &origin.to_string_lossy(),
            ],
        );
        let work = root.join("work");
        run_fixture_git(
            &root,
            &home,
            &[
                "clone",
                "-q",
                &origin.to_string_lossy(),
                &work.to_string_lossy(),
            ],
        );
        Self {
            _dir: dir,
            root,
            home,
            origin,
            work,
        }
    }

    /// `git <args>` in `repo`, printing nothing and returning stdout.
    fn git(&self, repo: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(repo)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .args(args)
            .output()
            .expect("fixture git");
        assert!(
            out.status.success(),
            "fixture git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn handle(repo: &Path) -> codotheca_core::git::RepoHandle {
        codotheca_core::git::RepoHandle::resolve(
            repo,
            codotheca_core::git::StoreKey::new("s"),
            codotheca_core::mount::StoreClass::Local,
        )
        .expect("resolves")
    }

    fn read_git(&self) -> codotheca_core::git::SystemGit {
        let hooks = codotheca_core::git::ensure_empty_hooks_dir(&self.root).expect("hooks");
        codotheca_core::git::SystemGit::new(
            std::sync::Arc::new(codotheca_core::git::GitExec::new(
                support::test_git(),
                hooks,
            )),
            std::sync::Arc::new(codotheca_core::git::GitSlots::for_machine()),
            std::sync::Arc::new(codotheca_core::clock::SystemClock::new()),
        )
    }

    fn write_git(&self) -> SystemMutatingGit {
        let hooks = codotheca_core::git::ensure_empty_hooks_dir(&self.root).expect("hooks");
        SystemMutatingGit::with_transport_fixture(
            support::test_git(),
            hooks,
            codotheca_core::gitw::TransportFixture::new(&self.root),
        )
    }

    /// Every ref, through git, with its object — the comparator's ref half.
    fn refs(&self, repo: &Path) -> String {
        self.git(repo, &["for-each-ref", "--format=%(refname) %(objectname)"])
    }
}

const fn ctx(cancel: &CancelToken) -> codotheca_core::git::JobContext<'_> {
    codotheca_core::git::JobContext::new(codotheca_core::git::JobClass::Interactive, cancel, None)
}

/// An index holding `repo` as one observed location under a scan root, its row carrying the
/// scan's lineage: no local refusal stops the analysis before step 6.
fn seeded_index(
    repo: &TestRepo,
    dir: &Path,
) -> (
    std::sync::Arc<std::sync::Mutex<codotheca_core::index::Index>>,
    i64,
) {
    let index = std::sync::Arc::new(std::sync::Mutex::new(
        codotheca_core::index::Index::open_at(dir, 1_750_000_000).expect("index"),
    ));
    // The row's lineage is the scan's, so §45.6 step 1 matches and the remotes are read.
    let lineage =
        support::git_world::scan_lineage(&support::git_world::system_git(repo), repo.path());
    let location = {
        let mut guard = index.lock().expect("index");
        guard
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
                     VALUES ('widget', 'widget', ?1, 0, 0)",
                    [&lineage],
                )?;
                let project = tx.last_insert_rowid();
                // Observed, and under a scan root: no local refusal stops the analysis before
                // step 6, whose order this test reads.
                let root = repo.path().parent().expect("the repository's parent");
                tx.execute(
                    "INSERT INTO scan_root (kind, distro, path_bytes, path_key, path_display,
                                            added_by, added_at)
                     VALUES ('linux', '', ?1, ?1, ?2, 'user', 0)",
                    rusqlite::params![root.to_string_lossy().as_bytes(), root.to_string_lossy()],
                )?;
                tx.execute(
                    "INSERT INTO location
                       (project_id, kind, distro, path_bytes, path_key, path_display,
                        store_key, presence, repo_kind, refstate_observed_at,
                        worktree_observed_at)
                     VALUES (?1, 'linux', '', ?2, ?2, ?3, 'store', 'present', 'worktree', 1, 1)",
                    rusqlite::params![
                        project,
                        repo.path().to_string_lossy().as_bytes(),
                        repo.path().to_string_lossy()
                    ],
                )?;
                Ok(tx.last_insert_rowid())
            })
            .expect("seeded")
    };
    (index, location)
}

/// **M5 (§47.2 rule 2): stdin is argv by another channel.** A destination on a stdin line wrote
/// a ref under every other pin. The grammar refuses the line — and fed to a real child carrying
/// every objects-step pin, the same line does create the ref, so the grammar is what stops it.
#[test]
fn a_destination_refspec_on_stdin_is_refused_by_the_grammar_and_would_write_a_ref() {
    use std::io::Write as _;

    let line = "refs/heads/main:refs/heads/injected";
    assert_eq!(
        AdvertisedRef::parse(line),
        Err(codotheca_core::gitw::IntentRefusal::UnsafeRefName),
        "the grammar must refuse a destination"
    );

    let world = OriginWorld::new();
    let intent = objects_intent(&world.work);
    let hooks = codotheca_core::git::ensure_empty_hooks_dir(&world.root).expect("hooks");
    let mut cmd = Command::new(support::test_git());
    cmd.args(write_base_args(&intent, &anonymous_env(&hooks)));
    cmd.args(intent.argv());
    neutralise_env(&mut cmd);
    for (key, value) in intent.env_pins(&hooks) {
        cmd.env(key, value);
    }
    cmd.env("GIT_ALLOW_PROTOCOL", "https:ssh:file");
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("child");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(format!("{line}\n").as_bytes())
        .expect("stdin written");
    let out = child.wait_with_output().expect("child ends");
    let refs = world.refs(&world.work);
    eprintln!(
        "M5: a raw stdin destination under every pin -> exit {:?}; refs {:?}",
        out.status.code(),
        refs.lines().collect::<Vec<_>>()
    );
    assert!(
        refs.contains("refs/heads/injected"),
        "the fixture no longer shows M5, so the grammar is proving nothing: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// **AC-P4-47-22 — the read's scope.** An origin advertises 1,000 absent review refs and one
/// absent branch tip: the objects step carries exactly that branch, and no review ref's object
/// arrives. Through the production verifier, with the transport fixture.
#[test]
fn the_objects_step_carries_only_branch_and_tag_tips() {
    use codotheca_core::analyser::remote::{GitRemoteVerifier, RemoteReading, RemoteVerifier as _};

    let world = OriginWorld::new();
    // An absent branch tip on origin.
    let side = world.root.join("side");
    world.git(
        &world.root,
        &[
            "clone",
            "-q",
            &world.origin.to_string_lossy(),
            &side.to_string_lossy(),
        ],
    );
    world.git(&side, &["checkout", "-q", "-b", "extra"]);
    std::fs::write(side.join("b.txt"), b"extra\n").expect("file");
    world.git(&side, &["add", "b.txt"]);
    world.git(&side, &["commit", "-q", "-m", "extra"]);
    world.git(&side, &["push", "-q", "origin", "extra"]);
    let extra = world.git(&side, &["rev-parse", "HEAD"]).trim().to_owned();
    // One orphan commit, absent everywhere but origin, behind 1,000 review refs.
    world.git(&side, &["checkout", "-q", "--orphan", "review"]);
    std::fs::write(side.join("c.txt"), b"review\n").expect("file");
    world.git(&side, &["add", "c.txt"]);
    world.git(&side, &["commit", "-q", "-m", "review"]);
    let review = world.git(&side, &["rev-parse", "HEAD"]).trim().to_owned();
    world.git(
        &side,
        &["push", "-q", "origin", "HEAD:refs/heads/review-tmp"],
    );
    let mut updates = String::new();
    for n in 0..1_000 {
        let _ = writeln!(updates, "create refs/changes/{n:02}/{n}/1 {review}");
    }
    updates.push_str("delete refs/heads/review-tmp\n");
    let mut child = Command::new("git")
        .current_dir(&world.origin)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["update-ref", "--stdin"])
        .stdin(Stdio::piped())
        .spawn()
        .expect("update-ref");
    {
        use std::io::Write as _;
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(updates.as_bytes())
            .expect("updates");
    }
    assert!(child.wait().expect("update-ref ends").success());

    let read_git = world.read_git();
    let write_git = world.write_git();
    let verifier = GitRemoteVerifier::with_transport_fixture(
        &write_git,
        &read_git,
        codotheca_core::gitw::TransportFixture::new(&world.root),
    );
    let cancel = CancelToken::new();
    let reading = verifier.read(&OriginWorld::handle(&world.work), "origin", &ctx(&cancel));
    let RemoteReading::Answered { present, fetched } = &reading else {
        panic!("the origin must answer: {reading:?}");
    };
    eprintln!(
        "AC-P4-47-22: objects-step stdin {fetched:?}; {} advertised objects present after the read",
        present.len()
    );
    assert_eq!(fetched, &vec!["refs/heads/extra".to_owned()]);
    let has = |oid: &str| {
        Command::new("git")
            .current_dir(&world.work)
            .args(["cat-file", "-e", oid])
            .status()
            .expect("cat-file")
            .success()
    };
    assert!(has(&extra), "the branch tip's object arrived");
    assert!(!has(&review), "no review ref's object may arrive");
    assert!(present.contains(&extra) && !present.contains(&review));
}

/// **Every configured remote is read, one at a time, in config order** — through the pre-flight
/// handler, with the write path pointed at the recording stand-in so the order of its children
/// can be read back. The shipped handler read `origin` only.
#[test]
fn every_configured_remote_is_read_one_at_a_time() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("one");
    repo.git(&[
        "remote",
        "add",
        "first",
        "https://forge.example/acme/first.git",
    ]);
    repo.git(&[
        "remote",
        "add",
        "second",
        "https://forge.example/acme/second.git",
    ]);

    let dir = tempfile::tempdir().expect("tempdir");
    let (index, location) = seeded_index(&repo, dir.path());
    let hooks = codotheca_core::git::ensure_empty_hooks_dir(dir.path()).expect("hooks");
    let write_git = SystemMutatingGit::new(recording_git(), hooks);
    let read_git = support::git_world::system_git(&repo);
    let remotes = codotheca_core::analyser::remote::GitRemoteVerifier::new(&write_git, &read_git);
    let trash = codotheca_core::testing::CountingTrash::new();
    let verdict = codotheca_core::uninstall::handle_preflight_off_lock(
        &index,
        &codotheca_core::analyser::AnalyserSeams {
            git: &read_git,
            remotes: &remotes,
            trash: &trash,
            before_act: None,
        },
        serde_json::json!({ "locationId": location }),
        1_750_000_000,
    )
    .expect("the pre-flight answers");

    let calls = support::git_world::read_recordings(repo.path());
    let order: Vec<(String, String)> = calls
        .iter()
        .map(|call| {
            let verb = call
                .argv
                .iter()
                .find(|a| *a == "ls-remote" || *a == "fetch")
                .cloned()
                .unwrap_or_default();
            let step = if call.argv.iter().any(|a| a == "--get-url") {
                format!("{verb} --get-url")
            } else {
                verb
            };
            (step, call.argv.last().cloned().unwrap_or_default())
        })
        .collect();
    eprintln!(
        "remotes read, in order: {order:?}; verdict {}",
        verdict["disposition"]
    );
    let names: Vec<&str> = order.iter().map(|(_, name)| name.as_str()).collect();
    let first_last = names.iter().rposition(|n| *n == "first");
    let second_first = names.iter().position(|n| *n == "second");
    assert!(
        names.contains(&"first") && names.contains(&"second"),
        "both configured remotes must be read: {order:?}"
    );
    assert!(
        first_last < second_first,
        "one at a time, in config order: {order:?}"
    );
    assert_eq!(
        order.first().map(|(step, _)| step.as_str()),
        Some("ls-remote --get-url"),
        "each read starts by resolving, contacting nothing"
    );
}

/// **M2's config, and the verifying read writes no ref.** A config refspec
/// `+refs/heads/*:refs/heads/*` with `fetch.prune` and `pruneTags`, over a checked-out
/// local-only branch and a local-only tag: every ref and `HEAD` are byte-identical after the
/// read, which did fetch. The retired argv, run as a literal on a twin, deletes the branch.
#[test]
fn a_verify_read_writes_no_ref_under_a_pruning_refspec() {
    use codotheca_core::analyser::remote::{GitRemoteVerifier, RemoteReading, RemoteVerifier as _};

    let world = OriginWorld::new();
    // Origin moves on, so the read has objects to fetch.
    let side = world.root.join("side");
    world.git(
        &world.root,
        &[
            "clone",
            "-q",
            &world.origin.to_string_lossy(),
            &side.to_string_lossy(),
        ],
    );
    std::fs::write(side.join("a.txt"), b"two\n").expect("file");
    world.git(&side, &["commit", "-q", "-am", "two"]);
    world.git(&side, &["push", "-q", "origin", "main"]);

    let dress = |repo: &Path| {
        world.git(repo, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(repo.join("l.txt"), b"local only\n").expect("file");
        world.git(repo, &["add", "l.txt"]);
        world.git(repo, &["commit", "-q", "-m", "local only"]);
        world.git(repo, &["tag", "-a", "local-tag", "-m", "local"]);
        world.git(
            repo,
            &[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/heads/*",
            ],
        );
        world.git(repo, &["config", "fetch.prune", "true"]);
        world.git(repo, &["config", "fetch.pruneTags", "true"]);
    };
    dress(&world.work);
    let refs_before = world.refs(&world.work);
    let head_before = std::fs::read(world.work.join(".git/HEAD")).expect("HEAD");

    let read_git = world.read_git();
    let write_git = world.write_git();
    let verifier = GitRemoteVerifier::with_transport_fixture(
        &write_git,
        &read_git,
        codotheca_core::gitw::TransportFixture::new(&world.root),
    );
    let cancel = CancelToken::new();
    let reading = verifier.read(&OriginWorld::handle(&world.work), "origin", &ctx(&cancel));
    let refs_after = world.refs(&world.work);
    let head_after = std::fs::read(world.work.join(".git/HEAD")).expect("HEAD");
    eprintln!(
        "M2 config: the verifying read ended {reading:?}; {} refs compared",
        refs_before.lines().count()
    );
    assert!(
        matches!(&reading, RemoteReading::Answered { fetched, .. } if !fetched.is_empty()),
        "the read must answer and fetch: {reading:?}"
    );
    assert_eq!(refs_before, refs_after, "the verifying read moved a ref");
    assert_eq!(head_before, head_after, "the verifying read moved HEAD");
    assert!(
        !world.work.join(".git/FETCH_HEAD").exists(),
        "no FETCH_HEAD"
    );

    // The bite: the retired argv, as a literal, on a twin dressed the same way.
    let twin = world.root.join("twin");
    world.git(
        &world.root,
        &[
            "clone",
            "-q",
            &world.origin.to_string_lossy(),
            &twin.to_string_lossy(),
        ],
    );
    world.git(&twin, &["reset", "-q", "--hard", "HEAD~1"]);
    dress(&twin);
    let _ = Command::new("git")
        .current_dir(&twin)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["fetch", "--progress", "origin"])
        .output()
        .expect("the retired fetch");
    let twin_refs = world.refs(&twin);
    eprintln!("M2 config: the retired `fetch --progress origin` left {twin_refs:?}");
    assert!(
        !twin_refs.contains("refs/heads/feature") || !twin_refs.contains("refs/tags/local-tag"),
        "the fixture no longer reproduces M2, so this test proves nothing"
    );
}

// ---------------------------------------------------------------------------
// §47.9 C — layer C: every intent's effect under the hostile profile, by four routes.
// ---------------------------------------------------------------------------

const LAYER_C_TEST: &str = "ac_p4_47_6";
const ROUTE_VAR: &str = "CODOTHECA_LAYER_C_ROUTE";

/// The intents layer C drives against a world: a clone to a destination that does not exist,
/// and each step of the verifying read of `origin` in the work repository.
fn layer_c_intents(world: &support::git_world::World, tag: &str) -> Vec<Intent> {
    let tips = vec![
        AdvertisedRef::parse("refs/heads/extra").expect("tip"),
        AdvertisedRef::parse("refs/heads/main").expect("tip"),
    ];
    let mut intents = vec![Intent::Clone {
        url: RemoteUrl::parse(&world.clone_url).expect("url"),
        dest: world.root.join(format!("clone-{tag}")),
        depth: None,
    }];
    for step in [
        VerifyStep::ResolveUrl,
        VerifyStep::Advertise,
        VerifyStep::Objects { tips },
    ] {
        intents.push(Intent::VerifyRead {
            repo: world.work.clone(),
            remote: RemoteName::parse("origin").expect("remote"),
            step,
        });
    }
    intents
}

/// The production write path with the one test-only difference, over `world`.
fn fixture_write_git(world: &support::git_world::World) -> SystemMutatingGit {
    let hooks =
        codotheca_core::git::ensure_empty_hooks_dir(&world.root.join("app-data")).expect("hooks");
    SystemMutatingGit::with_transport_fixture(
        support::test_git(),
        hooks,
        codotheca_core::gitw::TransportFixture::new(&world.root),
    )
}

/// `PATH` with the world's marker helper directory first, so a helper git reached would run.
fn path_with_helpers(world: &support::git_world::World) -> OsString {
    let mut dirs = vec![world.helper_dir.clone()];
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    std::env::join_paths(dirs).expect("PATH")
}

/// **AC-P4-47-6 — layer C.** Every (kind, step) × four routes × the hostile parent environment
/// writes **exactly** what `effect()` declares: every ref, reflog, `HEAD`, `FETCH_HEAD`,
/// `packed-refs`, `.git/config`, index and worktree file of the work repository, the origin and
/// the decoy compared through git and by hash; the object set never shrinks; no marker program
/// runs and no trace file is written.
///
/// Each route runs in a re-executed child carrying its environment — `GIT_CONFIG_GLOBAL`,
/// `GIT_CONFIG_COUNT` and the hostile parent's scrub variables — so the product inherits them
/// as it would from a user's shell. `HEAD` is detached for two of the four routes (D10's
/// detached variant).
#[test]
fn ac_p4_47_6() {
    use support::git_world::{child_dir, effect_violations, is_child, run_in_child, Route, World};

    if is_child() {
        let world = World::at(&child_dir());
        let route = std::env::var(ROUTE_VAR).expect("route");
        let trace = world.root.join("trace.txt");
        let write_git = fixture_write_git(&world);
        let mut refs = 0;
        let mut files = 0;
        let mut objects = 0;
        for intent in layer_c_intents(&world, &route) {
            let before = world.snapshot(&trace);
            let outcome = write_git.run(&intent, &CancelToken::new(), &mut |_| {});
            let after = world.snapshot(&trace);
            let violations = effect_violations(intent.effect(), &before, &after);
            eprintln!(
                "layer C [{route}] {:?} {:?}: {} — {} refs lines, {} files hashed, {} objects \
                 before, {} after, markers {:?}",
                intent.kind(),
                intent.step(),
                if outcome.is_ok() {
                    "ok"
                } else {
                    "refused or failed"
                },
                before
                    .refs
                    .values()
                    .map(|r| r.lines().count())
                    .sum::<usize>(),
                before.files.len(),
                before.objects.len(),
                after.objects.len(),
                after.markers
            );
            assert!(
                violations.is_empty(),
                "[{route}] {:?} {:?} wrote beyond {:?}: {violations:?}",
                intent.kind(),
                intent.step(),
                intent.effect()
            );
            refs += before
                .refs
                .values()
                .map(|r| r.lines().count())
                .sum::<usize>();
            files += before.files.len();
            objects += after.objects.len();
        }
        eprintln!(
            "layer C [{route}]: {} intents, {refs} refs compared, {files} files hashed, \
             {objects} objects counted, markers checked",
            layer_c_intents(&world, &route).len()
        );
        return;
    }

    let version = support::git_world::test_git_version();
    let mut routes = 0;
    let mut skipped = 0;
    for route in Route::ALL {
        let dir = tempfile::tempdir().expect("tempdir");
        let detached = matches!(route, Route::Include | Route::Count);
        let world = World::build(dir.path(), detached);
        let mut env = world.apply(route);
        if !world.route_delivers(&env) {
            assert!(
                route == Route::Count && !support::git_world::test_git_reads_config_env(),
                "the {} route did not deliver the hostile profile to {version}",
                route.slug()
            );
            eprintln!(
                "layer C [count]: skipped: {version} does not read GIT_CONFIG_COUNT, so the \
                 route cannot occur"
            );
            skipped += 1;
            continue;
        }
        let hostile = support::git_world::hostile_parent_env(&world.root);
        // The route's own GIT_CONFIG_* win over the hostile parent's, which the product scrubs
        // anyway; the rest of the hostile parent rides along.
        for (key, value) in hostile.scrubbed {
            if !env.iter().any(|(k, _)| *k == key) {
                env.push((key, value));
            }
        }
        env.push((ROUTE_VAR.to_owned(), OsString::from(route.slug())));
        env.push(("PATH".to_owned(), path_with_helpers(&world)));
        run_in_child(LAYER_C_TEST, &world.root, &env);
        routes += 1;
    }
    eprintln!(
        "layer C on {version}: {} kinds x {routes} routes ({} intents each) matched their \
         declared effect; {skipped} route(s) not run",
        Intent::ALL.len(),
        1 + VerifyStep::ALL.len()
    );
    assert!(routes > 0);
    assert_eq!(routes + skipped, Route::ALL.len());
}

/// **AC-P4-47-6's second clause.** The `TransportFixture` is the **only** difference between the
/// child a test runs and the child production runs: same argv, same environment, except that
/// `GIT_ALLOW_PROTOCOL` gains `:file`, rendered last.
#[test]
fn the_transport_fixture_is_the_only_test_to_production_difference() {
    let prod_dir = tempfile::tempdir().expect("tempdir");
    let test_dir = tempfile::tempdir().expect("tempdir");
    let token = || codotheca_core::accounts::keychain::SecretToken::new("unused".to_owned());
    let prod_fixture = AuditFixture::new(prod_dir.path(), token()).expect("fixture");
    let test_fixture = AuditFixture::new(test_dir.path(), token()).expect("fixture");
    let hooks = prod_dir.path().join("hooks-empty");
    std::fs::create_dir_all(&hooks).expect("hooks");
    let env = anonymous_env(&hooks);
    let production = codotheca_core::gitw::WriteExec::new(recording_git());
    let with_fixture = codotheca_core::gitw::WriteExec::with_transport_fixture(
        recording_git(),
        codotheca_core::gitw::TransportFixture::new(test_dir.path()),
    );
    let prod_intents = Intent::all_for_audit(&prod_fixture);
    let test_intents = Intent::all_for_audit(&test_fixture);
    for (p, t) in prod_intents.iter().zip(&test_intents) {
        production
            .run(p, &env, &CancelToken::new(), &mut |_| {})
            .expect("stand-in");
        with_fixture
            .run(t, &env, &CancelToken::new(), &mut |_| {})
            .expect("stand-in");
    }
    let prod_calls = support::git_world::read_recordings(prod_fixture.dest());
    let test_calls = support::git_world::read_recordings(test_fixture.dest());
    assert_eq!(prod_calls.len(), prod_intents.len());
    assert_eq!(test_calls.len(), test_intents.len());
    // The two fixtures live in two tempdirs; each child is keyed on its own.
    let normal = |text: &String| {
        text.replace(&*test_dir.path().to_string_lossy(), "<root>")
            .replace(&*prod_dir.path().to_string_lossy(), "<root>")
    };
    let mut compared = 0;
    for ((intent, prod), test) in prod_intents.iter().zip(&prod_calls).zip(&test_calls) {
        let prod_argv: Vec<String> = prod.argv.iter().map(normal).collect();
        let test_argv: Vec<String> = test.argv.iter().map(normal).collect();
        assert_eq!(
            prod_argv,
            test_argv,
            "{:?} {:?}: argv differs",
            intent.kind(),
            intent.step()
        );
        let prod_env: BTreeSet<String> = prod.env.iter().map(normal).collect();
        let test_env: BTreeSet<String> = test.env.iter().map(normal).collect();
        let only_prod: Vec<&String> = prod_env.difference(&test_env).collect();
        let only_test: Vec<&String> = test_env.difference(&prod_env).collect();
        let allowed = intent.allowed_protocols();
        assert_eq!(
            (only_prod, only_test),
            (
                vec![&format!("GIT_ALLOW_PROTOCOL={allowed}")],
                vec![&format!("GIT_ALLOW_PROTOCOL={allowed}:file")]
            ),
            "{:?} {:?}: the fixture must change exactly GIT_ALLOW_PROTOCOL",
            intent.kind(),
            intent.step()
        );
        compared += 1;
    }
    eprintln!("transport fixture: {compared} children compared; the only difference is `:file`");
    assert!(compared > 0);
}

const M2_TEST: &str = "ac_p4_47_7";

/// **AC-P4-47-7 — the §37.8 defect, proved visible.** The differential harness, driven with the
/// retired argv `fetch --progress origin` (a literal here: the variant is gone), reports M2's
/// branch and tag deletions under the environment route and under repository config; the same
/// harness passes the verifying read.
#[test]
fn ac_p4_47_7() {
    use support::git_world::{child_dir, effect_violations, is_child, run_in_child, Route, World};

    if is_child() {
        let world = World::at(&child_dir());
        let trace = world.root.join("trace.txt");
        let objects = Intent::VerifyRead {
            repo: world.work.clone(),
            remote: RemoteName::parse("origin").expect("remote"),
            step: VerifyStep::Objects {
                tips: vec![AdvertisedRef::parse("refs/heads/main").expect("tip")],
            },
        };
        let before = world.snapshot(&trace);
        let _ = fixture_write_git(&world).run(&objects, &CancelToken::new(), &mut |_| {});
        let after = world.snapshot(&trace);
        let violations = effect_violations(objects.effect(), &before, &after);
        eprintln!("M2 harness, verifying read: {violations:?}");
        assert!(
            violations.is_empty(),
            "the verifying read failed M2's harness: {violations:?}"
        );
        return;
    }

    let mut reported = 0;
    let mut skipped = 0;
    for route in [Route::Count, Route::RepoConfig] {
        // The literal, on its own world, behind the uniform `-c` pins and with the route's config
        // unscrubbed — as it shipped, when nothing removed `GIT_CONFIG_COUNT`.
        let dir = tempfile::tempdir().expect("tempdir");
        let world = World::build(dir.path(), false);
        let env = world.apply(route);
        if !world.route_delivers(&env) {
            assert!(
                route == Route::Count && !support::git_world::test_git_reads_config_env(),
                "the {} route did not deliver the hostile profile",
                route.slug()
            );
            eprintln!(
                "M2 [count]: skipped: {} does not read GIT_CONFIG_COUNT, so the env route \
                 cannot occur",
                support::git_world::test_git_version()
            );
            skipped += 1;
            continue;
        }
        let trace = world.root.join("trace.txt");
        let hooks = world.root.join("pins-hooks");
        std::fs::create_dir_all(&hooks).expect("hooks");
        let pins_of = Intent::Clone {
            url: RemoteUrl::parse(&world.clone_url).expect("url"),
            dest: world.root.join("unused"),
            depth: None,
        };
        let before = world.snapshot(&trace);
        let mut literal = Command::new(support::test_git());
        literal
            .current_dir(&world.work)
            .args(write_base_args(&pins_of, &anonymous_env(&hooks)))
            .args(["fetch", "--progress", "origin"])
            .env("GIT_CONFIG_NOSYSTEM", "1");
        for (key, value) in &env {
            literal.env(key, value);
        }
        let fetched = literal.output().expect("the retired fetch");
        eprintln!(
            "M2 [{}] exit {:?}: {}",
            route.slug(),
            fetched.status.code(),
            String::from_utf8_lossy(&fetched.stderr).trim()
        );
        let after = world.snapshot(&trace);
        let violations = effect_violations(
            codotheca_core::gitw::DeclaredEffect::ObjectsOnly,
            &before,
            &after,
        );
        let refs_line = violations
            .iter()
            .find(|v| v.starts_with("work: refs moved"))
            .cloned()
            .unwrap_or_default();
        eprintln!(
            "M2 harness [{}], `fetch --progress origin`: {refs_line}",
            route.slug()
        );
        assert!(
            refs_line.contains("refs/heads/feature") && refs_line.contains("refs/tags/local-tag"),
            "the harness must report the retired fetch deleting the local-only branch and tag \
             under the {} route: {violations:?}",
            route.slug()
        );
        reported += 1;

        // The verifying read, on a fresh world under the same route, in a child carrying it.
        let verify_dir = tempfile::tempdir().expect("tempdir");
        let verify_world = World::build(verify_dir.path(), false);
        let mut verify_env = verify_world.apply(route);
        verify_env.push(("PATH".to_owned(), path_with_helpers(&verify_world)));
        run_in_child(M2_TEST, &verify_world.root, &verify_env);
    }
    assert!(reported > 0);
    assert_eq!(reported + skipped, 2);
}

const PINS_TEST: &str = "ac_p4_47_9";

/// The production write path, recording each step it was asked to spawn.
#[derive(Debug)]
struct CountingWrite {
    inner: SystemMutatingGit,
    steps: std::sync::Mutex<Vec<String>>,
}

impl MutatingGit for CountingWrite {
    fn run(
        &self,
        intent: &Intent,
        cancel: &CancelToken,
        on_stderr: &mut dyn FnMut(&str),
    ) -> codotheca_core::git::GitResult<codotheca_core::gitw::RunOutput> {
        self.steps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("{:?}", intent.step()));
        self.inner.run(intent, cancel, on_stderr)
    }
}

/// M4's bite, live: the helper remote's `Advertise` child rendered without
/// `-c protocol.file.allow=never` and without `GIT_ALLOW_PROTOCOL`, under the user's global
/// config (`user`, from [`support::git_world::user_global`]). True when the marker helper ran.
fn helper_runs_without_the_transport_pins(
    world: &support::git_world::World,
    user: &[(String, OsString)],
) -> bool {
    let hooks = world.root.join("bite-hooks");
    std::fs::create_dir_all(&hooks).expect("hooks");
    let intent = Intent::VerifyRead {
        repo: world.work.clone(),
        remote: RemoteName::parse("helper").expect("remote"),
        step: VerifyStep::Advertise,
    };
    let mut env_no_pins = anonymous_env(&hooks);
    env_no_pins.work_dir = Some(world.work.clone());
    let base = write_base_args(&intent, &env_no_pins);
    let mut stripped: Vec<OsString> = Vec::new();
    let mut i = 0;
    while i < base.len() {
        if base[i] == "-c"
            && base
                .get(i + 1)
                .is_some_and(|v| v == "protocol.file.allow=never")
        {
            i += 2;
            continue;
        }
        stripped.push(base[i].clone());
        i += 1;
    }
    let mut cmd = Command::new(support::test_git());
    cmd.args(stripped).args(intent.argv());
    neutralise_env(&mut cmd);
    cmd.envs(user.iter().map(|(key, value)| (key, value)))
        .env("PATH", path_with_helpers(world))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _ = cmd.status().expect("the unpinned child");
    world.markers.join("helper").exists()
}

/// **AC-P4-47-9 — transports, with production pins and no `TransportFixture`.** A marker
/// transport helper the user allowed never runs; a user-global `protocol.file.allow=always`
/// opens no file remote; an https remote the user's `insteadOf` rewrites to a path is classified
/// same-machine and never advertised. And, proved live in the same run: the same helper child
/// rendered without `-c protocol.file.allow=never` and without `GIT_ALLOW_PROTOCOL` runs it.
#[test]
fn ac_p4_47_9() {
    use codotheca_core::analyser::remote::{GitRemoteVerifier, RemoteReading, RemoteVerifier as _};
    use support::git_world::{child_dir, is_child, run_in_child, World};

    if is_child() {
        let world = World::at(&child_dir());
        let hooks = codotheca_core::git::ensure_empty_hooks_dir(&world.root.join("app-data"))
            .expect("hooks");
        let production = SystemMutatingGit::new(support::test_git(), hooks.clone());
        let advertise = |remote: &str| Intent::VerifyRead {
            repo: world.work.clone(),
            remote: RemoteName::parse(remote).expect("remote"),
            step: VerifyStep::Advertise,
        };
        let helper = production.run(&advertise("helper"), &CancelToken::new(), &mut |_| {});
        let file = production.run(&advertise("origin"), &CancelToken::new(), &mut |_| {});
        let read_git = codotheca_core::git::SystemGit::new(
            std::sync::Arc::new(codotheca_core::git::GitExec::new(
                support::test_git(),
                hooks,
            )),
            std::sync::Arc::new(codotheca_core::git::GitSlots::for_machine()),
            std::sync::Arc::new(codotheca_core::clock::SystemClock::new()),
        );
        let counting = CountingWrite {
            inner: production,
            steps: std::sync::Mutex::new(Vec::new()),
        };
        let verifier = GitRemoteVerifier::new(&counting, &read_git);
        let cancel = CancelToken::new();
        let handle = OriginWorld::handle(&world.work);
        let rewritten = verifier.read(&handle, "rewritten", &ctx(&cancel));
        let spawned = counting.steps.lock().expect("steps").clone();
        assert_eq!(
            spawned,
            vec![format!(
                "{:?}",
                Some(codotheca_core::gitw::VerifyStepKind::ResolveUrl)
            )],
            "a same-machine remote is classified after ResolveUrl and never advertised"
        );
        let markers: Vec<String> = std::fs::read_dir(&world.markers)
            .expect("markers")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        eprintln!(
            "production pins: helper remote {helper:?}; file remote {file:?}; https->path \
             rewrite {rewritten:?}; markers {markers:?}"
        );
        assert!(
            helper.is_err(),
            "a user-allowed helper transport must be refused"
        );
        assert!(
            matches!(
                file,
                Err(codotheca_core::git::GitError::TransportRefused { .. })
            ),
            "a file remote must be refused even under protocol.file.allow=always: {file:?}"
        );
        assert_eq!(rewritten, RemoteReading::SameMachine);
        assert!(markers.is_empty(), "a marker program ran: {markers:?}");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let world = World::build(dir.path(), false);
    world.git(&world.work, &["remote", "add", "helper", "codotheca::x"]);
    world.git(
        &world.work,
        &[
            "remote",
            "add",
            "rewritten",
            "https://forge.invalid/acme/rewritten.git",
        ],
    );
    let user = support::git_world::user_global(
        &world.root,
        &format!(
            "[protocol \"file\"]\n\tallow = always\n[protocol \"codotheca\"]\n\tallow = always\n\
             [url \"{}\"]\n\tinsteadOf = https://forge.invalid/acme/rewritten.git\n",
            world.origin.to_string_lossy().replace('\\', "/")
        ),
    );
    let mut env = user.clone();
    env.push(("PATH".to_owned(), path_with_helpers(&world)));
    run_in_child(PINS_TEST, &world.root, &env);

    // The bite, live: the helper child without the two transport pins runs the helper.
    let ran = helper_runs_without_the_transport_pins(&world, &user);
    eprintln!("production pins: without the transport pins the helper ran: {ran}");
    assert!(
        ran,
        "the fixture no longer shows M4, so the pins are proving nothing"
    );
}

/// **AC-P4-47-12's Lane-0 clause.** A config key git lists in an audited namespace and nobody
/// has classified does not stop the verifying read or a clone: only `TagArchived`, the one write
/// into an existing repository, refuses on an unknown key (§47.7).
#[test]
fn ac_p4_47_12() {
    use codotheca_core::analyser::remote::{GitRemoteVerifier, RemoteReading, RemoteVerifier as _};
    use support::git_world::World;

    let dir = tempfile::tempdir().expect("tempdir");
    let world = World::build(dir.path(), false);
    world.git(
        &world.work,
        &["config", "fetch.inventedByANewerGit", "true"],
    );
    world.git(&world.work, &["config", "remote.origin.somethingNew", "1"]);
    let write_git = fixture_write_git(&world);
    let read_git = codotheca_core::git::SystemGit::new(
        std::sync::Arc::new(codotheca_core::git::GitExec::new(
            support::test_git(),
            codotheca_core::git::ensure_empty_hooks_dir(&world.root.join("read-hooks"))
                .expect("hooks"),
        )),
        std::sync::Arc::new(codotheca_core::git::GitSlots::for_machine()),
        std::sync::Arc::new(codotheca_core::clock::SystemClock::new()),
    );
    let verifier = GitRemoteVerifier::with_transport_fixture(
        &write_git,
        &read_git,
        codotheca_core::gitw::TransportFixture::new(&world.root),
    );
    let cancel = CancelToken::new();
    let handle = OriginWorld::handle(&world.work);
    let reading = verifier.read(&handle, "origin", &ctx(&cancel));
    eprintln!("unknown audited keys: the verifying read {reading:?}");
    assert!(matches!(reading, RemoteReading::Answered { .. }));

    // The clone reads the same unknown key from the user's global config.
    let user = support::git_world::user_global(
        &world.root,
        &format!(
            "[clone]\n\tinventedByANewerGit = true\n[url \"{}\"]\n\tinsteadOf = {}\n",
            world.origin.to_string_lossy().replace('\\', "/"),
            world.clone_url
        ),
    );
    let dest = world.root.join("clone-unknown");
    let intent = Intent::Clone {
        url: RemoteUrl::parse(&world.clone_url).expect("url"),
        dest: dest.clone(),
        depth: None,
    };
    let out = run_stripped(
        &intent,
        &anonymous_env(&world.root.join("read-hooks")),
        &[],
        &user
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .chain([("GIT_ALLOW_PROTOCOL", OsString::from("https:file"))])
            .collect::<Vec<_>>(),
    );
    eprintln!(
        "unknown audited keys: the clone exited {:?}",
        out.status.code()
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dest.join(".git").exists());
}

/// **AC-P4-47-20's `VerifyRead` clause — its production caller.** The pre-flight handler, the
/// real read backend and the production write path with the `TransportFixture`: every step of
/// the verifying read runs, in order, through `handle_preflight_off_lock`, and the copy the
/// origin covers is `safe` with the call's own instant.
#[test]
fn the_verifying_read_runs_through_the_preflight() {
    use codotheca_core::analyser::remote::GitRemoteVerifier;
    use support::analyser_world::{Library, Verdict, NOW};

    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    // A branch the origin holds and the copy has never seen, so the objects step runs too.
    let other = lib.base.join("other");
    lib.git(
        &lib.base,
        &[
            "clone",
            "-q",
            &lib.net.join("widget.git").to_string_lossy(),
            &other.to_string_lossy(),
        ],
    );
    lib.git(&other, &["checkout", "-q", "-b", "elsewhere"]);
    lib.commit(&other, "e.txt", "elsewhere\n");
    lib.git(&other, &["push", "-q", "origin", "elsewhere"]);
    let id = lib.register(&copy);

    let counting = CountingWrite {
        inner: lib.write_git.clone(),
        steps: std::sync::Mutex::new(Vec::new()),
    };
    let remotes = GitRemoteVerifier::with_transport_fixture(
        &counting,
        &lib.read_git,
        codotheca_core::gitw::TransportFixture::new(&lib.net),
    );
    let trash = codotheca_core::testing::CountingTrash::new();
    let value = codotheca_core::uninstall::handle_preflight_off_lock(
        &lib.index,
        &codotheca_core::analyser::AnalyserSeams {
            git: &lib.read_git,
            remotes: &remotes,
            trash: &trash,
            before_act: None,
        },
        serde_json::json!({ "locationId": id }),
        NOW,
    )
    .expect("the pre-flight answers");
    let verdict = Verdict(value);
    let steps = counting.steps.lock().expect("steps").clone();
    eprintln!("VerifyRead through the pre-flight: {steps:?}; {verdict}");
    assert_eq!(
        steps,
        [
            codotheca_core::gitw::VerifyStepKind::ResolveUrl,
            codotheca_core::gitw::VerifyStepKind::Advertise,
            codotheca_core::gitw::VerifyStepKind::Objects,
        ]
        .iter()
        .map(|kind| format!("{:?}", Some(*kind)))
        .collect::<Vec<_>>()
    );
    assert_eq!(verdict.disposition(), "safe", "{verdict}");
    assert_eq!(verdict.0["remoteVerifiedAt"].as_i64(), Some(NOW));
}
