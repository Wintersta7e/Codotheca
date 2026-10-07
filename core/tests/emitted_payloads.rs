#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Every event the core emits is the type the schema declares for it (§48.11).
//!
//! The core emits untyped JSON through `EventSink`, so a payload drifting from its schema type
//! compiles, passes every Rust test, and reaches a renderer that reads keys that are not there.
//! The first test drives a real scan and the real job pump into a validating sink; the second
//! proves every test sink validates, so the rest of the suite checks every event it sees.

mod support;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use codotheca_core::assembly::jobs::JobPump;
use codotheca_core::clock::Clock;
use codotheca_core::corpus::{ensure, CorpusOptions};
use codotheca_core::git::{ensure_empty_hooks_dir, GitBackend, GitExec, GitSlots, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::paths::path_key;
use codotheca_core::protocol::ScanMode;
use codotheca_core::scan::launcher::{ScanLauncherDeps, ThreadScanLauncher};
use codotheca_core::scan::run::platform_of;
use codotheca_core::scan::skiplist::SkipList;
use codotheca_core::scan::store::SqliteScanStore;
use codotheca_core::scan::ScanSupervisor;
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::{FakeClock, FakeMountResolver};

const NOW: i64 = 1_700_000_000;

/// Small repositories on one volume, none of them a slow one: the point is the events, not scale.
const FIXTURES: [&str; 4] = ["upstream", "other-upstream", "zero-commit", "multi-root"];

/// A walk over four repositories plus every job they queue, on a loaded machine.
const DEADLINE: Duration = Duration::from_secs(240);

const fn host_kind() -> &'static str {
    if cfg!(windows) {
        "win"
    } else {
        "linux"
    }
}

fn corpus_volume(root: &Path) -> PathBuf {
    let mut options = CorpusOptions::new(root);
    options.only = Some(FIXTURES.iter().map(|n| (*n).to_owned()).collect());
    let manifest = ensure(&options).expect("corpus builds");
    for name in FIXTURES {
        let fixture = manifest.require(name).expect("fixture built");
        assert!(
            fixture.materialised,
            "{name} was skipped ({:?})",
            fixture.skip_reason
        );
    }
    manifest
        .volume("vol-a")
        .expect("the corpus lays these fixtures on volume A")
        .path
        .clone()
}

fn seed_root(index: &Mutex<Index>, path: &Path) {
    index
        .lock()
        .unwrap()
        .conn()
        .execute(
            "INSERT INTO scan_root (kind, distro, path_bytes, path_key, path_display,
                                    enabled, added_by, descend_into_repos, added_at)
             VALUES (?1, '', ?2, ?3, ?4, 1, 'user', 0, 1)",
            rusqlite::params![
                host_kind(),
                path.to_string_lossy().as_bytes(),
                path_key(path, platform_of(host_kind())),
                path.display().to_string()
            ],
        )
        .unwrap();
}

/// Every panic, from any thread. A payload the validator refuses inside the job pump panics a
/// worker thread, which the test thread never joins, so without this the test would pass on the
/// events the surviving workers emitted.
static PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn record_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        PANICS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(info.to_string());
        default(info);
    }));
}

