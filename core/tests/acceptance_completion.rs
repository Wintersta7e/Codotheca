//! §31.11's core criteria — `AC-P3-31-*`.
//!
//! Every test carries its criterion id in its own name so §36's `tagsIn` finds it, and every
//! scanning check prints the number of things it looked at: **a gate whose passing run scans zero
//! is a failing gate.**

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use codotheca_core::clock::SystemClock;
use codotheca_core::completion::proposal::{proposes_na, suppressed_source};
use codotheca_core::completion::{evaluate_and_write, set_check_na, Written};
use codotheca_core::debt::singletons::{evaluate_singletons, ArmReading, SINGLETON_ARMS};
use codotheca_core::debt::store::SqliteDebtStore;
use codotheca_core::git::read_ref_state;
use codotheca_core::index::completion::{set_completion, Completion};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::IndexError;
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::classify::ARCHETYPES;
use codotheca_core::jobs::j1_refstate::persist;
use codotheca_core::protocol::{CompletionCheck, DebtSource, LocationId, ProjectId};
use support::TestRepo;

fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

fn insert_project(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, 'abc123', 1, 1)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// One copy. `worktree_newest_mtime` is §5.1's primary key, so a caller that needs a particular
/// copy to be the primary one says so rather than relying on insertion order.
fn insert_location(conn: &rusqlite::Connection, project: i64, touched: Option<i64>) -> LocationId {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM location", [], |r| r.get(0))
        .unwrap();
    let path = format!("/copy-{n}");
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind, worktree_newest_mtime)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree', ?4)",
        rusqlite::params![project, path.as_bytes(), path, touched],
    )
    .unwrap();
    LocationId(conn.last_insert_rowid())
}

fn set_tag_count(conn: &rusqlite::Connection, location: LocationId, tags: Option<i64>) {
    conn.execute(
        "UPDATE location SET tag_count = ?2 WHERE id = ?1",
        rusqlite::params![location.0, tags],
    )
    .unwrap();
}

fn set_shallow(conn: &rusqlite::Connection, project: i64, shallow: bool) {
    conn.execute(
        "UPDATE project SET is_shallow = ?2 WHERE id = ?1",
        rusqlite::params![project, i64::from(shallow)],
    )
    .unwrap();
}

/// The `no_release` arm's own reading, before the store turns it into a row.
fn no_release_reading(conn: &mut rusqlite::Connection, project: i64) -> ArmReading {
    let arm = SINGLETON_ARMS
        .iter()
        .find(|a| a.source() == DebtSource::NoRelease)
        .expect("the registry declares no_release");
    let tx = conn.transaction().unwrap();
    let reading = arm.observe(&tx, ProjectId(project)).unwrap();
    tx.rollback().unwrap();
    reading
}

/// The same predicate as the store sees it: the sweep outcome and the open item count.
fn no_release_sweep(conn: &mut rusqlite::Connection, project: i64) -> (String, i64) {
    let tx = conn.transaction().unwrap();
    evaluate_singletons(&tx, ProjectId(project), 100, &SqliteDebtStore).unwrap();
    tx.commit().unwrap();
    let outcome: String = conn
        .query_row(
            "SELECT outcome FROM debt_sweep WHERE project_id = ?1 AND source = 'no_release'",
            [project],
            |r| r.get(0),
        )
        .unwrap();
    let items: i64 = conn
        .query_row(
            "SELECT count(*) FROM debt_item WHERE project_id = ?1 AND source = 'no_release'",
            [project],
            |r| r.get(0),
        )
        .unwrap();
    (outcome, items)
}

// ---------------------------------------------------------------------------------------------
// AC-P3-31-11 — `release` never reads a shallow zero as a failure, and J1 writes the column
// ---------------------------------------------------------------------------------------------

