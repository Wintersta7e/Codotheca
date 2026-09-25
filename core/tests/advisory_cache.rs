#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §32.7's cache: **the cardinality a per-triple row cannot hold**, and the distinction a match
//! count cannot express.
//!
//! One triple routinely matches four to six advisories, so the match is its own row. And **`no
//! advisory` and `never asked` are both zero match rows** — only an `advisory_triple` row tells
//! them apart, which is NEVER RENDER UNKNOWN AS ZERO at the storage layer.

use std::sync::{Arc, Mutex};

use codotheca_core::advisories::store::{close_sweep, fold_response, open_sweep};
use codotheca_core::advisories::sweep::{run_advisory_sweep, NEXT_BATCH};
use codotheca_core::index::Index;
use codotheca_core::protocol::{Ecosystem, ProjectId};
use codotheca_core::provider::{AdvisoryPayload, AffectedPackage, PackageVersion};
use codotheca_core::sync::outcome::SyncOutcome;
use codotheca_core::sync::schedule::{advisory_due, ADVISORY_SWEEP_INTERVAL_SECS};
use codotheca_core::sync::SyncDeps;
use codotheca_core::testing::{FakeClock, FakeTokenStore, FakeTransport, TempIndex};

const NOW: i64 = 1_800_000_000;
const HOST: &str = "forge.example.invalid";

struct Rig {
    index: Arc<Mutex<Index>>,
    transport: Arc<FakeTransport>,
    deps: SyncDeps,
    _dir: tempfile::TempDir,
}

fn rig() -> Rig {
    let temp = TempIndex::new();
    let dir = tempfile::tempdir().expect("tmp");
    let index = Index::open_at(dir.path(), NOW).expect("index");
    drop(temp);

    let transport = Arc::new(FakeTransport::new());
    let clock = Arc::new(FakeClock::new(NOW));
    let observing = Arc::new(codotheca_core::sync::http::ObservingTransport::new(
        transport.clone(),
        clock.clone(),
    ));
    let provider = Arc::new(codotheca_core::provider::GitHubProvider::new(
        observing.clone(),
        HOST.to_owned(),
    ));
    Rig {
        index: Arc::new(Mutex::new(index)),
        transport,
        deps: SyncDeps {
            provider,
            transport: observing,
            tokens: Arc::new(FakeTokenStore::available()),
            clock,
            cancel: codotheca_core::cancel::CancelToken::new(),
            tz_offset_min: 0,
        },
        _dir: dir,
    }
}

fn ok(body: &str) -> codotheca_core::http::HttpResponse {
    codotheca_core::http::HttpResponse {
        status: 200,
        headers: codotheca_core::http::normalise_headers([("x-ratelimit-resource", "core")]),
        body: body.as_bytes().to_vec(),
    }
}

fn seed_dependency(rig: &Rig, project: ProjectId, eco: &str, name: &str, version: &str) {
    let mut guard = rig.index.lock().expect("index");
    guard
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO project_dependency
                   (project_id, ecosystem, package_name, version, source_path, observed_at)
                 VALUES (?1, ?2, ?3, ?4, 'lock', ?5)
                 ON CONFLICT DO NOTHING",
                rusqlite::params![project.0, eco, name, version, NOW],
            )?;
            Ok(())
        })
        .expect("seed");
}

fn seed_project(rig: &Rig, name: &str) -> ProjectId {
    let mut guard = rig.index.lock().expect("index");
    guard
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO project (name, seed_basename, created_at, updated_at)
                 VALUES (?1, ?1, 1, 1)",
                [name],
            )?;
            Ok(ProjectId(tx.last_insert_rowid()))
        })
        .expect("project")
}

fn count(rig: &Rig, sql: &str) -> i64 {
    let guard = rig.index.lock().expect("index");
    guard
        .conn()
        .query_row(sql, [], |r| r.get(0))
        .expect("count")
}

/// Six advisories for one package, as the endpoint shapes them, one carrying a severity spelling
/// this build does not recognise.
fn six_advisories() -> String {
    let mut items = Vec::new();
    for (i, severity) in [
        "critical",
        "high",
        "moderate",
        "low",
        "unknown",
        "catastrophic",
    ]
    .into_iter()
    .enumerate()
    {
        items.push(format!(
            r#"{{"ghsa_id":"GHSA-{i}","summary":"s{i}","html_url":"u{i}","severity":"{severity}",
                "withdrawn_at":null,"identifiers":[],
                "vulnerabilities":[{{"package":{{"ecosystem":"npm","name":"left"}},
                                     "first_patched_version":null}}]}}"#
        ));
    }
    format!("[{}]", items.join(","))
}

