#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! **R40: the production `ScanStore`.** Every scan write goes through the trait, and until this
//! landed the only implementation anywhere was the in-memory double — the third instance of
//! "a seam every plan writes through, with only a test implementation behind it", after R1 and
//! R35(a). Plan 07 declares the trait, so plan 07 owns the real one.
//!
//! **R35(a) closed the one hole this file used to pin.** `upsert_location` was a refusal on the
//! trait, naming plan 08 as the owner of the `location` writer; the method is gone from the
//! trait entirely, and `assembly::handoff` calls plan 08's writer inside the transaction
//! `resolve_identity` holds. `core/tests/handoff_identity.rs` asserts that writer's idempotence
//! against the real database.

use codotheca_core::index::path::{PathPlatform, StoredPath};
use codotheca_core::index::Index;
use codotheca_core::scan::presence::{
    apply_presence, Presence, PresenceContext, ScanRunFinish, ScanRunStart, ScanStore,
};
use codotheca_core::scan::skiplist::SkipList;
use codotheca_core::scan::store::SqliteScanStore;
use codotheca_core::scan::{ScanProblem, ScanProblemKind};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

struct Harness {
    _dir: tempfile::TempDir,
    store: SqliteScanStore,
    index: Arc<Mutex<Index>>,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open(dir.path()).unwrap()));
    Harness {
        _dir: dir,
        store: SqliteScanStore::new(Arc::clone(&index)),
        index,
    }
}

fn seed_root(h: &Harness, path: &str, enabled: bool, descend: bool) {
    let stored = StoredPath::from_bytes(path.as_bytes().to_vec(), PathPlatform::Unix);
    let (bytes, key, display) = stored.as_params();
    h.index
        .lock()
        .unwrap()
        .conn()
        .execute(
            "INSERT INTO scan_root (kind, distro, path_bytes, path_key, path_display,
                                    enabled, added_by, descend_into_repos, added_at)
             VALUES ('linux', '', ?1, ?2, ?3, ?4, 'user', ?5, 1)",
            rusqlite::params![bytes, key, display, i64::from(enabled), i64::from(descend)],
        )
        .unwrap();
}

/// One project with one location, written directly — plan 08 owns the real writer.
fn seed_location(h: &Harness, path: &str, store_key: &str, generation: i64) -> i64 {
    let conn_guard = h.index.lock().unwrap();
    let conn = conn_guard.conn();
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES ('p', 'p', 1, 1)",
        [],
    )
    .unwrap();
    let project_id = conn.last_insert_rowid();
    let stored = StoredPath::from_bytes(path.as_bytes().to_vec(), PathPlatform::Unix);
    let (bytes, key, display) = stored.as_params();
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                               store_key, presence, repo_kind, scan_generation)
         VALUES (?1, 'linux', ?2, ?3, ?4, ?5, 'present', 'worktree', ?6)",
        rusqlite::params![project_id, bytes, key, display, store_key, generation],
    )
    .unwrap();
    conn.last_insert_rowid()
}

#[test]
fn generations_are_monotonic_across_runs() {
    let h = harness();
    assert_eq!(h.store.next_generation().unwrap(), 1);
    h.store
        .begin_scan_run(&ScanRunStart {
            generation: 1,
            started_at: 100,
            mode: "full",
            roots_json: "[]".to_owned(),
        })
        .unwrap();
    assert_eq!(h.store.next_generation().unwrap(), 2);
}

/// A generation already stamped on a `location` row must never be reissued: two runs sharing one
/// generation would let the later mark the earlier's locations missing.
#[test]
fn a_generation_already_on_a_location_is_never_reissued() {
    let h = harness();
    seed_location(&h, "/r/a", "s1", 42);
    assert_eq!(h.store.next_generation().unwrap(), 43);
}

#[test]
fn roots_come_back_with_their_flags_in_id_order() {
    let h = harness();
    seed_root(&h, "/r/one", true, false);
    seed_root(&h, "/r/two", false, true);
    let roots = h.store.scan_roots().unwrap();
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0].path_key, b"/r/one");
    assert!(roots[0].enabled);
    assert!(!roots[0].descend_into_repos);
    assert_eq!(roots[1].kind, "linux");
    assert!(!roots[1].enabled);
    assert!(roots[1].descend_into_repos);
}

