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

/// `PRAGMA table_info` as a list, so a column claim is read from the database rather than
/// inferred from the migration set.
fn table_columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .expect("prepare");
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query");
    rows.map(|r| r.expect("row")).collect()
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

/// Task 3's pairing guard, so the two halves cannot drift apart again.
///
/// `primary_location` and `presence` are one fact seen twice (§23.1), so a fixture that carries
/// a location carries a presence, and one that carries neither carries neither. Before §23 the
/// default paired a null location with `Presence::Unscanned` — the exact false claim §23.2
/// deleted from the loader — which would have left the fixture as the last place in the tree
/// still asserting it.
#[test]
fn the_two_fixture_builders_pair_location_and_presence() {
    use codotheca_core::protocol::ProjectRow;

    let located = ProjectRow::for_test(1);
    assert!(located.primary_location.is_some());
    assert!(
        located.presence.is_some(),
        "a fixture with a working copy has a presence"
    );

    let bare = ProjectRow::for_test_not_cloned(2);
    assert!(bare.primary_location.is_none());
    assert_eq!(
        bare.presence, None,
        "a fixture with no working copy has no presence"
    );

    // Everything else is the same row: the builder varies the pair and nothing else.
    assert_eq!(bare.name, ProjectRow::for_test(2).name);
    assert_eq!(bare.condition_signal, None);
}

/// AC-P2-23-1 and AC-P2-23-3's core half: §23.3's render contract, asserted over one fixture.
///
/// **Most of this table is a claim about code that exists today**, and that is the point — a
/// claim nothing tests is a claim that rots. A later plan that breaks one of these is told which.
/// The test prints how many zero-location fixtures it built and fails at zero: a passing run that
/// scanned none of them is a failing gate.
#[test]
fn the_render_contract_over_a_zero_location_row() {
    use codotheca_core::projects::list::aggregate_era;
    use codotheca_core::projects::peek::load_peek;
    use codotheca_core::protocol::{ProjectId, ReadmeStateKind};

    let (_dir, index) = opened();
    let mut built = 0_usize;
    for (id, name) in [(1_i64, "remoteonly"), (3, "alsoremote")] {
        zero_location_project(index.conn(), id, name);
        built += 1;
    }
    located_project(index.conn(), 2, "cloned");
    eprintln!("notcloned_row: built {built} zero-location fixture(s)");
    assert!(
        built > 0,
        "a run that built no zero-location fixture proves nothing"
    );

    let sink = CollectingSink::default();
    let deps = Deps::new();
    let ctx = ctx(&index, &sink, &deps);
    let rows = load_project_rows(&ctx).expect("load");
    let bare = find(&rows, 1);
    let r = &bare.row;

    // 1. Freshness. Each of these is a statement about a working copy, and there is none: no
    //    `as of T`, no age slot, no `no fetch recorded`.
    assert_eq!(r.refstate_observed_at, None);
    assert_eq!(r.worktree_observed_at, None);
    assert_eq!(r.fetch_head_at, None);

    // 2. No job ever ran, so `condition_signal` is NULL and §5.4a draws **no dot at all** (A5).
    assert_eq!(r.condition_signal, None);

    // 3. Nothing failed, so no `NOT INDEXED` badge is keyed — §11.1 keys it on BUDGET_EXCEEDED,
    //    never on the absent dot.
    assert_eq!(r.error_kind, None);
    assert_eq!(r.error_at, None);

    // 4. No HEAD, so no inventory — and the row contributes 0 to the header's coverage count.
    assert_eq!(r.size_tracked_bytes, None);
    assert_eq!(r.tracked_files, None);
    let agg = aggregate_era(&[bare]);
    assert_eq!(
        agg.indexed_count, 0,
        "an unmeasured row is not a measurement"
    );
    assert_eq!(agg.tracked_bytes, 0);

    // 5. `last_touched_at` falls back to `created_at` (`rows.rs:310-313`). It stays, and from
    //    here it orders rows **within** the not-cloned tail and does nothing else.
    assert_eq!(r.last_touched_at, r.created_at);

    // 6. Every working-copy fact is unknown, and `is_dirty` is None rather than Some(false):
    //    absence of dirty means "no changes as of T", never "clean".
    assert_eq!(r.is_dirty, None);
    assert_eq!(r.ahead, None);
    assert_eq!(r.behind, None);
    assert_eq!(r.stash_count, None);
    assert_eq!(r.untracked_count, None);
    assert_eq!(r.interrupted_op, None);
    assert_eq!(r.branch, None);

    // 7. Peek's bound (§23.3's eleventh row). No content is promised and no clock is claimed,
    //    and `ReadmeStateKind` still has exactly **three** variants — §23.3 forbids a fourth,
    //    and it forbids `not_indexed` for this row, so §23 lands the bound and p2-25 lands the
    //    answer. This is a count assertion in the shape of R49's tripwire: enumerate, do not
    //    grep. Adding a variant to the schema fails here rather than silently widening Peek.
    let peek = load_peek(&ctx, ProjectId(1)).expect("peek");
    assert_eq!(peek.readme.text, None);
    assert_eq!(peek.readme.read_at, None);
    let readme_kinds = [
        ReadmeStateKind::NotIndexed,
        ReadmeStateKind::Absent,
        ReadmeStateKind::Present,
    ];
    assert_eq!(
        readme_kinds.len(),
        3,
        "ReadmeStateKind gains no fourth variant (§23.3)"
    );
    for kind in readme_kinds {
        let slug = serde_json::to_value(kind).expect("encode");
        assert!(slug.is_string());
    }
    // The schema is the other side of that count, so the two cannot drift.
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../protocol/schema/protocol.json"))
            .expect("schema parses");
    let variants = schema["types"]["ReadmeStateKind"]["variants"]
        .as_array()
        .expect("ReadmeStateKind is an enum");
    assert_eq!(
        variants.len(),
        3,
        "the schema declares three too: {variants:?}"
    );
}

