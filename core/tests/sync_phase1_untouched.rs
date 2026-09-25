#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! **AC-P2-21-10** — phase 1 is untouched.
//!
//! §21.1 lists six structural blockers against making sync a job, and this is the gate that stops
//! a later author undoing the decision by halves: a `JobKind` variant added *"for symmetry"*, a
//! `project_job_state` row written by a sync run, or `JobOutcome::Partial` pressed into service as
//! the yield a park needs and is not (**A2**).

use std::sync::{Arc, Mutex};

use codotheca_core::accounts::keychain::{token_ref, SecretToken, TokenStore};
use codotheca_core::accounts::store::{insert_account, NewAccount};
use codotheca_core::http::HttpResponse;
use codotheca_core::index::Index;
use codotheca_core::jobs::JobKind;
use codotheca_core::protocol::{AuthKind, ProjectId, ScopeTier, SyncTaskKind};
use codotheca_core::provider::GitHubProvider;
use codotheca_core::sync::http::ObservingTransport;
use codotheca_core::sync::runner::SyncRunner;
use codotheca_core::sync::task::{kind_slug, SyncTask};
use codotheca_core::sync::SyncDeps;
use codotheca_core::testing::{FakeClock, FakeTokenStore, FakeTransport, TempIndex};

const NOW: i64 = 1_800_000_000;
const HOST: &str = "forge.example.invalid";

#[derive(Debug)]
struct Quiet;

impl codotheca_core::proto::EventSink for Quiet {
    fn emit(&self, _topic: &str, _event: &str, _payload: serde_json::Value) {}
}

/// §4.1's vocabulary holds no sync task, whatever its size.
///
/// This is what stops a later *"add a sync task for symmetry"*: sync is its own runner because a
/// sync task has no `location_id` and the queue coalesces on one, and a sentinel id would collide
/// every project's task onto one queue entry and silently drop all but one.
///
/// **The count is gone and the name went with it.** It asserted `len() == 7` under the name
/// `the_job_vocabulary_is_still_seven`, which stopped being true when §29 added `j7` — a real
/// eighth *job*, not a sync task, so the thing this test is about did not change at all. The
/// property is a **set** property and is now stated as one (R132/F16).
#[test]
fn the_job_vocabulary_holds_no_sync_task() {
    let jobs: Vec<&'static str> = JobKind::ALL.iter().map(|k| k.slug()).collect();
    let syncs: Vec<&'static str> = SyncTaskKind::ALL.iter().map(|k| kind_slug(*k)).collect();
    eprintln!("sync_phase1_untouched: jobs {jobs:?}, sync tasks {syncs:?}");
    assert!(!jobs.is_empty(), "the job vocabulary is empty");
    assert!(!syncs.is_empty(), "the sync vocabulary is empty");
    for sync in &syncs {
        assert!(!jobs.contains(sync), "{sync} names a job and a sync task");
    }
}

/// **A full sync run writes no `project_job_state` row.**
///
/// `project_job_state` is `PRIMARY KEY (project_id, job)` over a NOT NULL foreign key: an account
/// listing belongs to no project by construction, so the row is unrepresentable rather than merely
/// awkward. Asserting the count is unchanged is how a writer that found a way anyway is caught.
/// A seeded index, a scripted forge and a runner over both.
///
/// Extracted from the test below rather than inlined: the assertion is *one* count, and a
/// hundred lines of fixture around it is what makes a reviewer stop reading before reaching it.
fn seeded_lane() -> (
    Arc<Mutex<Index>>,
    Arc<SyncRunner>,
    codotheca_core::protocol::AccountId,
    tempfile::TempDir,
) {
    let temp = TempIndex::new();
    let dir = tempfile::tempdir().expect("tmp");
    let mut index = Index::open_at(dir.path(), NOW).expect("index");
    drop(temp);

    let (account, _project) = index
        .with_tx(|tx| {
            let account = insert_account(
                tx,
                &NewAccount {
                    provider: "github".to_owned(),
                    host: HOST.to_owned(),
                    login: "owner".to_owned(),
                    display_name: None,
                    auth_kind: AuthKind::Device,
                    scope_tier: ScopeTier::Private,
                    granted_scopes: vec!["repo".to_owned()],
                    token_ref: token_ref("github", HOST, "owner"),
                },
                NOW,
            )
            .expect("account");
            tx.execute(
                "INSERT INTO project (name, seed_basename, remote_key, provider,
                                      provider_repo_id, remote_link_basis, created_at, updated_at)
                 VALUES ('alpha', 'alpha', ?1, 'github', '7', 'provider_id', ?2, ?2)",
                rusqlite::params![format!("{HOST}/owner/alpha"), NOW],
            )?;
            Ok((account, ProjectId(tx.last_insert_rowid())))
        })
        .expect("seed");

    let index = Arc::new(Mutex::new(index));
    let scripted = Arc::new(FakeTransport::new());
    for _ in 0..6 {
        scripted.push(HttpResponse {
            status: 200,
            headers: codotheca_core::http::normalise_headers([("x-ratelimit-resource", "core")]),
            body: b"[]".to_vec(),
        });
    }
    let clock = Arc::new(FakeClock::new(NOW));
    let observing = Arc::new(ObservingTransport::new(scripted, clock.clone()));
    let tokens = Arc::new(FakeTokenStore::available());
    tokens
        .store(
            &token_ref("github", HOST, "owner"),
            &SecretToken::new("t".to_owned()),
        )
        .expect("token");
    let runner = SyncRunner::new(
        Arc::clone(&index),
        SyncDeps {
            provider: Arc::new(GitHubProvider::new(observing.clone(), HOST.to_owned())),
            transport: observing,
            tokens,
            clock,
            cancel: codotheca_core::cancel::CancelToken::new(),
            // UTC in a test, so a local date never depends on the machine running it.
            tz_offset_min: 0,
        },
        Arc::new(Quiet),
    );
    (index, runner, account, dir)
}

