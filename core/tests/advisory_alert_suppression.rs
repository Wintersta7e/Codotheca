#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §32.12's fifth conjunct, through the **real** sync runner: a `surface_suppressed` project never
//! fires the one notification.
//!
//! The predicate is §30's. The runner used to pass one that suppressed nothing, so an unenrolled or
//! archived project with a new critical advisory interrupted the user exactly like an enrolled one.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::index::Index;
use codotheca_core::protocol::{ProjectId, SyncTaskKind, SyncTaskState};
use codotheca_core::provider::GitHubProvider;
use codotheca_core::sync::http::ObservingTransport;
use codotheca_core::sync::runner::SyncRunner;
use codotheca_core::sync::state::SyncTaskStateRow;
use codotheca_core::sync::store::{load, put};
use codotheca_core::sync::SyncDeps;
use codotheca_core::testing::{FakeClock, FakeTokenStore, FakeTransport};

const NOW: i64 = 1_800_000_000;
const HOST: &str = "forge.example.invalid";
const DEADLINE: Duration = Duration::from_secs(10);

/// Every event the runner emits, kept so the test can find the one alert.
#[derive(Debug, Default)]
struct Recorder(Mutex<Vec<(String, serde_json::Value)>>);

impl codotheca_core::proto::EventSink for Recorder {
    fn emit(&self, _topic: &str, event: &str, payload: serde_json::Value) {
        self.0.lock().unwrap().push((event.to_owned(), payload));
    }
}

fn deps() -> SyncDeps {
    let clock = Arc::new(FakeClock::new(NOW));
    let observing = Arc::new(ObservingTransport::new(
        Arc::new(FakeTransport::new()),
        clock.clone(),
    ));
    let provider = Arc::new(GitHubProvider::new(observing.clone(), HOST.to_owned()));
    SyncDeps {
        provider,
        transport: observing,
        tokens: Arc::new(FakeTokenStore::available()),
        clock,
        cancel: codotheca_core::cancel::CancelToken::new(),
        tz_offset_min: 0,
    }
}

/// An installed project holding one triple that matches one critical advisory with a fix, asked
/// about by the sweep `sweep` — so the sweep has nothing left to ask and settles without a request.
fn vulnerable(
    tx: &rusqlite::Transaction<'_>,
    name: &str,
    acknowledged_at: Option<i64>,
    archived: bool,
    sweep: i64,
) -> ProjectId {
    tx.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, acknowledged_at, is_archived,
                              created_at, updated_at)
         VALUES (?1, ?1, ?1, ?2, ?3, 1, 1)",
        rusqlite::params![name, acknowledged_at, i64::from(archived)],
    )
    .unwrap();
    let project = ProjectId(tx.last_insert_rowid());
    tx.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
        rusqlite::params![project.0, name.as_bytes(), name],
    )
    .unwrap();
    let advisory = format!("GHSA-{name}");
    tx.execute(
        "INSERT INTO project_dependency
           (project_id, ecosystem, package_name, version, source_path, observed_at)
         VALUES (?1, 'npm', ?2, '1.0.0', 'package-lock.json', ?3)",
        rusqlite::params![project.0, name, NOW],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO advisory_triple
           (ecosystem, package_name, version, sweep_id, observed_at, answered)
         VALUES ('npm', ?1, '1.0.0', ?2, ?3, 1)",
        rusqlite::params![name, sweep, NOW],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO advisory (advisory_id, severity, summary, url, observed_at)
         VALUES (?1, 'critical', 's', 'u', ?2)",
        rusqlite::params![advisory, NOW],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO advisory_match
           (ecosystem, package_name, version, advisory_id, fix_available, fixed_version)
         VALUES ('npm', ?1, '1.0.0', ?2, 1, '2.0.0')",
        rusqlite::params![name, advisory],
    )
    .unwrap();
    project
}

/// An unenrolled project and an archived one each carry a new critical advisory with a fix, beside
/// an enrolled one that does. The sweep settles through `SyncRunner`, and **only the enrolled
/// project fires or enters the ledger**.
#[test]
fn a_surface_suppressed_project_never_fires_through_the_runner() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), NOW).unwrap();
    let (enrolled, unenrolled, archived) = index
        .with_tx(|tx| {
            // The sweep in flight: every triple is already answered under it, so the next pick
            // closes it and settles `Done`.
            tx.execute(
                "INSERT INTO advisory_sweep (started_at, complete) VALUES (?1, 0)",
                [NOW],
            )?;
            let sweep = tx.last_insert_rowid();
            let enrolled = vulnerable(tx, "enrolled", Some(NOW), false, sweep);
            let unenrolled = vulnerable(tx, "unenrolled", None, false, sweep);
            let archived = vulnerable(tx, "archived", Some(NOW), true, sweep);
            put(
                tx,
                &SyncTaskStateRow::queued(SyncTaskKind::Advisories, None, NOW),
            )?;
            Ok((enrolled, unenrolled, archived))
        })
        .unwrap();
    let index = Arc::new(Mutex::new(index));
    let events = Arc::new(Recorder::default());

    let runner = SyncRunner::new(Arc::clone(&index), deps(), events.clone());
    runner.start();
    let deadline = Instant::now() + DEADLINE;
    loop {
        let state = {
            let guard = index.lock().unwrap();
            load(guard.conn(), SyncTaskKind::Advisories, None)
                .unwrap()
                .map(|r| r.state)
        };
        if state == Some(SyncTaskState::Ok) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the sweep never settled: {state:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    runner.request_stop();
    runner.join();

    let alerts: Vec<serde_json::Value> = events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(event, _)| event == "advisory_alert")
        .map(|(_, payload)| payload.clone())
        .collect();
    let notified: Vec<i64> = {
        let guard = index.lock().unwrap();
        let mut stmt = guard
            .conn()
            .prepare("SELECT project_id FROM advisory_notified ORDER BY project_id")
            .unwrap();
        let ids = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(stmt);
        drop(guard);
        ids
    };
    eprintln!("advisory_alert_suppression: alerts {alerts:?}, ledger {notified:?}");

    assert_eq!(alerts.len(), 1, "the enrolled project fires exactly once");
    assert_eq!(
        alerts[0]["projectCount"], 1,
        "a suppressed project was counted in the alert"
    );
    assert_eq!(alerts[0]["projectId"], enrolled.0);
    assert_eq!(
        notified,
        vec![enrolled.0],
        "a suppressed project entered the ledger: unenrolled {}, archived {}",
        unenrolled.0,
        archived.0
    );
}