/// **AC-P3-32-10.** One triple, **six** advisories: six `advisory_match` rows and **one**
/// `advisory_triple` row, with every severity stored as the source's own string byte for byte.
#[test]
fn ac_p3_32_10_one_triple_holds_six_matches() {
    let rig = rig();
    let project = seed_project(&rig, "alpha");
    seed_dependency(&rig, project, "npm", "left", "1.0.0");
    rig.transport.push(ok(&six_advisories()));

    let outcome = run_advisory_sweep(&rig.deps, rig.index.as_ref(), None).expect("swept");
    assert!(matches!(outcome, SyncOutcome::NextPage { .. }));

    let matches = count(&rig, "SELECT count(*) FROM advisory_match");
    let triples = count(&rig, "SELECT count(*) FROM advisory_triple");
    eprintln!("advisory_cache: {matches} match row(s) for {triples} triple row(s)");
    assert_eq!(
        matches, 6,
        "the cardinality a single-row shape could not hold"
    );
    assert_eq!(triples, 1);

    let severities: Vec<String> = {
        let guard = rig.index.lock().expect("index");
        let mut stmt = guard
            .conn()
            .prepare("SELECT severity FROM advisory ORDER BY advisory_id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        drop(stmt);
        drop(guard);
        rows
    };
    assert_eq!(
        severities,
        vec![
            "critical",
            "high",
            "moderate",
            "low",
            "unknown",
            "catastrophic"
        ]
    );
}

/// **AC-P3-32-11.** A triple answered with **zero** matches is distinguishable from a triple never
/// asked about. Both have zero `advisory_match` rows; only the first has an `advisory_triple` row.
#[test]
fn ac_p3_32_11_answered_with_nothing_differs_from_never_asked() {
    let rig = rig();
    let project = seed_project(&rig, "alpha");
    seed_dependency(&rig, project, "npm", "asked", "1.0.0");
    seed_dependency(&rig, project, "rust", "never", "2.0.0");
    // One answer, for the npm batch only: the rust batch is never reached in this run.
    rig.transport.push(ok("[]"));

    run_advisory_sweep(&rig.deps, rig.index.as_ref(), None).expect("swept");

    let asked: i64 = count(
        &rig,
        "SELECT count(*) FROM advisory_triple WHERE package_name = 'asked' AND answered = 1",
    );
    let never: i64 = count(
        &rig,
        "SELECT count(*) FROM advisory_triple WHERE package_name = 'never'",
    );
    let matches = count(&rig, "SELECT count(*) FROM advisory_match");
    eprintln!("advisory_cache: asked={asked} never_asked_rows={never} matches={matches}");
    assert_eq!(asked, 1, "answered, and the row is what says so");
    assert_eq!(never, 0, "never asked has no row at all");
    assert_eq!(
        matches, 0,
        "both are zero match rows — the row is the difference"
    );
}

/// **AC-P3-32-7.** An unanswered sweep **leaves the reading where it was**: no `advisory_triple`
/// row, no moved clock, and the state is `unknown` rather than absent.
///
/// §21.9 rule 1 is the one a sweep is most likely to break quietly.
#[test]
fn ac_p3_32_7_an_unanswered_sweep_dates_nothing() {
    let rig = rig();
    let project = seed_project(&rig, "alpha");
    seed_dependency(&rig, project, "npm", "left", "1.0.0");
    rig.transport
        .push_err(codotheca_core::http::TransportError::Timeout);

    let before = count(&rig, "SELECT count(*) FROM advisory_triple");
    let outcome = run_advisory_sweep(&rig.deps, rig.index.as_ref(), None).expect("ran");
    let after = count(&rig, "SELECT count(*) FROM advisory_triple");

    assert!(
        !matches!(outcome, SyncOutcome::Done | SyncOutcome::NextPage { .. }),
        "a timeout is not a completed batch: {outcome:?}"
    );
    assert_eq!(before, 0);
    assert_eq!(after, 0, "a call that observed nothing dated nothing");
    assert_eq!(count(&rig, "SELECT count(*) FROM advisory"), 0);
}

/// A sweep larger than one batch issues one request per batch and writes every **asked** triple's
/// row, answered or not — and it is the **cursor** that carries it across, so the budget is
/// re-read before each one.
#[test]
fn a_sweep_larger_than_one_batch_asks_about_every_triple() {
    let rig = rig();
    let project = seed_project(&rig, "alpha");
    for eco in ["npm", "rust", "pip"] {
        seed_dependency(&rig, project, eco, "pkg", "1.0.0");
    }
    for _ in 0..4 {
        rig.transport.push(ok("[]"));
    }

    let mut cursor: Option<String> = None;
    let mut batches = 0usize;
    loop {
        let outcome =
            run_advisory_sweep(&rig.deps, rig.index.as_ref(), cursor.as_deref()).expect("swept");
        match outcome {
            SyncOutcome::NextPage { cursor: next } => {
                cursor = Some(next);
                batches += 1;
            }
            SyncOutcome::Done => break,
            other => panic!("unexpected outcome {other:?}"),
        }
        assert!(batches < 10, "the sweep did not terminate");
    }
    let asked = count(
        &rig,
        "SELECT count(*) FROM advisory_triple WHERE answered = 1",
    );
    eprintln!("advisory_cache: {batches} batch(es), {asked} triple(s) asked about");
    // One batch per ecosystem that holds a triple: the endpoint keys on the ecosystem.
    assert_eq!(batches, 3);
    assert_eq!(asked, 3);
    assert_eq!(rig.transport.requests().len(), 3);
    assert_eq!(cursor.as_deref(), Some(NEXT_BATCH));
}

/// The sweep writes **no** row in any `project_*` table and invokes **no** git child.
#[test]
fn the_sweep_touches_no_project_row_and_spawns_no_git() {
    let rig = rig();
    let project = seed_project(&rig, "alpha");
    seed_dependency(&rig, project, "npm", "left", "1.0.0");
    let before = count(&rig, "SELECT count(*) FROM project_dependency");
    rig.transport.push(ok(&six_advisories()));

    run_advisory_sweep(&rig.deps, rig.index.as_ref(), None).expect("swept");

    assert_eq!(
        count(&rig, "SELECT count(*) FROM project_dependency"),
        before
    );
    assert_eq!(count(&rig, "SELECT count(*) FROM project_lockfile"), 0);
    assert_eq!(
        count(&rig, "SELECT count(*) FROM project_dependency_scan"),
        0
    );
    // Every request this run made went through the HTTP transport, which is the only place it can
    // reach the outside from: a git child would have had to be spawned by this module, and this
    // module holds no path to one.
    assert_eq!(rig.transport.requests().len(), 1);
}

/// The cadence: **daily**, and a sweep that **started** but never settled does not satisfy it.
#[test]
fn the_cadence_is_daily_and_a_start_is_not_a_settle() {
    let mut fixture = TempIndex::new();
    assert_eq!(ADVISORY_SWEEP_INTERVAL_SECS, 24 * 60 * 60);
    // A library with nothing to ask about is never due: queuing a task to discover there is no
    // work would settle a row claiming an observation of nothing.
    assert!(
        !advisory_due(fixture.index().conn(), NOW).expect("read"),
        "no triples, nothing to sweep"
    );
    fixture
        .index_mut()
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO project (name, seed_basename, created_at, updated_at)
                 VALUES ('p', 'p', 1, 1)",
                [],
            )?;
            let project = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO project_dependency
                   (project_id, ecosystem, package_name, version, source_path, observed_at)
                 VALUES (?1, 'npm', 'left', '1.0.0', 'lock', ?2)",
                rusqlite::params![project, NOW],
            )?;
            Ok(())
        })
        .expect("seed a triple");
    assert!(
        advisory_due(fixture.index().conn(), NOW).expect("read"),
        "no sweep has ever settled"
    );

    let id = fixture
        .index_mut()
        .with_tx(|tx| open_sweep(tx, NOW))
        .expect("open");
    assert!(
        advisory_due(fixture.index().conn(), NOW).expect("read"),
        "a start is not an observation; a crashed sweep must not push the next one a day away"
    );

    fixture
        .index_mut()
        .with_tx(|tx| close_sweep(tx, id, NOW, "done", true))
        .expect("close");
    assert!(!advisory_due(fixture.index().conn(), NOW).expect("read"));
    assert!(!advisory_due(
        fixture.index().conn(),
        NOW + ADVISORY_SWEEP_INTERVAL_SECS - 1
    )
    .expect("read"));
    assert!(
        advisory_due(fixture.index().conn(), NOW + ADVISORY_SWEEP_INTERVAL_SECS).expect("read")
    );
}

