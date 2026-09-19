//! §31.5's evaluator has **exactly two production call sites**, and R123 names them because an
//! unnamed trigger is how a producer gets its fake and never its real implementation.
//!
//! None of §31.5's own three triggers is observable: `coverage_for` returns two booleans,
//! `next_jobs_after` returns empty for four job kinds — so *the last input job for a project* is a
//! thing nothing in this code base can report — and two of the ten checks come from sync and one
//! from a scheduled sweep, so a job-shaped trigger could never fire for them at all.
//!
//! A source walk can count call sites and cannot prove one fires. Both are here.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::testing::FakeClock;
use support::TestRepo;

/// Frozen, so a second run of the same job writes the same observation times — which is what
/// makes *the row did not change* a state a test can reach at all.
const T0: i64 = 1_700_000_000;

#[derive(Debug, Default)]
struct SilentSink;

impl EventSink for SilentSink {
    fn emit(&self, _topic: &str, _event: &str, _payload: serde_json::Value) {}
}

fn core_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_sources(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                // Skipped BEFORE it is counted, so the guard below keeps meaning what it says.
                if let Ok(text) = std::fs::read_to_string(&path) {
                    out.push((path.display().to_string(), text));
                }
            }
        }
    }
    out
}

/// **Exactly two**, and the walk prints the number of files it read and fails at zero.
#[test]
fn the_evaluator_has_exactly_two_production_call_sites() {
    let sources = rust_sources(&core_src());
    eprintln!(
        "completion_hooks: walked {} core source file(s)",
        sources.len()
    );
    assert!(
        !sources.is_empty(),
        "the walk read no file, so it proved nothing"
    );

    let callers: Vec<&String> = sources
        .iter()
        // The declaration itself, and the `set_check_na` wrapper beside it, both live in
        // `completion/mod.rs`; what is being counted is who calls in from outside the module.
        .filter(|(path, _)| !path.ends_with("completion/mod.rs"))
        .filter(|(_, text)| text.contains("evaluate_and_write"))
        .map(|(path, _)| path)
        .collect();

    assert_eq!(
        callers.len(),
        2,
        "R123 names two hook sites — JobRunner::settle and SyncRunner::settle — and a third \
         would be a trigger nobody can count: {callers:?}"
    );
    assert!(
        callers.iter().any(|p| p.ends_with("jobs/scheduler.rs")),
        "hook site 1 is JobRunner::settle: {callers:?}"
    );
    assert!(
        callers.iter().any(|p| p.ends_with("sync/runner.rs")),
        "hook site 2 is SyncRunner::settle: {callers:?}"
    );
}

/// **The ordering, read off the source at each site.**
///
/// Six of §31's ten checks read what §28 wrote in the same transaction (R124). Reversing the two
/// makes every Group-A check answer from the **previous** settle — a one-settle lag that is
/// invisible in any fixture whose inputs did not change, and that no test of either plan alone
/// would catch: §28's tests see correct rows written and §31's see rows that are merely stale.
#[test]
fn the_singleton_evaluator_runs_before_the_completion_evaluator_at_both_sites() {
    let sources = rust_sources(&core_src());
    let mut checked = 0_u32;
    for suffix in ["jobs/scheduler.rs", "sync/runner.rs"] {
        let (_, text) = sources
            .iter()
            .find(|(path, _)| path.ends_with(suffix))
            .unwrap_or_else(|| panic!("{suffix} is not in the walk"));
        let singletons = text
            .find("settle_singletons(")
            .unwrap_or_else(|| panic!("{suffix} does not call §28's evaluator"));
        let completion = text
            .find("completion::evaluate_and_write(")
            .unwrap_or_else(|| panic!("{suffix} does not call §31's evaluator"));
        assert!(
            singletons < completion,
            "{suffix}: §31's evaluator must not run before §28's, or every Group-A check answers \
             from the previous settle"
        );
        checked += 1;
    }
    eprintln!("completion_hooks: ordering checked at {checked} site(s)");
    assert_eq!(checked, 2);
}

/// **[p3] §31.5's hook site 1, proved live.** A real job settles and ten rows exist afterwards.
///
/// A source walk can count call sites and cannot prove one fires; this is the other half.
#[test]
fn a_job_settle_leaves_ten_rows_and_a_moved_projection() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");

    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open_at(dir.path(), 0).unwrap()));
    let (project, location) = {
        let guard = index.lock().unwrap();
        let conn = guard.conn();
        conn.execute(
            "INSERT INTO project (name, seed_basename, authored_by_user, created_at, updated_at)
             VALUES ('p', 'p', 1, 0, 0)",
            [],
        )
        .unwrap();
        let project = ProjectId(conn.last_insert_rowid());
        let path = repo.path().to_string_lossy().into_owned();
        conn.execute(
            "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                   store_key, presence, repo_kind)
             VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
            rusqlite::params![project.0, path.as_bytes(), path],
        )
        .unwrap();
        (project, LocationId(conn.last_insert_rowid()))
    };

    let rows = |index: &Mutex<Index>| -> i64 {
        let guard = index.lock().unwrap();
        guard
            .conn()
            .query_row(
                "SELECT count(*) FROM project_check WHERE project_id = ?1",
                [project.0],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(rows(&index), 0, "nothing has settled yet");

    let clock = Arc::new(FakeClock::new(T0));
    let deps = JobDeps {
        git: Arc::new(SystemGit::new(
            Arc::new(repo.exec()),
            Arc::new(GitSlots::new(4)),
            Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
        )),
        clock: Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
        cancel: CancelToken::new(),
        tz_offset_min: 0,
    };
    let runner = JobRunner::new(
        Arc::clone(&index),
        deps,
        Arc::new(SilentSink) as Arc<dyn EventSink>,
    );
    assert!(runner.enqueue(Job {
        kind: JobKind::J1Refstate,
        project_id: project,
        location_id: location,
        store_key: "store".to_owned(),
        store_kind: StoreClass::Local,
        priority: Priority::Interactive,
        not_before: 0,
        origin: JobOrigin::Interactive,
    }));
    runner.start(1);

    // Spun rather than slept: a fixed wait is a race that reports the machine it ran on.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline && rows(&index) != 10 {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    runner.request_stop();
    runner.join();

    assert_eq!(
        rows(&index),
        10,
        "§31.5's evaluator did not fire from JobRunner::settle"
    );

    // **The ordering, observed rather than read off the source.** Nothing was swept before this
    // settle, so if §31's evaluator ran BEFORE §28's it would find no `debt_sweep` row for
    // `unpushed_commits` and write `notRunYet`. §28's arm ran first, found no upstream ref and
    // recorded `unobservable`, so §31 maps it to its own per-key reason — `notObserved`.
    //
    // The two reasons are the one-settle lag made visible: `notRunYet` resolves at the next
    // sweep and `notObserved` is a fact about the repository, and a stale read says the first
    // when the truth is the second.
    let (state, reason): (String, Option<String>) = {
        let guard = index.lock().unwrap();
        guard
            .conn()
            .query_row(
                "SELECT state, unknown_reason FROM project_check
                  WHERE project_id = ?1 AND check_key = 'pushed'",
                [project.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("the pushed row")
    };
    assert_eq!(state, "unknown");
    assert_eq!(
        reason.as_deref(),
        Some("notObserved"),
        "§31 read §28's answer from the PREVIOUS settle: the two evaluators are in the wrong order"
    );
}
