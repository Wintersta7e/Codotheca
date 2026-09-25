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
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::neutralise_env;
use codotheca_core::gitw::{
    write_base_args, AuditFixture, CredentialChannel, FilterDrivers, Intent, MutatingGit,
    RemoteName, RemoteUrl, SystemMutatingGit, WriteEnv,
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
            "bundle pins: {:?} — {} argv tokens, GIT_ALLOW_PROTOCOL={allowed:?}",
            intent.kind(),
            recorded.argv.len()
        );
    }
    assert_eq!(checked, Intent::ALL.len(), "every intent kind is checked");
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

/// The production intent's child, minus exactly the listed `-c` pins — the **bite**, rendered in
/// the test so the product is never built without them.
fn stripped_child(intent: &Intent, env: &WriteEnv, strip: &[&str]) -> Command {
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
    cmd.env("GIT_ALLOW_PROTOCOL", intent.allowed_protocols());
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
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
        let intent = Intent::Fetch {
            work_dir: work.clone(),
            remote: RemoteName::parse("origin").expect("remote"),
        };
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
        let control_intent = Intent::Fetch {
            work_dir: control.clone(),
            remote: RemoteName::parse("origin").expect("remote"),
        };
        let mut env = anonymous_env(&hooks);
        env.work_dir = Some(control.clone());
        let _ = stripped_child(&control_intent, &env, &["fetch.bundleURI="])
            .output()
            .expect("control child");
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
        let mut cmd = stripped_child(&intent, &anonymous_env(&hooks), strip);
        cmd.env("GIT_CONFIG_GLOBAL", &global);
        cmd.env(
            "GIT_ALLOW_PROTOCOL",
            format!("{}:file", intent.allowed_protocols()),
        );
        let out = cmd.output().expect("clone child");
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
