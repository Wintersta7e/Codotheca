#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! What a **completed advisory sweep** changes, driven through the real `SyncRunner` with the
//! forge's answers scripted on the transport.
//!
//! The item producer and the ledger seed each had their tests and no production caller, so in the
//! product no `dependency_advisory` item ever opened or closed, and a project's first computation
//! could interrupt the user. Everything below runs the sweep the runner runs: batches asked,
//! answers folded, and the close computing each project's items.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::advisories::lockfiles::read_lockfiles;
use codotheca_core::http::HttpResponse;
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
const DAY: i64 = 86_400;
const HOST: &str = "forge.example.invalid";
const DEADLINE: Duration = Duration::from_secs(10);

/// One emitted event: `(topic, event, payload)`.
type Emitted = (String, String, serde_json::Value);

/// Every event the runner emits, kept so a test can find the ones it asserts on.
#[derive(Debug, Default)]
struct Recorder(Mutex<Vec<Emitted>>);

impl codotheca_core::proto::EventSink for Recorder {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        self.0
            .lock()
            .unwrap()
            .push((topic.to_owned(), event.to_owned(), payload));
    }
}

/// The payloads of every `topic/event` in `emitted`.
fn payloads(emitted: &[Emitted], topic: &str, event: &str) -> Vec<serde_json::Value> {
    emitted
        .iter()
        .filter(|(t, e, _)| t == topic && e == event)
        .map(|(_, _, payload)| payload.clone())
        .collect()
}

fn alerts(emitted: &[Emitted]) -> Vec<serde_json::Value> {
    payloads(emitted, "sync", "advisory_alert")
}

/// One project and the working copy its lockfile is read from.
struct Copy {
    project: ProjectId,
    work: tempfile::TempDir,
}

struct World {
    index: Arc<Mutex<Index>>,
    transport: Arc<FakeTransport>,
    alpha: Copy,
    _dir: tempfile::TempDir,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open_at(dir.path(), NOW).unwrap()));
    let alpha = copy(&index, "alpha");
    World {
        index,
        transport: Arc::new(FakeTransport::new()),
        alpha,
        _dir: dir,
    }
}

/// One installed, enrolled, authored project with a lineage key — the ledger is keyed on the
/// subject, and an enrolled authored project has a health reading for a delta to move.
fn copy(index: &Mutex<Index>, name: &str) -> Copy {
    let work = tempfile::tempdir().unwrap();
    let mut guard = index.lock().unwrap();
    let project = guard
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO project (name, seed_basename, lineage_key, acknowledged_at,
                                      authored_by_user, is_reference, created_at, updated_at)
                 VALUES (?1, ?1, ?1, ?2, 1, 0, 1, 1)",
                rusqlite::params![name, NOW],
            )?;
            let project = ProjectId(tx.last_insert_rowid());
            let path = work.path().to_string_lossy().into_owned();
            tx.execute(
                "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                       store_key, presence, repo_kind)
                 VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
                rusqlite::params![project.0, path.as_bytes(), path],
            )?;
            Ok(project)
        })
        .unwrap();
    Copy { project, work }
}

/// The working copy resolves `left` at `version`, and the read that J6 runs has seen it.
fn lock(world: &World, copy: &Copy, version: &str, at: i64) {
    std::fs::write(
        copy.work.path().join("package-lock.json"),
        format!(
            r#"{{"lockfileVersion":3,"packages":{{"":{{}},"node_modules/left":{{"version":"{version}"}}}}}}"#
        ),
    )
    .unwrap();
    let mut guard = world.index.lock().unwrap();
    guard
        .with_tx(|tx| {
            read_lockfiles(tx, copy.project, copy.work.path(), at).unwrap();
            Ok(())
        })
        .unwrap();
}

/// One critical advisory against `left`, fixed in 2.0.0, as the endpoint shapes it.
fn advisory(id: &str, cve: &str, withdrawn_at: Option<&str>) -> String {
    let withdrawn = withdrawn_at.map_or_else(|| "null".to_owned(), |w| format!("\"{w}\""));
    format!(
        r#"{{"ghsa_id":"{id}","cve_id":"{cve}","summary":"s","html_url":"u","severity":"critical",
            "withdrawn_at":{withdrawn},
            "identifiers":[{{"value":"{id}","type":"GHSA"}},{{"value":"{cve}","type":"CVE"}}],
            "vulnerabilities":[{{"package":{{"ecosystem":"npm","name":"left"}},
              "vulnerable_version_range":"< 2.0.0","first_patched_version":"2.0.0"}}]}}"#
    )
}

fn answer(json: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: codotheca_core::http::normalise_headers([("x-ratelimit-resource", "core")]),
        body: json.as_bytes().to_vec(),
    }
}

