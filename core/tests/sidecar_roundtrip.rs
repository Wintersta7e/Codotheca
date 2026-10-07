#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §48.8: a library exported, corrupted, rebuilt and rediscovered by a real scan exports the same
//! sidecar again; the restore writes nothing the sidecar did not carry and announces nothing; and a
//! sidecar this build cannot trust either refuses the rebuild untouched or restores nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::clock::Clock;
use codotheca_core::completion::evaluate_and_write;
use codotheca_core::git::{ensure_empty_hooks_dir, GitBackend, GitExec, GitSlots, SystemGit};
use codotheca_core::index::migrate::SUPPORTED_SCHEMA_VERSION;
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildError, RebuildOutcome};
use codotheca_core::index::sidecar::{counts, read, Sidecar, BASE_COUNT_KEYS, SECTIONS};
use codotheca_core::index::Index;
use codotheca_core::jobs::NullJobSink;
use codotheca_core::mount::SystemMountResolver;
use codotheca_core::protocol::{ProjectId, ScanMode};
use codotheca_core::scan::launcher::{ScanLauncherDeps, ThreadScanLauncher};
use codotheca_core::scan::skiplist::SkipList;
use codotheca_core::scan::store::SqliteScanStore;
use codotheca_core::scan::ScanSupervisor;
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::sidecar::build_library;
use codotheca_core::testing::FakeClock;

const NOW: i64 = 1_760_000_000;

/// A walk over three small repositories, on a loaded machine.
const DEADLINE: Duration = Duration::from_secs(120);

/// What one export, rebuild and rediscovery left behind.
struct RoundTrip {
    /// The sidecar the library wrote before its index was corrupted.
    before: Sidecar,
    /// The sidecar the rebuilt, rediscovered index writes.
    after: Sidecar,
    /// Every event the rediscovering scan emitted.
    events: Arc<ValidatingSink>,
    /// The session-track XP rows in the rebuilt index, by `dedupe_key`.
    session_xp: BTreeSet<String>,
}