#[test]
fn a_run_records_its_final_counters_only_when_it_finishes() {
    let h = harness();
    let id = h
        .store
        .begin_scan_run(&ScanRunStart {
            generation: 1,
            started_at: 100,
            mode: "incremental",
            roots_json: "[{\"id\":1,\"enabled\":true}]".to_owned(),
        })
        .unwrap();

    let mid: (i64, Option<i64>, i64) = h
        .index
        .lock()
        .unwrap()
        .conn()
        .query_row(
            "SELECT walked_dirs, ended_at, cancelled FROM scan_run WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        mid,
        (0, None, 0),
        "counters are written at the end, not live"
    );

    h.store
        .finish_scan_run(
            id,
            &ScanRunFinish {
                ended_at: 160,
                walked_dirs: 214_903,
                found_repos: 147,
                cancelled: true,
            },
        )
        .unwrap();
    let done: (i64, Option<i64>, i64, i64) = h
        .index
        .lock()
        .unwrap()
        .conn()
        .query_row(
            "SELECT walked_dirs, ended_at, cancelled, found_repos FROM scan_run WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(done, (214_903, Some(160), 1, 147));
}

/// R26 in the one place it can actually fire. `ScanProblemKind::as_str` and the
/// `scan_problem.kind` CHECK are the same vocabulary written twice, and a mismatch is a runtime
/// insert failure on the two kinds a real machine produces most.
#[test]
fn every_problem_kind_the_core_emits_is_accepted_by_the_check_constraint() {
    let h = harness();
    let run = h
        .store
        .begin_scan_run(&ScanRunStart {
            generation: 1,
            started_at: 1,
            mode: "full",
            roots_json: "[]".to_owned(),
        })
        .unwrap();
    for kind in ScanProblemKind::ALL {
        h.store
            .record_problem(
                run,
                &ScanProblem {
                    kind,
                    path_display: "/r/a".to_owned(),
                    detail: "why".to_owned(),
                },
            )
            .unwrap_or_else(|e| panic!("kind {} rejected by the DDL: {e}", kind.as_str()));
    }
    let count: i64 = h
        .index
        .lock()
        .unwrap()
        .conn()
        .query_row("SELECT COUNT(*) FROM scan_problem", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 6);
}

#[test]
fn presence_is_written_and_read_back_through_the_column() {
    let h = harness();
    let id = seed_location(&h, "/r/a", "s1", 1);
    h.store.set_presence(id, Presence::Offline).unwrap();
    let rows = h.store.locations_for_presence().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].presence, Presence::Offline);
    assert_eq!(rows[0].store_key, "s1");
    assert_eq!(rows[0].path_key, b"/r/a");
    assert_eq!(h.store.indexed_project_count().unwrap(), 1);
}

/// §17, against the real database: a full offline sweep reclassifies and holds every row.
#[test]
fn a_presence_sweep_over_the_real_index_deletes_nothing() {
    let h = harness();
    seed_root(&h, "/r", true, false);
    let a = seed_location(&h, "/r/a", "internal", 1);
    let b = seed_location(&h, "/r/b", "usb", 1);
    let roots = h.store.scan_roots().unwrap();
    let skip = SkipList::default();
    let mut present = BTreeSet::new();
    present.insert("internal".to_owned());

    let summary = apply_presence(
        &h.store,
        &PresenceContext {
            generation: 2,
            roots: &roots,
            skip: &skip,
            present_stores: &present,
        },
    )
    .unwrap();
    assert_eq!(summary.missing, 1);
    assert_eq!(summary.offline, 1);

    let rows = h.store.locations_for_presence().unwrap();
    assert_eq!(rows.len(), 2, "presence never removes a row");
    let by_id: Vec<_> = rows.iter().map(|r| (r.location_id, r.presence)).collect();
    assert!(by_id.contains(&(a, Presence::Missing)));
    assert!(by_id.contains(&(b, Presence::Offline)));
}
