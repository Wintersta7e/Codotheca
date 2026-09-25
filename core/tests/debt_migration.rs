//! `0013_debt.sql` — the `xp_events` rebuild, §28.4a's backfill, and the two debt tables.
//!
//! §27.7's `subject_key` defect rides this plan: `core/src/index/subject.rs:45-48` renders
//! `lineage:{k}|remote:{r}` and `core/src/jobs/j4_history.rs` rendered `{lineage}:{remote}` for
//! the same logical subject. **A test exercising only the new writer proves nothing about the
//! defect** — every case below that matters reads the *other* writer's rows.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;

use codotheca_core::index::migrate::{apply_all, MIGRATIONS, SUPPORTED_SCHEMA_VERSION};

use codotheca_core::index::subject::ProjectSubject;
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::j4_history::{commit_days, local_date, HistoryFacts};
use codotheca_core::protocol::ProjectId;

/// The committed contract, compiled in rather than re-found at runtime.
const SCHEMA: &str = include_str!("../../protocol/schema/protocol.json");

/// Every variant the schema declares for `name`, in declaration order.
///
/// Panics rather than returning an empty vector for an absent type: a missing enum must fail the
/// test, not silently reduce it to a loop over nothing.
fn schema_variants(name: &str) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(SCHEMA).expect("protocol.json parses");
    let decl = doc["types"]
        .get(name)
        .unwrap_or_else(|| panic!("{name} is not declared in protocol.json"));
    assert_eq!(decl["kind"], "enum", "{name} is not an enum");
    let variants: Vec<String> = decl["variants"]
        .as_array()
        .unwrap_or_else(|| panic!("{name} declares no variants"))
        .iter()
        .map(|v| v.as_str().expect("a variant is a string").to_owned())
        .collect();
    assert!(!variants.is_empty(), "{name} declares no variants");
    variants
}

/// A database at exactly `version` files applied, through the shipped set.
fn migrated_to(version: usize) -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, &MIGRATIONS[..version]).unwrap();
    (dir, conn)
}

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    migrated_to(MIGRATIONS.len())
}

fn insert_project(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn set_identity(conn: &rusqlite::Connection, id: i64, lineage: &str, remote: Option<&str>) {
    conn.execute(
        "UPDATE project SET lineage_key = ?2, remote_key = ?3 WHERE id = ?1",
        rusqlite::params![id, lineage, remote],
    )
    .unwrap();
}

fn columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let mut st = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .unwrap();
    let rows = st.query_map([], |r| r.get::<_, String>(1)).unwrap();
    rows.map(Result::unwrap).collect()
}

fn xp_insert(
    conn: &rusqlite::Connection,
    project: i64,
    kind: &str,
    track: &str,
    dedupe: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (1, 0, ?1, 'lineage:k|remote:', ?2, ?3, ?4, NULL)",
        rusqlite::params![project, kind, dedupe, track],
    )
}

// ---------------------------------------------------------------------------------------------
// The rebuild itself
// ---------------------------------------------------------------------------------------------

/// **The prefix is the subject here, not the tip.** Read against `fresh()` this asserted whichever
/// migration happened to be last, so every later file had to edit it — and a test named for
/// `0013` that tracks the tip stops meaning what it says. `migrated_to(13)` is the database this
/// file actually produces; the tip constant is asserted where the tip is owned, in the last
/// migration's own test.
#[test]
fn the_chain_reaches_thirteen_and_stamps_it() {
    let (_d, conn) = migrated_to(13);
    let stamped: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stamped, 13, "0013 must stamp its own version");
    let shipped = MIGRATIONS.iter().find(|m| m.version == 13);
    assert_eq!(
        shipped.map(|m| m.name),
        Some("debt"),
        "0013 is still the debt migration in the shipped chain"
    );
    assert_eq!(
        shipped.map(|m| m.version),
        Some(u32::min(13, SUPPORTED_SCHEMA_VERSION)),
        "the chain still reaches 0013"
    );
}