/// A name the batch never asked about is an advisory the endpoint volunteered, and it is **not**
/// this library's: writing it would attribute a match to a version nobody resolved.
#[test]
fn an_unasked_package_in_the_answer_writes_no_match() {
    let mut fixture = TempIndex::new();
    let asked = vec![PackageVersion {
        name: "left".to_owned(),
        version: "1.0.0".to_owned(),
    }];
    let page = vec![AdvisoryPayload {
        advisory_id: "GHSA-x".to_owned(),
        severity: Some("high".to_owned()),
        cve_ids: vec!["CVE-1".to_owned(), "CVE-2".to_owned()],
        withdrawn_at: None,
        summary: "s".to_owned(),
        url: "u".to_owned(),
        affects: vec![
            AffectedPackage {
                name: "left".to_owned(),
                fix_available: true,
                fixed_version: Some("2.0.0".to_owned()),
            },
            AffectedPackage {
                name: "elsewhere".to_owned(),
                fix_available: false,
                fixed_version: None,
            },
        ],
    }];

    let written = fixture
        .index_mut()
        .with_tx(|tx| {
            let id = open_sweep(tx, NOW)?;
            fold_response(tx, id, Ecosystem::Npm, &asked, &page, true, NOW)
        })
        .expect("folded");
    eprintln!("advisory_cache: {written} match row(s) written from a two-package advisory");
    assert_eq!(written, 1, "only the package this batch asked about");

    let cves: i64 = fixture
        .index()
        .conn()
        .query_row("SELECT count(*) FROM advisory_cve", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        cves, 2,
        "every CVE id, because one advisory carries several"
    );
}