/// The fourth clause: **J1 writes `tag_count`**, and it equals the repository's tag count.
///
/// The count is asserted `> 0` as well as equal, so a fixture that silently created no tag
/// cannot pass by agreeing with a column that is also zero.
#[test]
fn ac_p3_31_11_j1_persists_the_tag_count() {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    repo.git(&["tag", "v1"]);
    repo.git(&["tag", "v2"]);
    repo.git(&["tag", "v3"]);

    let expected = i64::try_from(repo.git(&["tag", "--list"]).lines().count()).unwrap();
    eprintln!("fixture tags: {expected}");
    assert!(
        expected > 0,
        "a fixture that created no tag proves nothing about the column"
    );

    let (_dir, mut conn) = fresh();
    let project = insert_project(&conn, "tagged");
    let location = insert_location(&conn, project, Some(10));

    // Before J1 the column is NULL, which is *no refstate persisted* and never *no tags*.
    let before: Option<i64> = conn
        .query_row(
            "SELECT tag_count FROM location WHERE id = ?1",
            [location.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, None);

    let state = read_ref_state(&repo.handle(), &SystemClock::new()).unwrap();
    assert_eq!(i64::from(state.tag_count), expected, "the reader counted");
    let tx = conn.transaction().unwrap();
    persist(&tx, location, &state).unwrap();
    tx.commit().unwrap();

    let stored: Option<i64> = conn
        .query_row(
            "SELECT tag_count FROM location WHERE id = ?1",
            [location.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        stored,
        Some(expected),
        "persist names eleven columns after §31.2, and tag_count is the eleventh"
    );
}

/// The first three clauses, asserted on the **arm** and again on the **check's own store row**.
///
/// A test that covered only the shallow case would have read green against p3-28's provisional
/// stub, which answered `Unobservable` unconditionally.
#[test]
fn ac_p3_31_11_a_shallow_zero_is_unknown_and_a_real_zero_is_a_failure() {
    let (_dir, mut conn) = fresh();

    // tag_count = 0, is_shallow = 1 -> Unobservable. A depth-1 clone fetches no tags.
    let shallow = insert_project(&conn, "shallow");
    let shallow_loc = insert_location(&conn, shallow, Some(10));
    set_tag_count(&conn, shallow_loc, Some(0));
    set_shallow(&conn, shallow, true);
    assert_eq!(
        no_release_reading(&mut conn, shallow),
        ArmReading::Unobservable
    );
    assert_eq!(
        no_release_sweep(&mut conn, shallow),
        ("unobservable".to_owned(), 0),
        "a shallow zero opens no item"
    );

    // tag_count = 0, is_shallow = 0 -> PredicateFalse, and exactly one item.
    let full = insert_project(&conn, "full");
    let full_loc = insert_location(&conn, full, Some(10));
    set_tag_count(&conn, full_loc, Some(0));
    set_shallow(&conn, full, false);
    assert!(matches!(
        no_release_reading(&mut conn, full),
        ArmReading::PredicateFalse(_)
    ));
    assert_eq!(
        no_release_sweep(&mut conn, full),
        ("complete".to_owned(), 1)
    );

    // tag_count IS NULL -> Unobservable whatever is_shallow says. Both halves run, because one
    // alone cannot tell a NULL from a shallow zero.
    for (name, shallow_flag) in [("null-shallow", true), ("null-full", false)] {
        let p = insert_project(&conn, name);
        let loc = insert_location(&conn, p, Some(10));
        set_tag_count(&conn, loc, None);
        set_shallow(&conn, p, shallow_flag);
        assert_eq!(
            no_release_reading(&mut conn, p),
            ArmReading::Unobservable,
            "{name}: NULL is never observed"
        );
        assert_eq!(
            no_release_sweep(&mut conn, p),
            ("unobservable".to_owned(), 0),
            "{name}"
        );
    }

    // tag_count >= 1 -> PredicateTrue, whatever the shallowness.
    let tagged = insert_project(&conn, "tagged");
    let tagged_loc = insert_location(&conn, tagged, Some(10));
    set_tag_count(&conn, tagged_loc, Some(2));
    set_shallow(&conn, tagged, true);
    assert_eq!(
        no_release_reading(&mut conn, tagged),
        ArmReading::PredicateTrue
    );
    assert_eq!(
        no_release_sweep(&mut conn, tagged),
        ("complete".to_owned(), 0)
    );
}

// ---------------------------------------------------------------------------------------------
// AC-P3-31-12 — multi-location aggregation is the primary copy's
// ---------------------------------------------------------------------------------------------

/// §5.1's primary copy decides, and `unknown` when **that** copy has no count — not when some
/// other copy does.
#[test]
fn ac_p3_31_12_release_reads_the_primary_locations_tag_count() {
    let (_dir, mut conn) = fresh();

    // The more recently touched copy holds no tags; the other one does. The primary decides.
    let p = insert_project(&conn, "two-copies");
    let older = insert_location(&conn, p, Some(10));
    let newer = insert_location(&conn, p, Some(99));
    set_tag_count(&conn, older, Some(4));
    set_tag_count(&conn, newer, Some(0));
    set_shallow(&conn, p, false);
    assert!(
        matches!(
            no_release_reading(&mut conn, p),
            ArmReading::PredicateFalse(_)
        ),
        "the primary copy has no tags, so the project has no release — the other copy's 4 is \
         not the project's answer"
    );

    // And the converse ordering, so the test cannot pass by always reading the same row.
    let q = insert_project(&conn, "two-copies-swapped");
    let q_older = insert_location(&conn, q, Some(10));
    let q_newer = insert_location(&conn, q, Some(99));
    set_tag_count(&conn, q_older, Some(0));
    set_tag_count(&conn, q_newer, Some(4));
    set_shallow(&conn, q, false);
    assert_eq!(no_release_reading(&mut conn, q), ArmReading::PredicateTrue);

    // `unknown` when the primary copy has none, even though a sibling copy does.
    let r = insert_project(&conn, "primary-unobserved");
    let r_older = insert_location(&conn, r, Some(10));
    let r_newer = insert_location(&conn, r, Some(99));
    set_tag_count(&conn, r_older, Some(4));
    set_tag_count(&conn, r_newer, None);
    set_shallow(&conn, r, false);
    assert_eq!(
        no_release_reading(&mut conn, r),
        ArmReading::Unobservable,
        "a sibling copy's count is not the primary copy's observation"
    );
}

// ---------------------------------------------------------------------------------------------
// AC-P3-31-17 — the proposal's coverage half
// ---------------------------------------------------------------------------------------------

/// The proposal table is asserted **against `classify.rs`'s returned archetype strings**, not
/// against a copy of them, and the count is derived rather than pinned at *"the eight"*
/// (R132/F11). **No literal `8` appears in this test.**
///
/// The command half — `na = true` surviving a J3 re-run, `na = null` returning the key to the
/// proposal, `na = false` forcing evaluation — is `projects.setCheckNa`'s.
#[test]
fn ac_p3_31_17_the_proposal_covers_every_archetype_and_every_key() {
    let mut covered = 0_u32;
    let mut proposed = 0_u32;
    for archetype in ARCHETYPES {
        for key in CompletionCheck::ALL {
            let structurally_meaningless = matches!(
                key,
                CompletionCheck::Tests
                    | CompletionCheck::Ci
                    | CompletionCheck::Deps
                    | CompletionCheck::Release
            );
            let expected = matches!(archetype, "docs" | "config") && structurally_meaningless;
            assert_eq!(
                proposes_na(Some(archetype), key),
                expected,
                "{archetype} / {key:?}"
            );
            covered += 1;
            proposed += u32::from(expected);
        }
    }
    eprintln!(
        "archetypes covered: {} · pairs asserted: {covered} · proposals: {proposed}",
        ARCHETYPES.len()
    );
    assert!(
        !ARCHETYPES.is_empty(),
        "a run covering no archetype is a failing run"
    );
    assert_eq!(
        covered,
        u32::try_from(ARCHETYPES.len() * CompletionCheck::ALL.len()).unwrap()
    );
    assert!(proposed > 0, "a table that proposes nothing proves nothing");

    // The second vocabulary. §30's suppression is over `DebtSource`, and the four keys a
    // documentation project proposes map onto three sources plus one deliberate absence — `ci`
    // owns no item at all, which is the dash in §31.4's table.
    let sources: Vec<Option<DebtSource>> = [
        CompletionCheck::Tests,
        CompletionCheck::Ci,
        CompletionCheck::Deps,
        CompletionCheck::Release,
    ]
    .into_iter()
    .map(suppressed_source)
    .collect();
    assert_eq!(
        sources,
        vec![
            Some(DebtSource::MissingTests),
            None,
            Some(DebtSource::DependencyAdvisory),
            Some(DebtSource::NoRelease),
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// AC-P3-31-1, -2, -3, -13 — the writer, the projection and the two NULL cases
// ---------------------------------------------------------------------------------------------

/// A project past every §31.8 gate, with one present copy whose refstate has been observed.
fn scorable(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                              created_at, updated_at)
         VALUES (?1, ?1, 'abc123', 1, 1, 100)",
        [name],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    let loc = insert_location(conn, id, Some(10));
    conn.execute(
        "UPDATE location SET refstate_observed_at = 50 WHERE id = ?1",
        [loc.0],
    )
    .unwrap();
    id
}

fn sweep(
    conn: &rusqlite::Connection,
    project: i64,
    source: &str,
    outcome: &str,
    items: Option<i64>,
) {
    conn.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, item_count, observed_at)
         VALUES (?1, ?2, ?3, ?4, 10)
         ON CONFLICT(project_id, source) DO UPDATE SET outcome = excluded.outcome,
             item_count = excluded.item_count",
        rusqlite::params![project, source, outcome, items],
    )
    .unwrap();
}

fn content_scan(conn: &rusqlite::Connection, project: i64, ci: &str) {
    conn.execute(
        "INSERT INTO project_content_scan
            (project_id, head_oid, complete_head_oid, blobs_total, blobs_pending,
             predicate_version, has_readme, has_license, has_tests, has_ci,
             presence_observed_at, enumerated_at, completed_at)
         VALUES (?1, 'head0', 'head0', 1, 0, 1, 'present', 'present', 'present', ?2, 1, 1, 1)
         ON CONFLICT(project_id) DO UPDATE SET has_ci = excluded.has_ci",
        rusqlite::params![project, ci],
    )
    .unwrap();
}

fn recompute(conn: &mut rusqlite::Connection, project: i64, now: i64) -> Written {
    let tx = conn.transaction().unwrap();
    let written = evaluate_and_write(&tx, ProjectId(project), now).unwrap();
    tx.commit().unwrap();
    written
}

fn projection(conn: &rusqlite::Connection, project: i64) -> (Option<i64>, Option<i64>) {
    conn.query_row(
        "SELECT completion_lit, completion_applicable FROM project WHERE id = ?1",
        [project],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

/// **`AC-P3-31-1`.** The projection equals a recount **from `project_check`**, taken in the test
/// rather than read back from the helper that wrote it.
#[test]
fn ac_p3_31_1_the_projection_equals_a_recount_of_the_rows() {
    let (_dir, mut conn) = fresh();

    // Three shapes, so the recount is exercised over more than one arithmetic.
    let all_pass = scorable(&conn, "all-pass");
    for source in [
        "missing_readme",
        "missing_license",
        "missing_tests",
        "ci_red",
        "unpushed_commits",
        "no_release",
    ] {
        sweep(&conn, all_pass, source, "complete", Some(0));
    }
    content_scan(&conn, all_pass, "present");

    let some_fail = scorable(&conn, "some-fail");
    sweep(&conn, some_fail, "missing_readme", "complete", Some(0));
    sweep(&conn, some_fail, "missing_license", "unobservable", None);
    content_scan(&conn, some_fail, "absent");

    let unswept = scorable(&conn, "unswept");

    for id in [all_pass, some_fail, unswept] {
        recompute(&mut conn, id, 1_000);
    }

    let mut recounted = 0_u32;
    let mut st = conn
        .prepare("SELECT DISTINCT project_id FROM project_check")
        .unwrap();
    let ids: Vec<i64> = st
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for id in ids {
        let lit: i64 = conn
            .query_row(
                "SELECT count(*) FROM project_check WHERE project_id = ?1 AND state = 'pass'",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        let evaluable: i64 = conn
            .query_row(
                "SELECT count(*) FROM project_check
                  WHERE project_id = ?1 AND state IN ('pass', 'fail')",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        let (stored_lit, stored_applicable) = projection(&conn, id);
        if evaluable == 0 {
            // §31.1: `evaluable == 0` is not a zero; it is NotComputed, and both columns are NULL.
            assert_eq!(
                (stored_lit, stored_applicable),
                (None, None),
                "project {id}"
            );
        } else {
            assert_eq!(stored_lit, Some(lit), "project {id} lit");
            assert_eq!(
                stored_applicable,
                Some(evaluable),
                "project {id} applicable is the EVALUABLE count"
            );
        }
        recounted += 1;
    }
    eprintln!("projects recounted: {recounted}");
    assert!(
        recounted > 0,
        "a run that recounted nothing is a failing run"
    );
}

/// **`AC-P3-31-2`.** No project holds a row count outside `{0, 10}`.
#[test]
fn ac_p3_31_2_a_project_holds_ten_rows_or_none() {
    let (_dir, mut conn) = fresh();
    let scored = scorable(&conn, "scored");
    content_scan(&conn, scored, "present");
    recompute(&mut conn, scored, 1_000);

    // A Reference project and a not-cloned one both write nothing at all.
    conn.execute(
        "INSERT INTO project (name, seed_basename, authored_by_user, is_reference,
                              created_at, updated_at)
         VALUES ('ref', 'ref', 0, 1, 1, 1)",
        [],
    )
    .unwrap();
    let reference = conn.last_insert_rowid();
    insert_location(&conn, reference, Some(1));
    recompute(&mut conn, reference, 1_000);

    conn.execute(
        "INSERT INTO project (name, seed_basename, authored_by_user, created_at, updated_at)
         VALUES ('uncloned', 'uncloned', 1, 1, 1)",
        [],
    )
    .unwrap();
    let uncloned = conn.last_insert_rowid();
    recompute(&mut conn, uncloned, 1_000);

    let mut st = conn
        .prepare(
            "SELECT p.id, (SELECT count(*) FROM project_check c WHERE c.project_id = p.id)
               FROM project p",
        )
        .unwrap();
    let counts: Vec<(i64, i64)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut distinct: Vec<i64> = counts.iter().map(|(_, n)| *n).collect();
    distinct.sort_unstable();
    distinct.dedup();
    eprintln!("distinct project_check row counts observed: {distinct:?}");
    assert!(
        !counts.is_empty(),
        "a run that scanned no project proves nothing"
    );
    for (id, n) in &counts {
        assert!(
            *n == 0 || *n == 10,
            "project {id} holds {n} rows, which is neither none nor ten"
        );
    }
    assert!(
        distinct.contains(&10),
        "no project was scored, so nothing was proven"
    );
    assert!(
        distinct.contains(&0),
        "no project was skipped, so nothing was proven"
    );
}

/// **`AC-P3-31-3`.** Ten rows that are all `unknown` or `na` store NULL on **both** columns, and
/// the guard that refuses `applicable = 0` stays rather than being removed.
#[test]
fn ac_p3_31_3_zero_evaluable_writes_null_and_keeps_its_guard() {
    let (_dir, mut conn) = fresh();
    // Nothing swept, no content scan, no remote, no refstate observed: every check is unknown,
    // except `description`, which is `na` because the project has no remote at all.
    conn.execute(
        "INSERT INTO project (name, seed_basename, authored_by_user, created_at, updated_at)
         VALUES ('bare', 'bare', 1, 1, 1)",
        [],
    )
    .unwrap();
    let p = conn.last_insert_rowid();
    insert_location(&conn, p, Some(1));

    let written = recompute(&mut conn, p, 1_000);
    assert!(matches!(written, Written::Rewritten { .. }));

    let rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM project_check WHERE project_id = ?1",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        rows, 10,
        "the ten rows are written; only the projection is NULL"
    );
    let evaluable: i64 = conn
        .query_row(
            "SELECT count(*) FROM project_check
              WHERE project_id = ?1 AND state IN ('pass', 'fail')",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(evaluable, 0);
    assert_eq!(projection(&conn, p), (None, None));

    // The guard is asserted, not removed.
    assert!(matches!(
        set_completion(
            &conn,
            ProjectId(p),
            Completion::Computed {
                lit: 0,
                applicable: 0
            }
        ),
        Err(IndexError::CompletionNotComputable)
    ));
}

/// **`AC-P3-31-13`.** A Reference project is never scored — and this is §31.10's
/// permanently-`NotComputed` fixture, which §16's criteria 45a–45c need to stay testable.
#[test]
fn ac_p3_31_13_a_reference_project_is_never_scored() {
    let (_dir, mut conn) = fresh();
    let mut scanned = 0_u32;
    for name in ["ref-one", "ref-two"] {
        conn.execute(
            "INSERT INTO project (name, seed_basename, authored_by_user, is_reference,
                                  created_at, updated_at)
             VALUES (?1, ?1, 0, 1, 1, 1)",
            [name],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        insert_location(&conn, id, Some(1));
        // Even fully swept, it is excluded: the gate is the project's kind, not its evidence.
        for source in ["missing_readme", "missing_license", "missing_tests"] {
            sweep(&conn, id, source, "complete", Some(0));
        }
        content_scan(&conn, id, "present");

        let written = recompute(&mut conn, id, 1_000);
        assert!(
            matches!(written, Written::Skipped(_)),
            "{name} was scored, and a Reference project is excluded for ever"
        );
        let rows: i64 = conn
            .query_row(
                "SELECT count(*) FROM project_check WHERE project_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0, "{name} holds a project_check row");
        assert_eq!(projection(&conn, id), (None, None), "{name}");
        scanned += 1;
    }
    eprintln!("reference fixtures scanned: {scanned}");
    assert!(
        scanned > 0,
        "a run that scanned no Reference project is a failing run"
    );
}

/// The diff gate: a recompute that changes nothing writes nothing, and `observed_at` does not
/// move. That is what makes *attempted* a derivation rather than a stored column (R123).
#[test]
fn a_no_change_recompute_leaves_observed_at_where_it_was() {
    let (_dir, mut conn) = fresh();
    let p = scorable(&conn, "steady");
    content_scan(&conn, p, "present");
    assert!(matches!(
        recompute(&mut conn, p, 1_000),
        Written::Rewritten { .. }
    ));

    let stamps = |conn: &rusqlite::Connection| -> Vec<i64> {
        let mut st = conn
            .prepare(
                "SELECT observed_at FROM project_check WHERE project_id = ?1 ORDER BY check_key",
            )
            .unwrap();
        st.query_map([p], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let before = stamps(&conn);
    assert_eq!(recompute(&mut conn, p, 9_999), Written::Unchanged);
    assert_eq!(
        stamps(&conn),
        before,
        "observed_at is when a state was last ESTABLISHED, not when it was last attempted"
    );

    // And a real change does move it.
    content_scan(&conn, p, "absent");
    assert!(matches!(
        recompute(&mut conn, p, 9_999),
        Written::Rewritten { .. }
    ));
    assert!(stamps(&conn).contains(&9_999));
}

// ---------------------------------------------------------------------------------------------
// AC-P3-31-17 — N/A is two stored facts, through the command that writes one of them
// ---------------------------------------------------------------------------------------------

/// `na = true` survives a J3 re-run that changes the archetype; `na = null` returns the key to
/// the proposal; `na = false` forces evaluation of a key the archetype proposes N/A for.
///
/// **The J3 re-run is modelled as what J3 writes** — the `archetype` column — because that is the
/// whole of what a re-classification changes, and the rule being tested is that `user_na` is a
/// *different* stored fact from it.
#[test]
fn ac_p3_31_17_a_user_ruling_survives_a_reclassification() {
    let (_dir, mut conn) = fresh();
    let p = scorable(&conn, "reclassified");
    content_scan(&conn, p, "present");
    // `tests` is a Group-A check, so its answer is §28's stored sweep and not a predicate §31
    // re-derives (R124).
    sweep(&conn, p, "missing_tests", "complete", Some(0));
    conn.execute(
        "UPDATE project SET archetype = 'library' WHERE id = ?1",
        [p],
    )
    .unwrap();
    recompute(&mut conn, p, 1_000);

    let row = |conn: &rusqlite::Connection, key: &str| -> (String, Option<i64>) {
        conn.query_row(
            "SELECT state, user_na FROM project_check WHERE project_id = ?1 AND check_key = ?2",
            rusqlite::params![p, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    assert_eq!(row(&conn, "tests").0, "pass", "a library evaluates `tests`");

    // The user rules `tests` not applicable.
    let set = |conn: &mut rusqlite::Connection, key: CompletionCheck, na: Option<bool>| {
        let tx = conn.transaction().unwrap();
        set_check_na(&tx, ProjectId(p), key, na, 2_000).unwrap();
        tx.commit().unwrap();
    };
    set(&mut conn, CompletionCheck::Tests, Some(true));
    assert_eq!(row(&conn, "tests"), ("na".to_owned(), Some(1)));

    // J3 re-runs and reclassifies the project as documentation, which **proposes** `tests` N/A.
    // The stored ruling is untouched: a proposal and a decision are two facts.
    conn.execute("UPDATE project SET archetype = 'docs' WHERE id = ?1", [p])
        .unwrap();
    recompute(&mut conn, p, 3_000);
    assert_eq!(
        row(&conn, "tests"),
        ("na".to_owned(), Some(1)),
        "a re-classification erased a decision the user made"
    );

    // Clearing the override returns the key to the proposal — still `na`, but now because the
    // archetype says so, which the stored NULL is what distinguishes.
    set(&mut conn, CompletionCheck::Tests, None);
    assert_eq!(row(&conn, "tests"), ("na".to_owned(), None));

    // And `na = false` forces evaluation of a key the archetype proposes N/A for.
    set(&mut conn, CompletionCheck::Tests, Some(false));
    assert_eq!(
        row(&conn, "tests"),
        ("pass".to_owned(), Some(0)),
        "user_na = 0 is an override, not the absence of a ruling"
    );

    // The row set stays ten across every transition.
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM project_check WHERE project_id = ?1",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 10);

    // And the projection is recounted in the same transaction, not left behind.
    let lit: i64 = conn
        .query_row(
            "SELECT count(*) FROM project_check WHERE project_id = ?1 AND state = 'pass'",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(projection(&conn, p).0, Some(lit));
}