/// `AC-P3-28-16`. The two CHECKs widen **together**: `debt_day` joins `kind`'s list and joins
/// the `session` side of the track equivalence. A `debt_day` row on the git track would satisfy
/// `kind`'s CHECK alone, be deleted by the first merge's `recompute_derived`, and be covered by
/// no `level_floor` — silent, permanent data loss.
#[test]
fn a_debt_day_is_a_session_row_and_the_git_track_is_refused() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");

    xp_insert(&conn, p, "debt_day", "session", "debt_day:a").unwrap();

    let refused = xp_insert(&conn, p, "debt_day", "git", "debt_day:b");
    assert!(
        refused.is_err(),
        "a debt_day on the git track must be refused by the DDL, not by a convention"
    );

    // The mirror case, so the equivalence is asserted in both directions rather than one.
    let refused_session_commit = xp_insert(&conn, p, "commit_day", "session", "commit_day:c");
    assert!(refused_session_commit.is_err());
}

/// The rebuild changes **two CHECKs and nothing else**. A create-copy-drop-rename drafted from a
/// stale copy of the table silently drops a column, which is exactly what `0007`'s two `ALTER`ed
/// columns did to `0012`'s draft.
#[test]
fn the_rebuild_changes_no_column() {
    let (_d, before) = migrated_to(12);
    let was = columns(&before, "xp_events");
    drop(before);

    let (_d2, after) = fresh();
    let now = columns(&after, "xp_events");

    assert_eq!(was, now, "0013 must widen two CHECKs and move no column");
    assert!(!was.is_empty(), "a diff over zero columns proves nothing");

    // The index is recreated by name, because a rebuild drops it with the table.
    let idx: i64 = after
        .query_row(
            "SELECT count(*) FROM sqlite_master
              WHERE type='index' AND name='idx_xp_events_project_ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(idx, 1, "idx_xp_events_project_ts was not recreated");
}

/// §1.5's guarantee lives entirely in `sqlite_sequence`'s high-water mark, and
/// `DROP TABLE` deletes it. Seed three, delete all three, migrate, insert one: `4` proves the
/// mark was carried; `1` proves it was re-seeded from `max(id)` of an empty table.
#[test]
fn the_autoincrement_high_water_mark_survives_the_rebuild() {
    let (_d, mut conn) = migrated_to(12);
    let p = insert_project(&conn, "thing");
    for n in 0..3 {
        xp_insert(&conn, p, "session", "session", &format!("session:{n}")).unwrap();
    }
    conn.execute("DELETE FROM xp_events", []).unwrap();

    apply_all(&mut conn, MIGRATIONS).unwrap();

    xp_insert(&conn, p, "debt_day", "session", "debt_day:after").unwrap();
    let id: i64 = conn
        .query_row("SELECT id FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(id, 4, "the high-water mark was not carried across the drop");
}

// ---------------------------------------------------------------------------------------------
// §28.4a's backfill
// ---------------------------------------------------------------------------------------------

/// The backfill rewrites from the **project row**, never by splitting the stored string:
/// `<lineage>:<remote>` cannot be split safely in general.
#[test]
fn the_backfill_rewrites_the_old_two_writer_key() {
    let (_d, mut conn) = migrated_to(12);
    let p = insert_project(&conn, "thing");
    set_identity(&conn, p, "abc123", Some("github.com/o/r"));

    // The shape `j4_history.rs` wrote before the convergence.
    conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (1, 0, ?1, 'abc123:github.com/o/r', 'commit_day', 'commit_day:old', 'git', NULL)",
        [p],
    )
    .unwrap();
    // A row that already reads the right shape must not be touched, and a `session` row of
    // another kind must not be touched either.
    conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (1, 0, ?1, 'lineage:abc123|remote:github.com/o/r', 'commit_day',
                 'commit_day:new', 'git', NULL)",
        [p],
    )
    .unwrap();

    apply_all(&mut conn, MIGRATIONS).unwrap();

    let want = ProjectSubject::Lineage {
        lineage_key: "abc123".to_owned(),
        remote_key: Some("github.com/o/r".to_owned()),
    }
    .to_key();

    let rewritten: i64 = conn
        .query_row(
            "SELECT count(*) FROM xp_events WHERE subject_key = ?1",
            [&want],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!("backfill: {rewritten} row(s) now read to_key()'s shape");
    assert_eq!(
        rewritten, 2,
        "a run that rewrites zero rows is a failing run, not a clean one"
    );

    let stragglers: i64 = conn
        .query_row(
            "SELECT count(*) FROM xp_events WHERE subject_key NOT LIKE 'lineage:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stragglers, 0);
}

/// A project with **no lineage** has nothing to rewrite from, and the backfill must leave its
/// rows alone rather than writing `lineage:|remote:` over them.
#[test]
fn the_backfill_leaves_a_row_it_cannot_rebuild_alone() {
    let (_d, mut conn) = migrated_to(12);
    let p = insert_project(&conn, "unlineaged");
    conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (1, 0, ?1, 'whatever', 'commit_day', 'commit_day:x', 'git', NULL)",
        [p],
    )
    .unwrap();

    apply_all(&mut conn, MIGRATIONS).unwrap();

    let key: String = conn
        .query_row("SELECT subject_key FROM xp_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        key, "whatever",
        "a row with no lineage to read was rewritten"
    );
}

// ---------------------------------------------------------------------------------------------
// `AC-P3-28-9` — the two writers agree, byte for byte
// ---------------------------------------------------------------------------------------------

/// **The defect this task exists to close.** One project, two writers: `commit_days` (phase 1's)
/// and `ProjectSubject::to_key()` (the sidecar's, and `debt_day`'s). The comparison is on the
/// bytes, and both sides round-trip through `parse`.
#[test]
fn both_xp_writers_render_one_subject_key() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    set_identity(&conn, p, "abc123", Some("github.com/o/r"));

    let facts = HistoryFacts {
        days: BTreeSet::from([20_000_i64]),
        ..HistoryFacts::default()
    };
    let tx = conn.transaction().unwrap();
    commit_days(
        &tx,
        ProjectId(p),
        Some("abc123"),
        Some("github.com/o/r"),
        &facts,
    )
    .unwrap();

    let subject = ProjectSubject::Lineage {
        lineage_key: "abc123".to_owned(),
        remote_key: Some("github.com/o/r".to_owned()),
    };
    tx.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (2, 0, ?1, ?2, 'debt_day', ?3, 'session', NULL)",
        rusqlite::params![
            p,
            subject.to_key(),
            format!("debt_day:{}:{}", subject.to_key(), local_date(20_000)),
        ],
    )
    .unwrap();
    tx.commit().unwrap();

    let mut st = conn
        .prepare("SELECT kind, subject_key FROM xp_events ORDER BY kind")
        .unwrap();
    let rows: Vec<(String, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 2, "both writers must have written");

    assert_eq!(
        rows[0].1.as_bytes(),
        rows[1].1.as_bytes(),
        "the two writers disagree: {:?} ({}) vs {:?} ({})",
        rows[0].1,
        rows[0].0,
        rows[1].1,
        rows[1].0,
    );

    for (kind, key) in &rows {
        assert_eq!(
            ProjectSubject::parse(key),
            Some(subject.clone()),
            "{kind}'s key does not round-trip through parse"
        );
    }
}