#[test]
fn a_full_sync_run_writes_no_job_row() {
    let (index, runner, account, _dir) = seeded_lane();
    let before: i64 = {
        let guard = index.lock().expect("index");
        guard
            .conn()
            .query_row("SELECT count(*) FROM project_job_state", [], |r| r.get(0))
            .expect("counted")
    };

    // All three kinds, so no arm of the runner is left unexercised by this assertion.
    runner.enqueue(SyncTask::AccountRepos {
        account_id: account,
    });
    runner.enqueue(SyncTask::RenameProbe {
        account_id: account,
    });
    runner.enqueue(SyncTask::ProjectRemote {
        project_id: ProjectId(1),
    });
    runner.start();
    // **The rows must exist before "nothing is pending" means anything**: `enqueue` records a
    // task in memory and the loop writes its row, so an empty table satisfies the settle
    // condition vacuously.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        let (rows, pending): (i64, i64) = {
            let guard = index.lock().expect("index");
            let conn = guard.conn();
            let counts = (
                conn.query_row("SELECT count(*) FROM sync_task_state", [], |r| r.get(0))
                    .unwrap_or(0),
                conn.query_row(
                    "SELECT count(*) FROM sync_task_state WHERE state IN ('queued', 'running')",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(1),
            );
            drop(guard);
            counts
        };
        if rows >= 3 && pending == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    runner.request_stop();
    runner.join();

    let guard = index.lock().expect("index");
    let after: i64 = guard
        .conn()
        .query_row("SELECT count(*) FROM project_job_state", [], |r| r.get(0))
        .expect("counted");
    let settled: i64 = guard
        .conn()
        .query_row("SELECT count(*) FROM sync_task_state", [], |r| r.get(0))
        .expect("counted");
    drop(guard);
    eprintln!(
        "sync_phase1_untouched: {settled} sync row(s), project_job_state {before} -> {after}"
    );
    assert!(settled >= 3, "the run must actually have happened");
    assert_eq!(after, before, "a sync run wrote a job row");
}

/// **`JobOutcome::Partial` is reached by the state machine and by J7, and by nothing else.**
///
/// Its arm returns `(next, Some(now))` — a yield is *immediately* runnable, which is right for a
/// CPU chunk and exactly wrong for a page that must not be fetched until `reset_at`. That is why
/// `core::sync` may not reach it (A2), and phase 2 did not.
///
/// **[p3] §29.6 makes J7 the first production writer**, so the set grew by exactly one named
/// file. It is a **set**, not a count: a new construction anywhere else still fails, and so does
/// a second one in either of these.
///
/// **This counts mentions per file rather than parsing Rust**, and says so rather than implying
/// more. A first attempt tried to tell a construction from a match arm by looking for `=>` on the
/// same line, and `core/src/jobs/state.rs:78`'s arm spreads its pattern over four lines — the
/// gate reported the variant's *only* legitimate reader as an offender. What is asserted instead
/// is the property that actually holds and that a new construction would break: **exactly one
/// production file mentions the variant at all, it is the state machine that must match on it,
/// and it mentions it once.** A construction anywhere else fails; a second one in that file fails.
#[test]
fn job_outcome_partial_is_mentioned_by_one_production_file_once() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut scanned = 0_usize;
    let mut mentions: Vec<(String, usize)> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("core/src is readable") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            scanned += 1;
            let text = std::fs::read_to_string(&path).expect("readable");
            // The production half only: an inline `#[cfg(test)] mod tests` is not production, and
            // `core/src/jobs/state.rs` carries the one legitimate construction there.
            let production = text.split("#[cfg(test)]").next().unwrap_or(&text);
            let count = production
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .filter(|line| line.contains("JobOutcome::Partial"))
                .count();
            if count > 0 {
                let name = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                mentions.push((name, count));
            }
        }
    }
    mentions.sort();
    eprintln!(
        "sync_phase1_untouched: scanned {scanned} file(s); JobOutcome::Partial in {mentions:?}"
    );
    assert!(scanned > 0, "a gate that scanned nothing is a failing gate");
    assert_eq!(
        mentions,
        vec![
            ("jobs/j7_markers.rs".to_owned(), 1),
            ("jobs/state.rs".to_owned(), 1)
        ],
        "JobOutcome::Partial reached a production path no section has given it"
    );
}

/// §21.13: **no phase-1 file under `core/src/jobs/` or `core/src/freshness/` changed.** The gate
/// is not a diff — it is the statement that `core::sync` reaches neither, which is what makes the
/// two runners independent rather than merely separate.
#[test]
fn the_sync_module_imports_nothing_from_the_job_scheduler_or_freshness() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sync");
    let mut scanned = 0_usize;
    let mut offenders: Vec<String> = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("core/src/sync is readable") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            scanned += 1;
            let text = std::fs::read_to_string(&path).expect("readable");
            for line in text.lines() {
                let line = line.trim();
                if !line.starts_with("use ") {
                    continue;
                }
                if line.contains("crate::jobs::") || line.contains("crate::freshness::") {
                    offenders.push(format!("{}: {line}", path.display()));
                }
            }
        }
    }
    eprintln!("sync_phase1_untouched: scanned {scanned} sync file(s) for phase-1 imports");
    assert!(scanned > 0);
    assert!(
        offenders.is_empty(),
        "core::sync reaches into the job scheduler or the freshness basis: {offenders:?}"
    );
}