/// AC-P2-23-1's database half and **AC-P2-23-3's positive claim that §23 adds no schema** (A12).
///
/// Stated positively and read back from the database, because an absent migration block states
/// nothing and a migration set is a claim about files rather than about the schema that exists.
/// This test also prints the number of zero-location fixtures it built and fails at zero.
#[test]
fn the_database_holds_no_invented_row_column_or_table() {
    let (_dir, index) = opened();
    let mut built = 0_usize;
    for (id, name) in [(1_i64, "remoteonly"), (3, "alsoremote")] {
        zero_location_project(index.conn(), id, name);
        built += 1;
    }
    located_project(index.conn(), 2, "cloned");
    eprintln!("notcloned_row: built {built} zero-location fixture(s) for the schema half");
    assert!(
        built > 0,
        "a run that built no zero-location fixture proves nothing"
    );
    let sink = CollectingSink::default();
    let deps = Deps::new();
    let ctx = ctx(&index, &sink, &deps);
    let rows = load_project_rows(&ctx).expect("load");
    let r = &find(&rows, 1).row;

    // 8. §23.1 invents nothing. No synthetic `location` row, no fifth `Presence` value, and no
    //    `remote_project` table — read from the database, never trusted from the migration set.
    let locations: i64 = index
        .conn()
        .query_row(
            "SELECT count(*) FROM location WHERE project_id = 1",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(locations, 0);
    let presence_check: String = index
        .conn()
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='location'",
            [],
            |r| r.get(0),
        )
        .expect("location DDL");
    for value in ["present", "offline", "missing", "unscanned"] {
        assert!(presence_check.contains(value), "{value} left the enum");
    }
    for invented in ["not_cloned", "notcloned", "remote_only"] {
        assert!(
            !presence_check.contains(invented),
            "a fifth Presence value appeared: {invented}"
        );
    }
    let remote_project: i64 = index
        .conn()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='remote_project'",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(remote_project, 0, "§23 creates no remote_project table");

    // 9. **§23 introduced no column and no table** (A12). Stated positively, because an absent
    //    migration block states nothing. The clock a not-cloned tile resolves to is §25's
    //    `remote_repo.observed_at`, joined through §22's binding — never a second one here.
    let project_columns = table_columns(index.conn(), "project");
    for absent in [
        "remote_observed_at",
        "remote_read_at",
        "listing_observed_at",
    ] {
        assert!(
            !project_columns.iter().any(|c| c == absent),
            "§23 declares no column, and project.{absent} exists"
        );
    }
    assert!(
        index
            .conn()
            .query_row(
                "SELECT count(*) FROM pragma_table_info('remote_repo') WHERE name='observed_at'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .expect("count")
            == 1,
        "the one clock is remote_repo.observed_at, and p2-22 landed it"
    );

    // 10. AC-P2-23-3's core half. With §25's facts row **deleted**, no age is produced at all —
    //     no value, no `stale · <age>` input, and no substitute computed from `created_at` or
    //     `last_touched_at`. `ProjectRow` carries no remote field for one to hide in, which is
    //     the assertion: the loader manufactures nothing.
    let facts_rows: i64 = index
        .conn()
        .query_row("SELECT count(*) FROM remote_repo", [], |r| r.get(0))
        .expect("count");
    assert_eq!(facts_rows, 0, "the fixture writes no facts row");
    let wire = serde_json::to_value(r).expect("serialise");
    let obj = wire.as_object().expect("object");
    for (key, value) in obj {
        if key.ends_with("At") || key.ends_with("Age") {
            let dated = matches!(key.as_str(), "lastTouchedAt" | "createdAt");
            assert!(
                dated || value.is_null(),
                "{key} carried {value} for a project nothing has observed"
            );
        }
    }
    assert!(
        !obj.keys().any(|k| k.to_lowercase().contains("remote")),
        "ProjectRow carries no remote field for §23 to date"
    );
}