/// The empty-remote case, because that is the one a naive split gets wrong: `to_key()` keeps the
/// `|remote:` separator with nothing after it, and `parse` reads it back as `None`.
#[test]
fn both_writers_agree_when_there_is_no_remote() {
    let (_d, mut conn) = fresh();
    let p = insert_project(&conn, "thing");
    set_identity(&conn, p, "abc123", None);

    let facts = HistoryFacts {
        days: BTreeSet::from([20_001_i64]),
        ..HistoryFacts::default()
    };
    let tx = conn.transaction().unwrap();
    commit_days(&tx, ProjectId(p), Some("abc123"), None, &facts).unwrap();
    tx.commit().unwrap();

    let key: String = conn
        .query_row("SELECT subject_key FROM xp_events", [], |r| r.get(0))
        .unwrap();
    let want = ProjectSubject::Lineage {
        lineage_key: "abc123".to_owned(),
        remote_key: None,
    };
    assert_eq!(key.as_bytes(), want.to_key().as_bytes());
    assert_eq!(ProjectSubject::parse(&key), Some(want));
}

// ---------------------------------------------------------------------------------------------
// §28.8's two tables — `AC-P3-28-10`
// ---------------------------------------------------------------------------------------------

fn insert_location(conn: &rusqlite::Connection, project: i64) -> i64 {
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', x'2f61', x'2f61', '/a', 'store', 'present', 'worktree')",
        [project],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn insert_item(
    conn: &rusqlite::Connection,
    project: i64,
    source: &str,
    fingerprint: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                first_seen_at, last_seen_at)
         VALUES (?1, 'lineage:k|remote:', ?2, ?3, 'open', 'scored', 1, 1)",
        rusqlite::params![project, source, fingerprint],
    )
}