/// Queue the sweep, script the answer if the sweep will ask, run the real runner at `at` until
/// the sweep settles, and return every event it emitted.
fn sweep(world: &World, at: i64, response: Option<&str>) -> Vec<Emitted> {
    {
        let mut guard = world.index.lock().unwrap();
        guard
            .with_tx(|tx| {
                put(
                    tx,
                    &SyncTaskStateRow::queued(SyncTaskKind::Advisories, None, at),
                )
            })
            .unwrap();
    }
    if let Some(response) = response {
        world.transport.push(answer(response));
    }

    let clock = Arc::new(FakeClock::new(at));
    let observing = Arc::new(ObservingTransport::new(
        Arc::clone(&world.transport) as Arc<dyn codotheca_core::http::HttpTransport>,
        Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
    ));
    let provider = Arc::new(GitHubProvider::new(
        Arc::clone(&observing) as Arc<dyn codotheca_core::http::HttpTransport>,
        HOST.to_owned(),
    ));
    let deps = SyncDeps {
        provider,
        transport: observing,
        tokens: Arc::new(FakeTokenStore::available()),
        clock: Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
        cancel: codotheca_core::cancel::CancelToken::new(),
        tz_offset_min: 0,
    };
    let events = Arc::new(Recorder::default());
    let runner = SyncRunner::new(
        Arc::clone(&world.index),
        deps,
        Arc::clone(&events) as Arc<dyn codotheca_core::proto::EventSink>,
    );
    runner.start();
    let deadline = Instant::now() + DEADLINE;
    loop {
        let state = {
            let guard = world.index.lock().unwrap();
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

    let emitted = events.0.lock().unwrap().clone();
    emitted
}

/// Every `dependency_advisory` item: `(fingerprint, state, scoring)`.
fn items(world: &World) -> Vec<(String, String, String)> {
    let guard = world.index.lock().unwrap();
    let mut stmt = guard
        .conn()
        .prepare(
            "SELECT fingerprint, state, scoring FROM debt_item
              WHERE source = 'dependency_advisory' ORDER BY fingerprint",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The XP ledger **by identity**, never by count.
fn ledger(world: &World) -> BTreeSet<String> {
    let guard = world.index.lock().unwrap();
    let mut stmt = guard
        .conn()
        .prepare("SELECT dedupe_key FROM xp_events")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The notified ledger: `(project_id, advisory_id, seeded)`.
fn notified(world: &World) -> Vec<(i64, String, i64)> {
    let guard = world.index.lock().unwrap();
    let mut stmt = guard
        .conn()
        .prepare(
            "SELECT project_id, advisory_id, seeded FROM advisory_notified
              ORDER BY project_id, advisory_id",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// A matching advisory **opens** a scored item at the sweep's close, and an upgrade that removes
/// the match **closes** it `fixed` at the next one — paying the day's row for a fix the user made.
#[test]
fn a_sweep_opens_an_advisory_item_and_an_upgrade_closes_it() {
    let world = world();
    lock(&world, &world.alpha, "1.0.0", NOW);
    sweep(
        &world,
        NOW,
        Some(&format!(
            "[{}]",
            advisory("GHSA-aaaa", "CVE-2026-0001", None)
        )),
    );
    let opened = items(&world);
    eprintln!("advisory_sweep_settle: after the first sweep {opened:?}");
    assert_eq!(
        opened,
        vec![(
            "npm:left:GHSA-aaaa".to_owned(),
            "open".to_owned(),
            "scored".to_owned()
        )],
        "a matching advisory opened no item"
    );
    let before = ledger(&world);

    // The user upgrades past the fix, the working copy is re-read, and the forge has nothing to
    // say about the new version.
    lock(&world, &world.alpha, "2.0.0", NOW + DAY);
    sweep(&world, NOW + DAY, Some("[]"));
    let after = ledger(&world);
    let paid: Vec<&String> = after.difference(&before).collect();
    eprintln!(
        "advisory_sweep_settle: after the upgrade {:?}, ledger gained {paid:?}",
        items(&world)
    );
    assert!(items(&world).is_empty(), "the upgrade closed nothing");
    assert_eq!(paid.len(), 1, "a fix the user made is paid once");
    assert!(paid[0].starts_with("debt_day:"), "{paid:?}");
}

/// A **withdrawn** advisory closes its item as `invalidated` (R135): the item goes and the XP
/// ledger's row set is unchanged, by identity.
#[test]
fn a_withdrawn_advisory_closes_its_item_without_paying() {
    let world = world();
    lock(&world, &world.alpha, "1.0.0", NOW);
    sweep(
        &world,
        NOW,
        Some(&format!(
            "[{}]",
            advisory("GHSA-aaaa", "CVE-2026-0001", None)
        )),
    );
    assert_eq!(items(&world).len(), 1, "a matching advisory opened no item");
    let before = ledger(&world);

    sweep(
        &world,
        NOW + DAY,
        Some(&format!(
            "[{}]",
            advisory("GHSA-aaaa", "CVE-2026-0001", Some("2027-01-02T03:04:05Z"))
        )),
    );
    eprintln!(
        "advisory_sweep_settle: after the withdrawal {:?}, ledger {} -> {}",
        items(&world),
        before.len(),
        ledger(&world).len()
    );
    assert!(items(&world).is_empty(), "the withdrawal closed nothing");
    assert_eq!(
        ledger(&world),
        before,
        "a withdrawal paid for a fix the user did not make"
    );
}

/// **§32.12 rule 3.** The first sweep that matches a critical, fixable advisory **seeds** the
/// ledger and fires nothing; a **new** advisory at the next sweep fires exactly once.
#[test]
fn a_first_computation_seeds_and_a_new_advisory_fires_once() {
    let world = world();
    let alpha = world.alpha.project.0;
    lock(&world, &world.alpha, "1.0.0", NOW);
    let first = alerts(&sweep(
        &world,
        NOW,
        Some(&format!(
            "[{}]",
            advisory("GHSA-aaaa", "CVE-2026-0001", None)
        )),
    ));
    eprintln!(
        "advisory_sweep_settle: first sweep alerts {first:?}, ledger {:?}",
        notified(&world)
    );
    assert!(first.is_empty(), "the first computation toasted: {first:?}");
    assert_eq!(
        notified(&world),
        vec![(alpha, "GHSA-aaaa".to_owned(), 1)],
        "the first computation seeded nothing"
    );

    let second = alerts(&sweep(
        &world,
        NOW + DAY,
        Some(&format!(
            "[{},{}]",
            advisory("GHSA-aaaa", "CVE-2026-0001", None),
            advisory("GHSA-bbbb", "CVE-2026-0002", None)
        )),
    ));
    eprintln!("advisory_sweep_settle: second sweep alerts {second:?}");
    assert_eq!(second.len(), 1, "a new advisory fires exactly once");
    assert_eq!(second[0]["advisoryId"], "GHSA-bbbb");
    assert_eq!(second[0]["projectId"], alpha);
    assert_eq!(
        notified(&world),
        vec![
            (alpha, "GHSA-aaaa".to_owned(), 1),
            (alpha, "GHSA-bbbb".to_owned(), 0)
        ]
    );
}

/// A project whose lockfile names a triple **already answered**, first computed by a sweep close
/// that asks nothing new, is seeded at that close and never toasts. No settle runs between its
/// lockfile read and its computation, so only the close can seed it.
#[test]
fn a_project_first_computed_at_a_close_that_asks_nothing_is_seeded() {
    let world = world();
    lock(&world, &world.alpha, "1.0.0", NOW);
    sweep(
        &world,
        NOW,
        Some(&format!(
            "[{}]",
            advisory("GHSA-aaaa", "CVE-2026-0001", None)
        )),
    );

    // A second copy resolves the same triple. The next sweep has already folded its last batch,
    // so its next pick is the close.
    let beta = copy(&world.index, "beta");
    lock(&world, &beta, "1.0.0", NOW + DAY);
    {
        let mut guard = world.index.lock().unwrap();
        guard
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO advisory_sweep (started_at, complete) VALUES (?1, 0)",
                    [NOW + DAY],
                )?;
                let open = tx.last_insert_rowid();
                tx.execute("UPDATE advisory_triple SET sweep_id = ?1", [open])?;
                Ok(())
            })
            .unwrap();
    }
    let fired = alerts(&sweep(&world, NOW + DAY, None));
    eprintln!(
        "advisory_sweep_settle: close-only sweep alerts {fired:?}, ledger {:?}",
        notified(&world)
    );
    assert!(
        fired.is_empty(),
        "a first computation at the close toasted: {fired:?}"
    );
    assert!(
        notified(&world).contains(&(beta.project.0, "GHSA-aaaa".to_owned(), 1)),
        "the close did not seed the new copy's first computation"
    );
}

/// An advisory read that **cannot observe** marks the project's **advisory** items `unverified`
/// and no other source's. A TODO item is evidence the advisory read never looked at, so it stays
/// `open` and stays counted.
#[test]
fn an_unobservable_advisory_read_marks_only_advisory_items() {
    let world = world();
    let alpha = world.alpha.project;
    lock(&world, &world.alpha, "1.0.0", NOW);
    sweep(
        &world,
        NOW,
        Some(&format!(
            "[{}]",
            advisory("GHSA-aaaa", "CVE-2026-0001", None)
        )),
    );
    {
        let guard = world.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state,
                                        scoring, first_seen_at, last_seen_at)
                 VALUES (?1, 'lineage:alpha|remote:', 'todo_marker', 'todo', 'open', 'scored',
                         ?2, ?2)",
                rusqlite::params![alpha.0, NOW],
            )
            .unwrap();
    }

    // A manifest of an ecosystem this build has no parser for: the read runs and cannot resolve
    // it, so the verdict is `unknown` and the advisory items become unobservable.
    std::fs::write(world.alpha.work.path().join("go.mod"), "module x\n").unwrap();
    lock(&world, &world.alpha, "1.0.0", NOW + DAY);
    sweep(
        &world,
        NOW + DAY,
        Some(&format!(
            "[{}]",
            advisory("GHSA-aaaa", "CVE-2026-0001", None)
        )),
    );

    let states: Vec<(String, String)> = {
        let guard = world.index.lock().unwrap();
        let mut stmt = guard
            .conn()
            .prepare("SELECT source, state FROM debt_item WHERE project_id = ?1 ORDER BY source")
            .unwrap();
        stmt.query_map([alpha.0], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    eprintln!("advisory_sweep_settle: after an unobservable read {states:?}");
    assert_eq!(
        states,
        vec![
            ("dependency_advisory".to_owned(), "unverified".to_owned()),
            ("todo_marker".to_owned(), "open".to_owned()),
        ],
        "an advisory read that could not observe froze another source's item"
    );
}

/// Every `health_delta` row: `(layer, from_value, to_value, detected_in)`.
fn delta_rows(world: &World) -> Vec<(String, f64, f64, String)> {
    let guard = world.index.lock().unwrap();
    let mut stmt = guard
        .conn()
        .prepare("SELECT layer, from_value, to_value, detected_in FROM health_delta ORDER BY id")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// **A15 for the advisory items.** A fixed advisory's close writes one `rust` decrease, observed
/// in the `background`, and announces it once on `projects/health_delta` after the commit. The
/// item's opening is a first observation and writes nothing.
#[test]
fn a_fixed_advisory_writes_one_rust_decrease_and_announces_it() {
    let world = world();
    lock(&world, &world.alpha, "1.0.0", NOW);
    let opened = sweep(
        &world,
        NOW,
        Some(&format!(
            "[{}]",
            advisory("GHSA-aaaa", "CVE-2026-0001", None)
        )),
    );
    assert_eq!(items(&world).len(), 1, "a matching advisory opened no item");
    assert!(
        delta_rows(&world).is_empty(),
        "a first observation wrote a row"
    );
    assert!(payloads(&opened, "projects", "health_delta").is_empty());

    lock(&world, &world.alpha, "2.0.0", NOW + DAY);
    let closed = sweep(&world, NOW + DAY, Some("[]"));
    let rows = delta_rows(&world);
    let announced = payloads(&closed, "projects", "health_delta");
    eprintln!("advisory_sweep_settle: fixed close rows {rows:?}, announced {announced:?}");
    assert_eq!(
        rows,
        vec![("rust".to_owned(), 1.0, 0.0, "background".to_owned())],
        "the fixed close wrote no rust decrease"
    );
    assert_eq!(announced.len(), 1, "the decrease was not announced once");
    assert_eq!(announced[0]["id"], world.alpha.project.0);
    assert_eq!(announced[0]["layers"][0]["layer"], "rust");
}

/// **R135.** A withdrawn advisory's close writes its `rust` row — the value did move — and
/// announces nothing: a third party changing its mind is not a restoration.
#[test]
fn a_withdrawn_advisory_writes_its_row_and_announces_nothing() {
    let world = world();
    lock(&world, &world.alpha, "1.0.0", NOW);
    sweep(
        &world,
        NOW,
        Some(&format!(
            "[{}]",
            advisory("GHSA-aaaa", "CVE-2026-0001", None)
        )),
    );
    assert_eq!(items(&world).len(), 1, "a matching advisory opened no item");

    let withdrawn = sweep(
        &world,
        NOW + DAY,
        Some(&format!(
            "[{}]",
            advisory("GHSA-aaaa", "CVE-2026-0001", Some("2027-01-02T03:04:05Z"))
        )),
    );
    let rows = delta_rows(&world);
    let announced = payloads(&withdrawn, "projects", "health_delta");
    eprintln!("advisory_sweep_settle: withdrawn close rows {rows:?}, announced {announced:?}");
    assert_eq!(
        rows,
        vec![("rust".to_owned(), 1.0, 0.0, "background".to_owned())],
        "the withdrawal's decrease was not written"
    );
    assert!(
        announced.is_empty(),
        "a withdrawal was announced as a restoration: {announced:?}"
    );
}
