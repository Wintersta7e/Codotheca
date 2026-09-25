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
    let mut cmd = Command::new("git");
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
    let world = BundleWorld::new();
    let hooks = world.root.join("hooks-empty");
    std::fs::create_dir_all(&hooks).expect("hooks dir");
    let backend = SystemMutatingGit::new(PathBuf::from("git"), hooks.clone());
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
/// this local fixture — a local stand-in for the `TransportFixture` Task 6 lands, and the only
/// difference from the production child (the production child refuses this transport outright,
/// which would make the bundle half vacuous). The hazard is proved live the same way: without
/// `transfer.bundleURI=false` the clone writes the ref.
#[test]
fn a_bundle_advertising_origin_gives_a_clone_no_bundle_ref() {
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
    let global = world.root.join("user.gitconfig");
    let src_url = format!(
        "file://{}{}",
        if cfg!(windows) { "/" } else { "" },
        world.src.to_string_lossy().replace('\\', "/")
    );
    std::fs::write(
        &global,
        format!(
            "[transfer]\n\tbundleURI = true\n[url \"{src_url}\"]\n\tinsteadOf = {}\n",
            url.as_str()
        ),
    )
    .expect("user config");

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
            &[
                ("GIT_CONFIG_GLOBAL", global.clone().into_os_string()),
                (
                    "GIT_ALLOW_PROTOCOL",
                    OsString::from(format!("{}:file", intent.allowed_protocols())),
                ),
            ],
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
    let git = SystemMutatingGit::new(PathBuf::from("git"), hooks);
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
        let global = tmp.path().join("user.gitconfig");
        std::fs::write(
            &global,
            format!("[url \"{target}\"]\n\tinsteadOf = https://forge.invalid/acme/widget.git\n"),
        )
        .expect("user config");
        let ssh = format!("touch '{}' #", marker.to_string_lossy().replace('\\', "/"));
        support::git_world::run_in_child(
            INSTALL_TEST,
            tmp.path(),
            &[
                ("GIT_CONFIG_GLOBAL".to_owned(), global.into_os_string()),
                ("GIT_SSH_COMMAND".to_owned(), OsString::from(ssh)),
                (CASE_VAR.to_owned(), OsString::from(case)),
            ],
        );
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
        let root = dir.path().canonicalize().expect("canonical fixture root");
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
            std::sync::Arc::new(codotheca_core::git::GitExec::system(hooks)),
            std::sync::Arc::new(codotheca_core::git::GitSlots::for_machine()),
            std::sync::Arc::new(codotheca_core::clock::SystemClock::new()),
        )
    }

    fn write_git(&self) -> SystemMutatingGit {
        let hooks = codotheca_core::git::ensure_empty_hooks_dir(&self.root).expect("hooks");
        SystemMutatingGit::with_transport_fixture(
            PathBuf::from("git"),
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
    let mut cmd = Command::new("git");
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
    let index = std::sync::Arc::new(std::sync::Mutex::new(
        codotheca_core::index::Index::open_at(dir.path(), 1_750_000_000).expect("index"),
    ));
    let location = {
        let mut guard = index.lock().expect("index");
        guard
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, created_at, updated_at)
                     VALUES ('widget', 'widget', 0, 0)",
                    [],
                )?;
                let project = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO location
                       (project_id, kind, distro, path_bytes, path_key, path_display,
                        store_key, presence, repo_kind)
                     VALUES (?1, 'linux', '', ?2, ?2, ?3, 'store', 'present', 'worktree')",
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
    let hooks = codotheca_core::git::ensure_empty_hooks_dir(dir.path()).expect("hooks");
    let write_git = SystemMutatingGit::new(recording_git(), hooks);
    let read_git = support::git_world::system_git(&repo);
    let verdict = codotheca_core::uninstall::handle_preflight_off_lock(
        &index,
        &read_git,
        &write_git,
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