/// **`AC-P3-28-10`.** The four mirrored CHECK literals are one value stated twice with a
/// generated enum (R26). Each loop **enumerates the schema** — never a literal beside the
/// column — inserts one row per variant against a real migrated database, prints the count, and
/// fails at zero. `core/src/jobs/mod.rs:202-208` is the shape, and its own doc comment records
/// R26 firing in production: the DDL rejected a value the column's own producer emits.
#[test]
fn ac_p3_28_10_every_declared_variant_is_accepted_by_its_column() {
    let (_d, conn) = fresh();
    let project = insert_project(&conn, "thing");
    let location = insert_location(&conn, project);

    let sources = schema_variants("DebtSource");
    let mut n_source = 0_usize;
    for (i, source) in sources.iter().enumerate() {
        insert_item(&conn, project, source, &format!("f{i}"))
            .unwrap_or_else(|e| panic!("debt_item.source refused {source:?}: {e}"));
        n_source += 1;
    }
    eprintln!("debt_item.source accepted {n_source} of DebtSource's declared variants");
    assert!(
        n_source > 0,
        "inserted nothing, so the column proved nothing"
    );
    assert_eq!(n_source, sources.len());

    let states = schema_variants("DebtItemState");
    let mut n_state = 0_usize;
    for (i, state) in states.iter().enumerate() {
        conn.execute(
            "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                    first_seen_at, last_seen_at)
             VALUES (?1, 'lineage:state', 'todo_marker', ?2, ?3, 'scored', 1, 1)",
            rusqlite::params![project, format!("s{i}"), state],
        )
        .unwrap_or_else(|e| panic!("debt_item.state refused {state:?}: {e}"));
        n_state += 1;
    }
    eprintln!("debt_item.state accepted {n_state} of DebtItemState's declared variants");
    assert!(
        n_state > 0,
        "inserted nothing, so the column proved nothing"
    );

    let scorings = schema_variants("DebtScoring");
    let mut n_scoring = 0_usize;
    for (i, scoring) in scorings.iter().enumerate() {
        conn.execute(
            "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                    first_seen_at, last_seen_at)
             VALUES (?1, 'lineage:scoring', 'todo_marker', ?2, 'open', ?3, 1, 1)",
            rusqlite::params![project, format!("c{i}"), scoring],
        )
        .unwrap_or_else(|e| panic!("debt_item.scoring refused {scoring:?}: {e}"));
        n_scoring += 1;
    }
    eprintln!("debt_item.scoring accepted {n_scoring} of DebtScoring's declared variants");
    assert!(
        n_scoring > 0,
        "inserted nothing, so the column proved nothing"
    );

    // `debt_sweep` is one row per `(project_id, source)`, so each outcome needs its own source.
    // More outcomes than sources is possible, so a fresh project per outcome keeps the loop
    // total over the enum rather than over whichever is shorter.
    let outcomes = schema_variants("DebtSweepOutcome");
    let mut n_outcome = 0_usize;
    for (i, outcome) in outcomes.iter().enumerate() {
        let p = insert_project(&conn, &format!("sweep-{i}"));
        // The honesty CHECK: `complete` and `partial` carry a count and nothing else may.
        let count: Option<i64> = (outcome == "complete" || outcome == "partial").then_some(0);
        conn.execute(
            "INSERT INTO debt_sweep (project_id, source, outcome, location_id, generation, basis,
                                     item_count, observed_at)
             VALUES (?1, 'todo_marker', ?2, ?3, 0, 'head', ?4, 1)",
            rusqlite::params![p, outcome, location, count],
        )
        .unwrap_or_else(|e| panic!("debt_sweep.outcome refused {outcome:?}: {e}"));
        n_outcome += 1;
    }
    eprintln!("debt_sweep.outcome accepted {n_outcome} of DebtSweepOutcome's declared variants");
    assert!(
        n_outcome > 0,
        "inserted nothing, so the column proved nothing"
    );

    // `ObservationBasis` rides two columns, and both are asserted: a CHECK copied to one and
    // mistyped in the other is exactly the drift R26 names.
    let bases = schema_variants("ObservationBasis");
    let mut n_basis = 0_usize;
    for (i, basis) in bases.iter().enumerate() {
        conn.execute(
            "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                    basis, first_seen_at, last_seen_at)
             VALUES (?1, 'lineage:basis', 'todo_marker', ?2, 'open', 'scored', ?3, 1, 1)",
            rusqlite::params![project, format!("b{i}"), basis],
        )
        .unwrap_or_else(|e| panic!("debt_item.basis refused {basis:?}: {e}"));
        let p = insert_project(&conn, &format!("basis-{i}"));
        conn.execute(
            "INSERT INTO debt_sweep (project_id, source, outcome, basis, item_count, observed_at)
             VALUES (?1, 'todo_marker', 'complete', ?2, 0, 1)",
            rusqlite::params![p, basis],
        )
        .unwrap_or_else(|e| panic!("debt_sweep.basis refused {basis:?}: {e}"));
        n_basis += 2;
    }
    eprintln!("the two basis columns accepted {n_basis} rows over ObservationBasis");
    assert!(
        n_basis > 0,
        "inserted nothing, so the columns proved nothing"
    );
}