/// Waits until the job pump has published nothing new for two seconds, after at least one job.
fn await_quiet(events: &ValidatingSink) {
    let deadline = Instant::now() + DEADLINE;
    let mut last = (0, Instant::now());
    while Instant::now() < deadline {
        // A refused payload kills the worker that emitted it; waiting on would only time out.
        let panics = PANICS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        assert_eq!(panics, Vec::<String>::new(), "a thread panicked");
        let now = events.count("scan", "job_done");
        if now != last.0 {
            last = (now, Instant::now());
        } else if now > 0 && last.1.elapsed() > Duration::from_secs(2) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the job pump never went quiet; {} jobs settled", last.0);
}

#[test]
fn ac_p4_48_23_emitted_payloads_validate() {
    record_panics();
    let corpus = tempfile::tempdir().unwrap();
    let volume = corpus_volume(corpus.path());
    let data = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open(data.path()).unwrap()));
    seed_root(&index, &volume);

    let events = Arc::new(ValidatingSink::default());
    let clock: Arc<dyn Clock> = Arc::new(FakeClock::new(NOW));
    let hooks = ensure_empty_hooks_dir(data.path()).unwrap();
    let git: Arc<dyn GitBackend> = Arc::new(SystemGit::new(
        Arc::new(GitExec::new(support::test_git(), hooks)),
        Arc::new(GitSlots::new(4)),
        Arc::clone(&clock),
    ));
    let pump = JobPump::start(
        Arc::clone(&index),
        Arc::clone(&git),
        Arc::clone(&clock),
        events.clone(),
        0,
    );
    let mounts = FakeMountResolver::new();
    mounts.map(
        &volume,
        MountFacts {
            store_key: "store-a".to_owned(),
            volume_key: Some("vol-a".to_owned()),
            class: StoreClass::Local,
        },
    );
    let scans = ScanSupervisor::new(Arc::new(ThreadScanLauncher::new(ScanLauncherDeps {
        store: Arc::new(SqliteScanStore::new(Arc::clone(&index))),
        index: Arc::clone(&index),
        git,
        mounts: Arc::new(mounts),
        clock,
        skip: Arc::new(SkipList::default()),
        wsl: None,
        jobs: pump.sink(),
        events: events.clone(),
    })));

    scans.start(ScanMode::Full, NOW).unwrap();
    let deadline = Instant::now() + DEADLINE;
    while scans.live().is_some() {
        assert!(Instant::now() < deadline, "the scan never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    await_quiet(&events);
    pump.stop();

    let seen = events.events.lock().unwrap().clone();
    let mut kinds: Vec<String> = seen.iter().map(|(t, e, _)| format!("{t}/{e}")).collect();
    kinds.sort();
    kinds.dedup();
    eprintln!("events validated: {} ({})", seen.len(), kinds.join(", "));
    assert_eq!(
        *PANICS.lock().unwrap_or_else(PoisonError::into_inner),
        Vec::<String>::new(),
        "a thread panicked while the scan ran"
    );
    assert!(
        !seen.is_empty(),
        "a scan that emitted nothing validated nothing"
    );
    // No `scan/repo_found`: the launcher does not publish one (`scan/launcher.rs`'s module doc).
    for (topic, event) in [
        ("scan", "progress"),
        ("scan", "job_done"),
        ("scan", "finished"),
    ] {
        assert!(
            events.count(topic, event) > 0,
            "the scan emitted no {topic}/{event}, so its shape went unchecked"
        );
    }
}

/// Every `.rs` under `dir`. A file that vanishes between the listing and the read is skipped
/// before it is counted, so another test's short-lived probe cannot fail this walk.
fn rust_files(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            match std::fs::read_to_string(&path) {
                Ok(text) => out.push((path, text)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => panic!("{}: {e}", path.display()),
            }
        }
    }
}

/// The body of the first `fn emit` at or after `from`, by brace depth.
fn emit_body(lines: &[&str], from: usize) -> String {
    let start = (from..lines.len())
        .find(|&i| lines[i].trim_start().starts_with("fn emit("))
        .expect("an EventSink impl defines emit");
    let mut depth = 0i32;
    let mut body = String::new();
    for line in &lines[start..] {
        body.push_str(line);
        body.push('\n');
        for c in line.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
        if depth == 0 && line.contains('}') {
            break;
        }
    }
    body
}

#[test]
fn every_test_event_sink_validates() {
    let core = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&core.join("src"), &mut files);
    rust_files(&core.join("tests"), &mut files);
    assert!(!files.is_empty(), "the walk read no source");

    let mut sinks = Vec::new();
    let mut offenders = Vec::new();
    for (path, text) in &files {
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if !(trimmed.starts_with("impl") && trimmed.contains("EventSink for ")) {
                continue;
            }
            let name = format!(
                "{}:{} {}",
                path.strip_prefix(core).unwrap_or(path).display(),
                i + 1,
                trimmed.trim_end_matches(" {")
            );
            if !emit_body(&lines, i + 1).contains("validated(") {
                offenders.push(name.clone());
            }
            sinks.push(name);
        }
    }
    eprintln!("event sinks found: {}", sinks.len());
    assert!(!sinks.is_empty(), "the walk found no EventSink impl");
    assert_eq!(
        offenders,
        Vec::<String>::new(),
        "sinks that do not validate"
    );
}
