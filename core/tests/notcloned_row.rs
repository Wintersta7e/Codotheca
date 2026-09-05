//! §23's shape, core side: a `project` row with **zero `location` rows**.
//!
//! Most of §23.3's render contract is a claim about code that already exists, and a claim
//! nothing tests is a claim that rots. This file turns those claims into assertions over one
//! fixture, so a later plan that breaks one is told which.
//!
//! Compiled only under `testkit`: `art::testsupport::CollectingSink` is gated there, so without
//! the feature this fails to compile rather than skipping silently.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::index::Index;
use codotheca_core::projects::rows::{load_project_rows, LoadedRow};
use codotheca_core::projects::ProjectsCtx;
use codotheca_core::protocol::Presence;

const NOW: i64 = 1_781_000_000; // mid-2026

/// Inserts a `project` row and **no** `location`. Returns the id it wrote.
///
/// The whole point of §23.1 is that this is all a not-cloned project is: one row, one column,
/// nothing else. No synthetic location, no fifth `Presence`, no second table.
fn zero_location_project(conn: &rusqlite::Connection, id: i64, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (id, name, seed_basename, remote_key, created_at, updated_at)
         VALUES (?1, ?2, ?2, ?3, ?4, ?4)",
        rusqlite::params![id, name, format!("forge.example/acme/{name}"), NOW - 10],
    )
    .expect("insert project");
    id
}

/// A project that does have a working copy, so every assertion below has its counter-case.
fn located_project(conn: &rusqlite::Connection, id: i64, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (id, name, seed_basename, last_touched_at, created_at, updated_at)
         VALUES (?1, ?2, ?2, ?3, ?3, ?3)",
        rusqlite::params![id, name, NOW],
    )
    .expect("insert project");
    conn.execute(
        "INSERT INTO location (id, project_id, kind, distro, path_bytes, path_key, path_display,
                               volume_key, store_key, presence, repo_kind)
         VALUES (?1, ?2, 'linux', '', ?3, ?3, ?4, 'v', 's', 'present', 'worktree')",
        rusqlite::params![
            id * 10,
            id,
            format!("/w/{name}").as_bytes(),
            format!("/w/{name}")
        ],
    )
    .expect("insert location");
    id
}

struct Deps {
    jobs: codotheca_core::jobs::NullJobSink,
    mounts: codotheca_core::testing::FakeMountResolver,
}

impl Deps {
    fn new() -> Deps {
        Deps {
            jobs: codotheca_core::jobs::NullJobSink,
            mounts: codotheca_core::testing::FakeMountResolver::new(),
        }
    }
}

fn ctx<'a>(index: &'a Index, sink: &'a CollectingSink, deps: &'a Deps) -> ProjectsCtx<'a> {
    ProjectsCtx {
        index,
        events: sink,
        jobs: &deps.jobs,
        mounts: &deps.mounts,
        now: NOW,
        tz_offset_min: 0,
    }
}

fn opened() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    (dir, index)
}

fn find(rows: &[LoadedRow], id: i64) -> &LoadedRow {
    rows.iter()
        .find(|r| r.row.id.0 == id)
        .expect("row is in the projection")
}

/// AC-P2-23-2. `unscanned` has a rendered meaning and a wired control — §8.5.2 draws
/// `NOT SCANNED` and `LocationsPanel.tsx:118` calls `roots.setEnabled` on the covering root —
/// and a not-cloned project is covered by **no root**, so that control names a root that cannot
/// exist. §11.3a's dead-switch rule is what it breaks.
#[test]
fn a_zero_location_project_carries_no_presence_and_a_located_one_keeps_its_own() {
    let (_dir, index) = opened();
    zero_location_project(index.conn(), 1, "remoteonly");
    located_project(index.conn(), 2, "cloned");
    let sink = CollectingSink::default();
    let rows = load_project_rows(&ctx(&index, &sink, &Deps::new())).expect("load");
    assert_eq!(rows.len(), 2);

    let bare = find(&rows, 1);
    assert!(
        bare.row.primary_location.is_none(),
        "§23.1: the predicate is exactly primary_location IS NULL"
    );
    assert_eq!(
        bare.row.presence, None,
        "§23.2: presence is a property of a location, and there is none"
    );

    let cloned = find(&rows, 2);
    assert!(cloned.row.primary_location.is_some());
    assert_eq!(
        cloned.row.presence,
        Some(Presence::Present),
        "a located project's presence is unchanged by this ruling"
    );

    // The wire form carries `null`, not the string `unscanned`.
    let wire = serde_json::to_value(&bare.row).expect("serialise");
    assert!(
        wire["presence"].is_null(),
        "presence crossed as {:?}",
        wire["presence"]
    );
}

/// AC-P2-23-2's last clause, asserted rather than promised.
///
/// `core/src/scan/presence.rs`'s rollup is **not changed** by §23. It shares a name with the row
/// field and answers a different question (R15: compare shapes, not strings), and `apply_presence`
/// iterates `locations_for_presence()` — so a project with no `location` row cannot enter it at
/// all. This is also §4.6's marker made into a test: the generation rule visits `location` rows,
/// so a project with none is untouched by it — not `missing`, not `offline`, not `unscanned`.
#[test]
fn the_presence_rollup_is_byte_identical_when_a_zero_location_project_is_added() {
    use codotheca_core::scan::presence::{apply_presence, PresenceContext, ScanStore};
    use codotheca_core::scan::skiplist::SkipList;
    use codotheca_core::scan::store::SqliteScanStore;
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};

    let dir = tempfile::tempdir().expect("tempdir");
    let index = Arc::new(Mutex::new(Index::open(dir.path()).expect("open")));
    let store = SqliteScanStore::new(Arc::clone(&index));
    located_project(index.lock().expect("lock").conn(), 2, "cloned");

    let roots = store.scan_roots().expect("roots");
    let skip = SkipList::default();
    let present: BTreeSet<String> = BTreeSet::new();
    let ctx = PresenceContext {
        generation: 1,
        roots: &roots,
        skip: &skip,
        present_stores: &present,
    };
    // Twice, so `changed` has settled to 0 and the comparison below is about the added project
    // rather than about this harness's first write.
    apply_presence(&store, &ctx).expect("apply");
    let before = apply_presence(&store, &ctx).expect("apply");
    assert_eq!(
        before.changed, 0,
        "the run has settled before the comparison"
    );

    zero_location_project(index.lock().expect("lock").conn(), 1, "remoteonly");
    let after = apply_presence(&store, &ctx).expect("apply");

    assert_eq!(
        before, after,
        "a zero-location project changes no PresenceSummary field"
    );
    assert_eq!(
        after.unscanned + after.present + after.missing + after.offline,
        1,
        "exactly the one located row was classified"
    );
    // And it left no location behind: §23.1 invents no synthetic row.
    let locations: i64 = index
        .lock()
        .expect("lock")
        .conn()
        .query_row("SELECT count(*) FROM location", [], |r| r.get(0))
        .expect("count");
    assert_eq!(locations, 1);
}