/// SQLite treats NULLs as distinct inside a UNIQUE index, so a nullable `fingerprint` would
/// silently permit duplicate singletons — the defect `0002_locations_and_roots.sql:7-9` records
/// against `location.distro`. `''` is the singleton's fingerprint and the column is NOT NULL.
#[test]
fn two_singletons_of_one_source_collide_on_the_unique_index() {
    let (_d, conn) = fresh();
    let project = insert_project(&conn, "thing");

    insert_item(&conn, project, "missing_readme", "").unwrap();
    let second = insert_item(&conn, project, "missing_readme", "");
    assert!(
        second.is_err(),
        "a second singleton of one source must collide, not duplicate"
    );

    // A NULL fingerprint is refused outright, so the NULL-distinctness hole cannot be reached.
    let nulled = conn.execute(
        "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state, scoring,
                                first_seen_at, last_seen_at)
         VALUES (?1, 'lineage:k|remote:', 'missing_license', NULL, 'open', 'scored', 1, 1)",
        [project],
    );
    assert!(nulled.is_err(), "fingerprint must be NOT NULL");
}

/// Zero and unknown are different facts and the DDL says so, as `scan_problem.count` refuses a
/// zero row and uninstall NULLs rather than zeroes.
#[test]
fn a_sweep_may_carry_a_count_only_when_it_observed() {
    let (_d, conn) = fresh();
    let p1 = insert_project(&conn, "a");
    let p2 = insert_project(&conn, "b");
    let p3 = insert_project(&conn, "c");

    let complete_without = conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, 'todo_marker', 'complete', NULL, 1)",
        [p1],
    );
    assert!(
        complete_without.is_err(),
        "a complete sweep with no count says it observed and refuses to say what"
    );

    let unobservable_with_zero = conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, 'todo_marker', 'unobservable', 0, 1)",
        [p2],
    );
    assert!(
        unobservable_with_zero.is_err(),
        "zero items is a reading; unobservable is the absence of one"
    );

    conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, 'todo_marker', 'complete', 0, 1)",
        [p3],
    )
    .unwrap();
}