/// Build, export (E1), corrupt, rebuild, rediscover through a real scan, evaluate each project
/// the scan found as its settle hook would, and export again (E2).
fn round_trip() -> RoundTrip {
    let data = tempfile::tempdir().unwrap();
    let repos = tempfile::tempdir().unwrap();
    let library = build_library(data.path(), repos.path(), NOW).unwrap();
    let exported = library.index.export_sidecar(NOW + 10).unwrap();
    let before = read(&exported.path).unwrap();
    drop(library);
    std::fs::write(Index::db_path(data.path()), b"this is not a database").unwrap();
    match rebuild_in_place(data.path(), NOW + 20) {
        Ok(RebuildOutcome::Rebuilt(report)) => eprintln!(
            "rebuilt: restored {:?}, {} pending",
            report.restored, report.pending
        ),
        other => panic!("expected a rebuild, got {other:?}"),
    }

    let index = Arc::new(Mutex::new(Index::open_at(data.path(), NOW + 30).unwrap()));
    let events = Arc::new(ValidatingSink::default());
    rediscover(&index, &events, data.path());
    let found: Vec<i64> = index
        .lock()
        .unwrap()
        .conn()
        .prepare("SELECT DISTINCT project_id FROM location WHERE removed_at IS NULL")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for project in found {
        let written = index
            .lock()
            .unwrap()
            .with_tx(|tx| {
                // The authorship job's write, which a settle hook's evaluation follows; the scan
                // here queues its jobs into nothing.
                tx.execute(
                    "UPDATE project SET authored_by_user = 1 WHERE id = ?1",
                    [project],
                )?;
                evaluate_and_write(tx, ProjectId(project), NOW + 40)
            })
            .unwrap();
        eprintln!("evaluated project {project}: {written:?}");
    }

    let guard = index.lock().unwrap();
    let after = read(&guard.export_sidecar(NOW + 50).unwrap().path).unwrap();
    let session_xp = guard
        .conn()
        .prepare("SELECT dedupe_key FROM xp_events WHERE track = 'session'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(guard);
    RoundTrip {
        before,
        after,
        events,
        session_xp,
    }
}

/// A full scan of the restored root, its events into `events` and its jobs into nothing: the
/// hand-off it runs for each repository is what matches the pending records.
fn rediscover(index: &Arc<Mutex<Index>>, events: &Arc<ValidatingSink>, data: &Path) {
    let clock: Arc<dyn Clock> = Arc::new(FakeClock::new(NOW + 30));
    let git: Arc<dyn GitBackend> = Arc::new(SystemGit::new(
        Arc::new(GitExec::system(ensure_empty_hooks_dir(data).unwrap())),
        Arc::new(GitSlots::for_machine()),
        Arc::clone(&clock),
    ));
    let scans = ScanSupervisor::new(Arc::new(ThreadScanLauncher::new(ScanLauncherDeps {
        store: Arc::new(SqliteScanStore::new(Arc::clone(index))),
        index: Arc::clone(index),
        git,
        mounts: Arc::new(SystemMountResolver::new()),
        clock,
        skip: Arc::new(SkipList::default()),
        wsl: None,
        jobs: Arc::new(NullJobSink),
        events: events.clone(),
    })));
    scans.start(ScanMode::Full, NOW + 30).unwrap();
    let deadline = Instant::now() + DEADLINE;
    while scans.live().is_some() {
        assert!(Instant::now() < deadline, "the scan never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// `value` with every list sorted by its rows' text. The export lists rows in id order, and a
/// rebuild assigns ids anew, so order is not part of what a round trip keeps.
fn canonical(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(rows) => {
            let mut rows: Vec<_> = rows.into_iter().map(canonical).collect();
            rows.sort_by_cached_key(ToString::to_string);
            serde_json::Value::Array(rows)
        }
        serde_json::Value::Object(map) => {
            serde_json::Value::Object(map.into_iter().map(|(k, v)| (k, canonical(v))).collect())
        }
        other => other,
    }
}

/// Each top-level key of the payload, and each section on its own, as canonical rows; a map's
/// entries are rows of their own.
fn by_key(doc: &Sidecar) -> BTreeMap<String, Vec<serde_json::Value>> {
    let payload = canonical(serde_json::to_value(&doc.payload).unwrap());
    let mut out = BTreeMap::new();
    for (key, value) in payload.as_object().unwrap() {
        let mut put = |name: String, held: &serde_json::Value| {
            let rows = match held {
                serde_json::Value::Array(rows) => rows.clone(),
                serde_json::Value::Object(map) => map
                    .iter()
                    .map(|(k, v)| serde_json::json!({ "key": k, "value": v }))
                    .collect(),
                other => vec![other.clone()],
            };
            out.insert(name, rows);
        };
        if key == "sections" {
            for (section, rows) in value.as_object().unwrap() {
                put(format!("sections.{section}"), rows);
            }
        } else {
            put(key.clone(), value);
        }
    }
    out
}

/// How `before`'s rows and `after`'s differ under `key`. Rows that name a subject are paired by
/// it and named by field, so a value the restore dropped is named rather than buried in a row.
fn differences(
    key: &str,
    before: &[serde_json::Value],
    after: &[serde_json::Value],
) -> Vec<String> {
    let lost: Vec<_> = before.iter().filter(|row| !after.contains(row)).collect();
    let gained: Vec<_> = after.iter().filter(|row| !before.contains(row)).collect();
    let mut out = Vec::new();
    for row in &lost {
        let twin = gained.iter().find(|other| {
            row.get("subject").is_some() && other.get("subject") == row.get("subject")
        });
        match (row.as_object(), twin.and_then(|t| t.as_object())) {
            (Some(was), Some(now)) => {
                for (field, value) in was {
                    if now.get(field) != Some(value) {
                        out.push(format!(
                            "{key}[{}].{field}: {value} became {}",
                            row["subject"],
                            now.get(field).unwrap_or(&serde_json::Value::Null)
                        ));
                    }
                }
            }
            _ => out.push(format!("{key}: lost {row}")),
        }
    }
    for row in gained {
        if !lost
            .iter()
            .any(|was| was.get("subject").is_some() && was.get("subject") == row.get("subject"))
        {
            out.push(format!("{key}: gained {row}"));
        }
    }
    out
}

/// AC-P4-48-16: every key of the sidecar, every registered section among them, survives the
/// export, the rebuild and the rediscovery unchanged, but for `generation` and `written_at`.
#[test]
fn ac_p4_48_16_sections_round_trip() {
    let trip = round_trip();
    let populated = counts(&trip.before);
    let expected: Vec<&str> = BASE_COUNT_KEYS
        .iter()
        .copied()
        .filter(|key| !["merges", "pending"].contains(key))
        .chain(SECTIONS.iter().map(|s| s.name))
        .collect();
    for key in &expected {
        eprintln!("the library exports {} {key}", populated[*key]);
        assert_ne!(populated[*key], 0, "the library populates no {key}");
    }

    let (before, after) = (by_key(&trip.before), by_key(&trip.after));
    let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let mut compared = 0;
    let mut differ = Vec::new();
    for key in keys {
        let (b, a) = (
            before.get(key).cloned().unwrap_or_default(),
            after.get(key).cloned().unwrap_or_default(),
        );
        eprintln!("{key}: {} compared", b.len());
        compared += b.len();
        differ.extend(differences(key, &b, &a));
    }
    for line in &differ {
        eprintln!("{line}");
    }
    eprintln!("rows compared: {compared}");
    assert_ne!(compared, 0, "a round trip of nothing proves nothing");
    assert_eq!(differ, Vec::<String>::new(), "the round trip changed");
}

/// AC-P4-48-19, its ledger half: the rebuilt index holds exactly the session-track XP rows the
/// sidecar carried, and the restore announces nothing. The rebuild takes no event sink at all;
/// the scan whose hand-offs match every record publishes through a validating one, and it heard
/// nothing but the scan's own topic.
#[test]
fn ac_p4_48_19_a_restore_writes_no_unexported_row_and_announces_nothing() {
    let trip = round_trip();
    let exported: BTreeSet<String> = trip
        .before
        .payload
        .projects
        .iter()
        .flat_map(|p| p.xp_events.iter().map(|e| e.dedupe_key.clone()))
        .collect();
    eprintln!(
        "session-track XP rows: {} exported, {} after the restore",
        exported.len(),
        trip.session_xp.len()
    );
    assert_ne!(exported.len(), 0, "the library exported no XP row");
    assert_eq!(trip.session_xp, exported);

    let heard = trip.events.events.lock().unwrap().clone();
    let elsewhere: Vec<String> = heard
        .iter()
        .filter(|(topic, _, _)| topic != "scan")
        .map(|(topic, event, _)| format!("{topic}/{event}"))
        .collect();
    eprintln!(
        "events heard: {}, outside the scan's topic: {}",
        heard.len(),
        elsewhere.len()
    );
    assert_ne!(
        heard.len(),
        0,
        "the sink heard nothing, so it proves nothing"
    );
    assert_eq!(elsewhere, Vec::<String>::new());
}

/// Every file under `dir`, by its path below it, with its bytes; a directory with none.
fn fingerprint(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            let name = path.strip_prefix(dir).unwrap().display().to_string();
            if path.is_dir() {
                out.insert(name, Vec::new());
                stack.push(path);
            } else {
                out.insert(name, std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// AC-P4-48-20: a sidecar from a newer build — a newer schema, an unknown format, an unknown
/// section — refuses the rebuild and leaves the directory byte-identical; one whose checksum fails
/// rebuilds with nothing restored, and is kept in the quarantine set.
#[test]
fn ac_p4_48_20_newer_and_unreadable_sidecars_restore_nothing() {
    type Tamper = fn(&mut serde_json::Value);
    let cases: [(&str, Tamper); 4] = [
        ("newer schema", |doc| {
            doc["schema_version"] = serde_json::json!(SUPPORTED_SCHEMA_VERSION + 1);
        }),
        ("unknown format", |doc| doc["format"] = serde_json::json!(3)),
        ("unknown section", |doc| {
            doc["payload"]["sections"]["a_section_no_build_registers"] = serde_json::json!([]);
        }),
        ("checksum mismatch", |doc| {
            let note = doc["payload"]["projects"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|p| p["notes"] == "a note the user wrote")
                .unwrap();
            note["notes"] = serde_json::json!("a NOTE the user wrote");
        }),
    ];

    let mut held = 0;
    for (case, tamper) in cases {
        let data = tempfile::tempdir().unwrap();
        let repos = tempfile::tempdir().unwrap();
        let library = build_library(data.path(), repos.path(), NOW).unwrap();
        let sidecar = library.index.export_sidecar(NOW + 10).unwrap().path;
        drop(library);
        std::fs::write(Index::db_path(data.path()), b"this is not a database").unwrap();
        let mut doc: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
        tamper(&mut doc);
        let tampered = serde_json::to_vec(&doc).unwrap();
        std::fs::write(&sidecar, &tampered).unwrap();
        let untouched = fingerprint(data.path());

        match rebuild_in_place(data.path(), NOW + 20) {
            Err(RebuildError::SidecarNewer { reason }) if case != "checksum mismatch" => {
                eprintln!("{case}: refused ({reason})");
                assert_eq!(
                    fingerprint(data.path()),
                    untouched,
                    "{case} moved something"
                );
            }
            Ok(RebuildOutcome::Rebuilt(report)) if case == "checksum mismatch" => {
                eprintln!(
                    "{case}: rebuilt, restored {:?}, quarantined {:?}",
                    report.restored, report.quarantine_files
                );
                assert_eq!(report.restored, BTreeMap::new(), "{case} restored rows");
                assert_eq!(report.pending, 0);
                let copy = report
                    .quarantine_files
                    .iter()
                    .find(|f| f.contains("sidecar"))
                    .unwrap_or_else(|| panic!("{case}: no sidecar in the quarantine set"));
                assert_eq!(std::fs::read(copy).unwrap(), tampered);
            }
            other => panic!("{case} answered {other:?}"),
        }
        held += 1;
    }
    eprintln!("cases held: {held}");
    assert_eq!(held, 4);
}
