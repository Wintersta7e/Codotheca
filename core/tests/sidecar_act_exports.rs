#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §48.8.6 and §1.12: an act that leaves bytes outside the index is followed by a newer sidecar,
//! and so is a clean shutdown. With the hourly export alone, a rebuild after either restores a
//! library from before it.

mod support;

use std::io::Write as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use codotheca_core::assembly::{CoreDeps, CoreHandler};
use codotheca_core::index::sidecar::{self, ExportAct};
use codotheca_core::index::Index;
use codotheca_core::proto::dispatch::CommandHandler;
use codotheca_core::proto::frame::{read_frame, write_frame};
use codotheca_core::proto::pubsub::{Publisher, PublisherSink};
use serde_json::{json, Value};
use support::analyser_world::{Library, NOW};

/// The generation of the sidecar in `data_dir`.
fn generation(data_dir: &Path) -> u64 {
    sidecar::read(&Index::sidecar_path(data_dir))
        .expect("the sidecar reads")
        .generation
}

/// The production handler over `lib`'s index and its real git seams. Everything the act does not
/// touch is the dispatch suite's fakes.
fn handler_over(lib: &Library) -> CoreHandler {
    let clock = Arc::new(codotheca_core::testing::FakeClock::new(NOW));
    let events = Arc::new(PublisherSink::new(Publisher::detached()));
    let http: Arc<dyn codotheca_core::http::HttpTransport> =
        Arc::new(codotheca_core::testing::FakeTransport::new());
    let sync_observing = Arc::new(codotheca_core::sync::http::ObservingTransport::new(
        Arc::clone(&http),
        clock.clone(),
    ));
    CoreHandler::new(CoreDeps {
        index: Arc::clone(&lib.index),
        provider: Arc::new(codotheca_core::provider::GitHubProvider::new(
            Arc::clone(&http),
            codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
        )),
        tokens: Arc::new(codotheca_core::testing::FakeTokenStore::unavailable()),
        http,
        client_id: "test-client-id".to_owned(),
        clock: clock.clone(),
        git: Arc::new(lib.read_git.clone()),
        write_git: Arc::new(lib.write_git.clone()),
        trash: Arc::new(codotheca_core::testing::CountingTrash::new()),
        mount: Arc::new(codotheca_core::testing::FakeMountResolver::default()),
        spawner: Box::new(codotheca_core::launch::spawn::RecordingSpawner::new()),
        sessions: codotheca_core::session::manager::SessionManager::new(
            clock.clone(),
            events.clone(),
            Box::new(codotheca_core::session::watch::FakeActivitySource::new()),
            Arc::new(codotheca_core::session::activity::FakeIgnoreCheck::new(&[])),
        ),
        scans: codotheca_core::scan::ScanSupervisor::new(Arc::new(
            codotheca_core::testing::ScanLauncherFake::new(),
        )),
        scan_store: Arc::new(codotheca_core::testing::MemScanStore::new()),
        firstrun: codotheca_core::firstrun::FirstRunEnv {
            sources: codotheca_core::firstrun::sources::SourceEnv {
                home: lib.home.clone(),
                app_data: None,
                xdg_config: None,
            },
            classifier: Arc::new(codotheca_core::firstrun::classify::FixedClassifier::new(
                vec![],
            )),
            distros: Arc::new(codotheca_core::firstrun::classify::NoDistros),
            platform: codotheca_core::index::path::PathPlatform::Unix,
            skip: codotheca_core::scan::skiplist::SkipList::default(),
            cache: codotheca_core::firstrun::roots::SuggestionCache::new(),
        },
        jobs: codotheca_core::assembly::jobs::JobPump::start(
            Arc::clone(&lib.index),
            Arc::new(codotheca_core::testing::FakeGitBackend::new()),
            clock.clone(),
            events.clone(),
            0,
        ),
        sync: codotheca_core::assembly::sync::SyncPump::start(
            Arc::clone(&lib.index),
            codotheca_core::sync::SyncDeps {
                provider: Arc::new(codotheca_core::provider::GitHubProvider::new(
                    sync_observing.clone(),
                    codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
                )),
                transport: sync_observing,
                tokens: Arc::new(codotheca_core::testing::FakeTokenStore::unavailable()),
                clock,
                cancel: codotheca_core::cancel::CancelToken::new(),
                tz_offset_min: 0,
            },
            events.clone(),
        ),
        events,
        tz_offset_min: 0,
    })
}