/// One row per `(project_id, source)`, upserted. The pair a closure needs is the stored row and
/// the sweep in hand — never a history of sweeps, which nothing reads.
#[test]
fn a_sweep_is_one_row_per_project_and_source() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");
    conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, 'todo_marker', 'complete', 0, 1)",
        [p],
    )
    .unwrap();
    let repeat = conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, 'todo_marker', 'partial', 1, 2)",
        [p],
    );
    assert!(repeat.is_err(), "a second sweep row for one source");

    // A different source on the same project is a different row and is admitted.
    conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, 'missing_readme', 'complete', 0, 2)",
        [p],
    )
    .unwrap();
}

/// Both tables are `ON DELETE CASCADE` children of `project`. The index exists by name, because
/// `idx_debt_item_project_state` is what makes the per-project open-item read not a table scan.
#[test]
fn both_tables_cascade_from_project_and_the_index_exists() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");
    insert_item(&conn, p, "todo_marker", "f0").unwrap();
    conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, 'todo_marker', 'complete', 1, 1)",
        [p],
    )
    .unwrap();

    conn.execute("DELETE FROM project WHERE id = ?1", [p])
        .unwrap();

    for table in ["debt_item", "debt_sweep"] {
        let left: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "{table} did not cascade");
    }

    let idx: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master
              WHERE type='index' AND name='idx_debt_item_project_state'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(idx, 1);
}

// ---------------------------------------------------------------------------------------------
// §28.2's name ban — `AC-P3-28-18a`, the exact-name half
// ---------------------------------------------------------------------------------------------

/// The four classes §30.6 refuses as debt, each named for why: decay by staleness alone,
/// upstream drift no offered act can close, uncacheable worktree state, and a remote's absence.
const BANNED_SOURCE_NAMES: [&str; 4] =
    ["staleness", "behind_upstream", "uncommitted", "dead_remote"];

/// Every quoted literal inside `CHECK (<column> IN (…))` in one table's DDL.
fn check_literals(ddl: &str, column: &str) -> Vec<String> {
    let opener = format!("CHECK ({column} IN (");
    let start = ddl
        .find(&opener)
        .unwrap_or_else(|| panic!("no `{opener}` in {ddl}"))
        + opener.len();
    let body = ddl.get(start..).expect("the opener ends inside the DDL");
    let (list, _) = body.split_once(')').expect("the IN list closes");
    list.split(',')
        .map(|v| v.trim().trim_matches('\'').to_owned())
        .collect()
}

/// **`AC-P3-28-18a`, the exact-name half.** The rendered-string half is `check-forbidden`'s
/// `p3-28-debt-source-names`, which must match value shapes (`uncommitted_work`) because the bare
/// word `uncommitted` is a live chip id — so a variant spelled exactly `uncommitted` passes it.
/// This half bans all four names **bare**, over the vocabularies where a source name can land: the
/// generated enums and every CHECK literal the migrated schema holds for a source or check key.
#[test]
fn ac_p3_28_18a_no_source_or_check_key_is_a_banned_name() {
    let mut scanned = 0_usize;
    for enumeration in ["DebtSource", "CompletionCheck"] {
        let variants = schema_variants(enumeration);
        for banned in BANNED_SOURCE_NAMES {
            assert!(
                !variants.iter().any(|v| v == banned),
                "{enumeration} declares the banned name {banned:?}"
            );
        }
        eprintln!("{enumeration}: {} variants scanned", variants.len());
        scanned += variants.len();
    }

    let (_d, conn) = fresh();
    for (table, column) in [
        ("debt_item", "source"),
        ("debt_sweep", "source"),
        ("project_check", "check_key"),
    ] {
        let ddl: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("{table} is not in the migrated schema: {e}"));
        let literals = check_literals(&ddl, column);
        for banned in BANNED_SOURCE_NAMES {
            assert!(
                !literals.iter().any(|v| v == banned),
                "{table}.{column}'s CHECK admits the banned name {banned:?}"
            );
        }
        eprintln!(
            "{table}.{column}: {} CHECK literals scanned",
            literals.len()
        );
        scanned += literals.len();
    }

    eprintln!("name ban: {scanned} names scanned in total");
    assert!(scanned > 0, "scanned nothing, so the ban proved nothing");
}