/// `locations.uninstall` through the router, over a copy that is safe to remove. Returns the
/// sidecar's generation before the act and after it.
///
/// The copy is an empty repository: nothing in it needs an elsewhere, so it is safe with no
/// remote. The router's verifier is the production one, which reads no fixture origin as network.
fn uninstall() -> (u64, u64) {
    let lib = Library::new();
    let copy = lib.root.join("widget");
    lib.git(
        &lib.base,
        &["init", "-q", "-b", "main", &copy.to_string_lossy()],
    );
    let id = lib.register(&copy);
    let data_dir = lib.base.join("index");
    let before = lib
        .index
        .lock()
        .expect("index")
        .export_sidecar(NOW)
        .expect("a starting sidecar")
        .generation;
    let mut handler = handler_over(&lib);
    let answer = handler.handle("locations.uninstall", json!({ "locationId": id }));
    assert!(answer.is_ok(), "the safe copy was not removed: {answer:?}");
    // Read before the shutdown below, which exports on its own.
    let after = generation(&data_dir);
    handler.shutdown();
    (before, after)
}

/// **AC-P4-48-22.** Every act `ExportAct` names, driven through its production arm, leaves a
/// sidecar newer than the one before it.
#[test]
fn ac_p4_48_22_every_act_is_followed_by_a_newer_export() {
    let mut exercised = 0_usize;
    for act in ExportAct::ALL {
        let (before, after) = match act {
            ExportAct::Uninstall => uninstall(),
        };
        eprintln!("{act:?}: sidecar generation {before} before, {after} after");
        assert!(
            after > before,
            "{act:?} left the sidecar at generation {after}"
        );
        exercised += 1;
    }
    eprintln!("acts exercised: {exercised}");
    assert!(exercised > 0, "no act was exercised");
}

fn send(child: &mut Child, frame: &Value) {
    let body = serde_json::to_vec(frame).expect("frame");
    let stdin = child.stdin.as_mut().expect("stdin");
    write_frame(stdin, &body).expect("write");
    stdin.flush().expect("flush");
}

fn recv(child: &mut Child) -> Value {
    let stdout = child.stdout.as_mut().expect("stdout");
    let mut buf = Vec::new();
    read_frame(stdout, &mut buf).expect("read");
    serde_json::from_slice(&buf).expect("json")
}

/// §1.12's clean-shutdown export, through the real binary: the handshake, then stdin closed.
#[test]
fn a_clean_shutdown_exports() {
    let dir = tempfile::tempdir().expect("tmp");
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after the epoch")
            .as_secs(),
    )
    .expect("seconds fit");
    // Written now, so the loop's first tick finds no hourly export due: only the shutdown can
    // write the next generation.
    let start = Index::open_at(dir.path(), now)
        .expect("index")
        .export_sidecar(now)
        .expect("a starting sidecar")
        .generation;

    let mut child = Command::new(env!("CARGO_BIN_EXE_codotheca-core"))
        .arg(format!("--data-dir={}", dir.path().display()))
        .arg("--epoch=7")
        .arg(format!("--parent-pid={}", std::process::id()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("core spawns");
    let hello = recv(&mut child);
    assert_eq!(hello["t"], "hello", "{hello}");
    send(
        &mut child,
        &json!({"t": "request", "id": 1, "command": "app.hello_ack", "args": {}}),
    );
    let ack = loop {
        let frame = recv(&mut child);
        if frame["t"] != "event" && frame["id"] == 1 {
            break frame;
        }
    };
    assert_eq!(ack["t"], "response", "{ack}");
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("exits");
    assert!(
        out.status.success(),
        "{:?}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let after = generation(dir.path());
    eprintln!("sidecar generation {start} at start, {after} after the shutdown");
    assert!(
        after > start,
        "a clean shutdown left the sidecar at generation {after}"
    );
}
