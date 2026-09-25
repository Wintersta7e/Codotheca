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

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::completion::proposal::{proposes_na, suppressed_source};
use codotheca_core::completion::{evaluate_and_write, set_check_na, Written};
use codotheca_core::debt::singletons::{evaluate_singletons, ArmReading, SINGLETON_ARMS};
use codotheca_core::debt::store::SqliteDebtStore;
use codotheca_core::git::{head_tree, read_ref_state, RunLimits};
use codotheca_core::identity::merge::recompute_derived;
use codotheca_core::index::completion::{set_completion, Completion};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::IndexError;
use codotheca_core::index::{open_connection, Index};
use codotheca_core::jobs::classify::ARCHETYPES;
use codotheca_core::jobs::j1_refstate::persist;
use codotheca_core::jobs::j6_content::{self, ContentFacts};
use codotheca_core::jobs::presence::{presence_for, PresenceAnswers, PresenceState};
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

fn committed_presence(repo: &TestRepo) -> PresenceAnswers {
    let entries = head_tree(
        &repo.exec(),
        &repo.handle(),
        RunLimits::none(),
        &CancelToken::new(),
    )
    .unwrap();
    assert!(
        !entries.is_empty(),
        "the fixture must enumerate committed paths"
    );
    presence_for(&entries)
}

/// Seed the independent observations so only the three budget-limited checks can be unknown.
fn observed_completion_inputs(conn: &rusqlite::Connection, project: i64) {
    conn.execute(
        "UPDATE location SET ahead = 0, tag_count = 1, branch = 'main' WHERE project_id = ?1",
        [project],
    )
    .unwrap();
    conn.execute(
        "UPDATE project SET remote_key = 'forge/fixture/project', provider = 'forge',
                provider_repo_id = ?1 WHERE id = ?1",
        [project],
    )
    .unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO account
            (provider, host, login, auth_kind, scope_tier, granted_scopes, token_ref, connected_at)
         VALUES ('forge', 'forge.invalid', 'fixture', 'pat', 'public', '', 'fixture-ref', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO remote_repo (provider, provider_repo_id, description, observed_at)
         VALUES ('forge', ?1, 'fixture description', 50)",
        [project],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO remote_topic (provider, provider_repo_id, topic) VALUES ('forge', ?1, 'topic')",
        [project],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO remote_ci_run
            (provider, provider_repo_id, run_id, workflow_name, conclusion, branch, run_number,
             started_at)
         VALUES ('forge', ?1, 1, 'checks', 'success', 'main', 1, 50)",
        [project],
    )
    .unwrap();
    dependency_scan(conn, project, true, 0);
}

fn open_items(conn: &rusqlite::Connection, project: i64, source: &str) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM debt_item WHERE project_id = ?1 AND source = ?2 AND state = 'open'",
        rusqlite::params![project, source],
        |r| r.get(0),
    )
    .unwrap()
}

/// Uses the persisted §29 row seam: a real J7 enumeration budget failure marks all FOUR
/// predicates `not_read`, including README. This criterion needs only three unknowns and seven
/// evaluable checks, so retain the observed README and inject the other three budget answers.
/// Both trees are enumerated with J7's HEAD path predicates before supplying those answers.
/// The renderer's companion check covers the notched frame and its demotion rule.
#[test]
fn ac_p3_31_10_budget_exceeded_is_unknown_but_absent_is_fail() {
    let (_dir, mut conn) = fresh();
    for exists in [true, false] {
        let repo = TestRepo::init();
        repo.write("README.md", b"# fixture\n");
        for path in ["LICENSE", "tests/check.rs", ".github/workflows/ci.yml"] {
            if exists {
                repo.write(path, b"fixture\n");
            }
        }
        repo.commit("fixture");
        let actual = committed_presence(&repo);
        let expected = if exists {
            PresenceState::Present
        } else {
            PresenceState::Absent
        };
        assert_eq!(actual.readme, PresenceState::Present);
        assert_eq!([actual.license, actual.tests, actual.ci], [expected; 3]);

        let p = scorable(&conn, if exists { "budget-exceeded" } else { "absent" });
        observed_completion_inputs(&conn, p);
        let reported = if exists { "not_read" } else { "absent" };
        content_scan(&conn, p, reported);
        conn.execute(
            "UPDATE project_content_scan SET has_license = ?2, has_tests = ?2 WHERE project_id = ?1",
            rusqlite::params![p, reported],
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        evaluate_singletons(&tx, ProjectId(p), 100, &SqliteDebtStore).unwrap();
        let written = evaluate_and_write(&tx, ProjectId(p), 100).unwrap();
        tx.commit().unwrap();

        for key in ["license", "tests", "ci"] {
            let expected_state = if exists {
                ("unknown".to_owned(), Some("notRead".to_owned()))
            } else {
                ("fail".to_owned(), None)
            };
            assert_eq!(
                check_state(&conn, p, key),
                expected_state,
                "{reported}: {key}"
            );
        }
        for source in ["missing_license", "missing_tests"] {
            assert_eq!(open_items(&conn, p, source), i64::from(!exists), "{source}");
        }
        let Written::Rewritten { counts } = written else {
            panic!("expected new check rows")
        };
        if exists {
            assert_eq!(
                (counts.lit, counts.evaluable, counts.unknown, counts.na),
                (7, 7, 3, 0)
            );
            assert_eq!(projection(&conn, p), (Some(7), Some(7)));
        } else {
            // No CI configuration makes ciGreen N/A; the three absent checks still fail.
            assert_eq!(
                (counts.lit, counts.evaluable, counts.unknown, counts.na),
                (6, 9, 0, 1)
            );
            assert_eq!(projection(&conn, p), (Some(6), Some(9)));
        }
    }
}

/// Model a read failure at J6's persistence seam, without platform-specific permissions:
/// a committed README path, `readme_seen = true`, and no excerpt. J6 writes the same NULL
/// excerpt for the absent sibling, while §29's HEAD enumeration distinguishes their presence.
#[test]
fn ac_p3_31_18_unreadable_readme_passes_but_missing_readme_fails() {
    for exists in [true, false] {
        let (_dir, mut conn) = fresh();
        let repo = TestRepo::init();
        repo.write("src/main.rs", b"fn main() {}\n");
        if exists {
            repo.write("README.md", b"# fixture\n");
        }
        repo.commit("fixture");
        let actual = committed_presence(&repo);
        assert_eq!(
            actual.readme,
            if exists {
                PresenceState::Present
            } else {
                PresenceState::Absent
            }
        );
        let p = scorable(
            &conn,
            if exists {
                "unreadable-readme"
            } else {
                "missing-readme"
            },
        );
        content_scan(&conn, p, actual.ci.slug());
        conn.execute(
            "UPDATE project_content_scan SET has_readme = ?2, has_license = ?3, has_tests = ?4
             WHERE project_id = ?1",
            rusqlite::params![
                p,
                actual.readme.slug(),
                actual.license.slug(),
                actual.tests.slug()
            ],
        )
        .unwrap();
        let facts = ContentFacts {
            readme_path: exists.then(|| "README.md".to_owned()),
            readme_seen: true,
            readme_excerpt: None,
            ..ContentFacts::default()
        };
        let tx = conn.transaction().unwrap();
        j6_content::persist(&tx, ProjectId(p), &facts, 100).unwrap();
        evaluate_singletons(&tx, ProjectId(p), 100, &SqliteDebtStore).unwrap();
        evaluate_and_write(&tx, ProjectId(p), 100).unwrap();
        tx.commit().unwrap();

        // query_row must find a row; a missing row cannot masquerade as a NULL excerpt.
        let excerpt: Option<String> = conn
            .query_row(
                "SELECT readme_excerpt FROM peek_cache WHERE project_id = ?1",
                [p],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(excerpt, None);
        assert_eq!(
            check_state(&conn, p, "readme"),
            (if exists { "pass" } else { "fail" }.to_owned(), None)
        );
        assert_eq!(open_items(&conn, p, "missing_readme"), i64::from(!exists));
    }
    assert_no_completion_peek_cache_reads();
}

fn assert_no_completion_peek_cache_reads() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut pending = vec![root.join("completion")];
    let mut completion_files = 0;
    let mut offenders = Vec::new();
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            pending.extend(std::fs::read_dir(path).unwrap().map(|e| e.unwrap().path()));
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            completion_files += 1;
            if text.contains("peek_cache") {
                offenders.push(path.strip_prefix(&root).unwrap().display().to_string());
            }
        }
    }
    let singletons = std::fs::read_to_string(root.join("debt/singletons.rs")).unwrap();
    if singletons.contains("peek_cache") {
        offenders.push("debt/singletons.rs".to_owned());
    }
    eprintln!(
        "peek_cache gate: read {completion_files} completion files and 1 singleton file ({} total)",
        completion_files + 1
    );
    assert!(
        completion_files > 0,
        "source walk read zero completion files"
    );
    assert!(
        offenders.is_empty(),
        "completion reads peek_cache: {offenders:?}"
    );
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

    let stamps = |db: &rusqlite::Connection| -> Vec<i64> {
        let mut st = db
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

    let row = |db: &rusqlite::Connection, key: &str| -> (String, Option<i64>) {
        db.query_row(
            "SELECT state, user_na FROM project_check WHERE project_id = ?1 AND check_key = ?2",
            rusqlite::params![p, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    assert_eq!(row(&conn, "tests").0, "pass", "a library evaluates `tests`");

    // The user rules `tests` not applicable.
    let set = |db: &mut rusqlite::Connection, key: CompletionCheck, na: Option<bool>| {
        let tx = db.transaction().unwrap();
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

// ---------------------------------------------------------------------------------------------
// AC-P3-31-15 — the merge sweep deletes and recomputes, and the projection goes with the rows
// ---------------------------------------------------------------------------------------------

/// **`AC-P3-31-15`.** After a merge neither side holds a stale `project_check` row, and **both
/// sides' projections are NULL** — the half R131/F9 added, because the pair was in no merge class
/// at all and a survivor could carry a figure with zero rows behind it.
///
/// The class assignment is also read **off the source**, so `derived`, `not-recomputable` and
/// `reparented` are distinguished by the test and not by a comment.
#[test]
fn ac_p3_31_15_a_merge_clears_the_rows_and_the_projection_on_both_sides() {
    let (_dir, mut conn) = fresh();
    let survivor = scorable(&conn, "survivor");
    let absorbed = scorable(&conn, "absorbed");
    for id in [survivor, absorbed] {
        content_scan(&conn, id, "present");
        for source in [
            "missing_readme",
            "missing_license",
            "missing_tests",
            "ci_red",
            "unpushed_commits",
            "no_release",
        ] {
            sweep(&conn, id, source, "complete", Some(0));
        }
        recompute(&mut conn, id, 1_000);
        assert_ne!(projection(&conn, id), (None, None), "seeded");
    }

    let tx = conn.transaction().unwrap();
    recompute_derived(&tx, survivor, absorbed).unwrap();
    tx.commit().unwrap();

    // **Scoped to every project, not to every project with rows** — the old scoping is exactly
    // what let a survivor with zero rows and a non-null pair pass.
    let mut checked = 0_u32;
    for id in [survivor, absorbed] {
        let rows: i64 = conn
            .query_row(
                "SELECT count(*) FROM project_check WHERE project_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0, "project {id} kept a stale row across the merge");
        assert_eq!(
            projection(&conn, id),
            (None, None),
            "project {id} kept a projection with zero rows behind it"
        );
        checked += 1;
    }
    eprintln!("merge sides checked: {checked}");
    assert_eq!(checked, 2);

    // And the next settle recomputes the survivor.
    recompute(&mut conn, survivor, 2_000);
    let rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM project_check WHERE project_id = ?1",
            [survivor],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 10);
}

/// The three classes, read from the source rather than from a comment about it.
#[test]
fn ac_p3_31_15_project_check_is_derived_and_is_not_reparented() {
    let source = include_str!("../src/identity/merge.rs");
    let lines = source.lines().count();
    eprintln!("identity/merge.rs: {lines} lines read");
    assert!(lines > 0, "a run that read nothing is a failing run");

    let derived_list = source
        .split_once("for table in [")
        .map(|(_, rest)| rest.split_once("] {").map(|(list, _)| list))
        .and_then(|inner| inner)
        .expect("the derived list");
    assert!(
        derived_list.contains("\"project_check\""),
        "project_check is not in recompute_derived's derived list"
    );

    // `reparent_rows` rewrites `project_id` and never deletes. A table in both classes would be
    // deleted and then reparented, which is how earned XP gets double-counted or lost.
    let reparent = source
        .split_once("fn reparent_rows")
        .map(|(_, rest)| rest)
        .expect("reparent_rows");
    // Bounded at the next item declaration, so the slice is this function and not the rest of
    // the file — an unbounded slice would read `recompute_derived`'s list below and report the
    // table as reparented when it is only deleted.
    let end = ["\npub fn ", "\nfn ", "\npub(crate) fn "]
        .iter()
        .filter_map(|marker| reparent.find(marker))
        .min()
        .unwrap_or(reparent.len());
    let reparent_body = reparent
        .get(..end)
        .expect("the bound is a marker offset or the length");
    assert!(
        !reparent_body.contains("project_check"),
        "project_check is reparented as well as deleted, which is two classes at once"
    );
}

// ---------------------------------------------------------------------------------------------
// AC-P3-31-16 — `deps` reads the set and owns no item
// ---------------------------------------------------------------------------------------------

/// §32's lockfile scan, complete or not. **The verdict already folds the completeness**, which is
/// what `deps` reads rather than re-deriving.
fn dependency_scan(conn: &rusqlite::Connection, project: i64, complete: bool, matched: i64) {
    conn.execute(
        "INSERT INTO project_dependency_scan
            (project_id, observed_at, files_matched, dirs_entered, unresolved_manifests, complete)
         VALUES (?1, 50, ?2, 1, 0, ?3)
         ON CONFLICT(project_id) DO UPDATE SET complete = excluded.complete,
             files_matched = excluded.files_matched",
        rusqlite::params![project, matched, i64::from(complete)],
    )
    .unwrap();
}

fn advisory_item(conn: &rusqlite::Connection, project: i64, fingerprint: &str) {
    conn.execute(
        "INSERT INTO debt_item
            (project_id, subject_key, source, fingerprint, state, scoring,
             first_seen_at, last_seen_at)
         VALUES (?1, 'abc123', 'dependency_advisory', ?2, 'open', 'scored', 10, 10)",
        rusqlite::params![project, fingerprint],
    )
    .unwrap();
}

fn check_state(conn: &rusqlite::Connection, project: i64, key: &str) -> (String, Option<String>) {
    conn.query_row(
        "SELECT state, unknown_reason FROM project_check WHERE project_id = ?1 AND check_key = ?2",
        rusqlite::params![project, key],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

/// **`AC-P3-31-16`.** With two `scored` open advisories `deps` is `fail` and **`debt_item` holds
/// two rows, not one**; closing one leaves it `fail`; closing both makes it `pass` **only when
/// §32 reports the sweep complete**, and `unknown` otherwise.
///
/// **No `debt_item` row exists for the check key** — `deps` reads a set and owns no item (A8).
#[test]
fn ac_p3_31_16_deps_reads_the_advisory_set_and_owns_no_item() {
    let (_dir, mut conn) = fresh();
    let p = scorable(&conn, "deps");
    content_scan(&conn, p, "present");
    dependency_scan(&conn, p, true, 1);
    advisory_item(&conn, p, "GHSA-one");
    advisory_item(&conn, p, "GHSA-two");

    recompute(&mut conn, p, 1_000);
    assert_eq!(check_state(&conn, p, "deps").0, "fail");
    let items: i64 = conn
        .query_row(
            "SELECT count(*) FROM debt_item
              WHERE project_id = ?1 AND source = 'dependency_advisory'",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(items, 2, "two advisories are two items, not one");

    // Closing one leaves it failing: the predicate is *this set is empty*, not *this set shrank*.
    conn.execute(
        "DELETE FROM debt_item WHERE project_id = ?1 AND fingerprint = 'GHSA-one'",
        [p],
    )
    .unwrap();
    recompute(&mut conn, p, 2_000);
    assert_eq!(check_state(&conn, p, "deps").0, "fail");

    // Closing both makes it pass — **only because the sweep is complete**.
    conn.execute(
        "DELETE FROM debt_item WHERE project_id = ?1 AND source = 'dependency_advisory'",
        [p],
    )
    .unwrap();
    recompute(&mut conn, p, 3_000);
    assert_eq!(check_state(&conn, p, "deps").0, "pass");

    // The `unknown` branch runs too: zero advisories on an INCOMPLETE sweep is `unknown`, and a
    // pass without the completeness conjunct is the defect this half exists to catch.
    dependency_scan(&conn, p, false, 1);
    recompute(&mut conn, p, 4_000);
    let (state, reason) = check_state(&conn, p, "deps");
    assert_eq!(state, "unknown");
    assert_eq!(reason.as_deref(), Some("notSynced"));

    // **R131/F7**: a lockfile the read could not take is `notRead`, never `notSynced`.
    conn.execute(
        "INSERT INTO project_lockfile
            (project_id, source_path, ecosystem, read_state, observed_at)
         VALUES (?1, 'package-lock.json', 'npm', 'notRead', 50)",
        [p],
    )
    .unwrap();
    recompute(&mut conn, p, 5_000);
    assert_eq!(
        check_state(&conn, p, "deps"),
        ("unknown".to_owned(), Some("notRead".to_owned())),
        "a 17 MB lockfile is not a network problem, and the note must not say it is"
    );

    // **No item is ever keyed on the check.** `deps` is the only check owning none.
    let mis_keyed: i64 = conn
        .query_row(
            "SELECT count(*) FROM debt_item WHERE project_id = ?1 AND source = 'deps'",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(mis_keyed, 0);
}

// ---------------------------------------------------------------------------------------------
// AC-P3-31-14 — silent demotion
// ---------------------------------------------------------------------------------------------

fn count_for(conn: &rusqlite::Connection, table: &str, project: i64) -> i64 {
    conn.query_row(
        &format!("SELECT count(*) FROM {table} WHERE project_id = ?1"),
        [project],
        |r| r.get(0),
    )
    .unwrap()
}

/// **`AC-P3-31-14`, the core half.** A recompute that lowers the tier writes **no `health_delta`
/// row** across its transaction. A delta is a transition in the open debt set (A15) and never a
/// completion tick, so a demotion made by a check that owns no item — `ci`, whose workflow file
/// went away — moves the projection and nothing else. The recompute takes no event sink, so it
/// has no way to notify either; the renderer half asserts the frame plays nothing.
#[test]
fn ac_p3_31_14_a_demotion_writes_no_health_delta_row() {
    let (_dir, mut conn) = fresh();
    let p = scorable(&conn, "demoted");
    observed_completion_inputs(&conn, p);
    for source in [
        "missing_readme",
        "missing_license",
        "missing_tests",
        "ci_red",
        "unpushed_commits",
        "no_release",
    ] {
        sweep(&conn, p, source, "complete", Some(0));
    }
    content_scan(&conn, p, "present");
    recompute(&mut conn, p, 1_000);
    let (Some(lit_before), Some(evaluable_before)) = projection(&conn, p) else {
        panic!("the fixture must start measured");
    };
    // Every evaluable check passing is the top of §31.6's ladder, so anything lower is a demotion.
    assert_eq!(
        lit_before, evaluable_before,
        "the fixture must start at the top rung: {lit_before}/{evaluable_before}"
    );
    let deltas_before = count_for(&conn, "health_delta", p);
    let items_before = count_for(&conn, "debt_item", p);

    content_scan(&conn, p, "absent");
    let tx = conn.transaction().unwrap();
    let written = evaluate_and_write(&tx, ProjectId(p), 2_000).unwrap();
    // Counted inside the transaction as well, so a row written and then deleted before the commit
    // could not pass either.
    let deltas_in_tx = count_for(&tx, "health_delta", p);
    tx.commit().unwrap();

    let (Some(lit_after), Some(evaluable_after)) = projection(&conn, p) else {
        panic!("a demotion is still a measurement");
    };
    let deltas_after = count_for(&conn, "health_delta", p);
    let items_after = count_for(&conn, "debt_item", p);
    eprintln!(
        "AC-P3-31-14 completion {lit_before}/{evaluable_before} -> {lit_after}/{evaluable_after}; \
         health_delta rows {deltas_before} -> {deltas_in_tx} in the transaction -> \
         {deltas_after}; debt items {items_before} -> {items_after}"
    );

    assert!(
        matches!(written, Written::Rewritten { .. }),
        "the recompute must have run"
    );
    assert!(
        // `ci` fails and `ciGreen` stops applying: 10/10 is the top rung and 8/9 is below it.
        lit_after < evaluable_after,
        "the fixture must demote: {lit_before}/{evaluable_before} -> {lit_after}/{evaluable_after}"
    );
    assert_eq!(check_state(&conn, p, "ci").0, "fail");
    assert_eq!(
        items_after, items_before,
        "the demotion must move no item, or a delta would be owed"
    );
    assert_eq!(
        deltas_in_tx, deltas_before,
        "a demotion wrote a health_delta row"
    );
    assert_eq!(
        deltas_after, deltas_before,
        "a demotion wrote a health_delta row"
    );
}

/// Every event the runner emits, as `(topic, event)`.
#[derive(Debug, Default)]
struct RecordingSink(std::sync::Mutex<Vec<(String, String)>>);

impl codotheca_core::proto::EventSink for RecordingSink {
    fn emit(&self, topic: &str, event: &str, _payload: serde_json::Value) {
        self.0
            .lock()
            .unwrap()
            .push((topic.to_owned(), event.to_owned()));
    }
}

/// **The protocol's notification events, derived and never listed.** An event is a notification
/// when the schema's own comment on its payload type says it is one — which is how §32.12's alert
/// is declared — so a notification added later is covered without this test being edited.
fn notification_events() -> Vec<(String, String)> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root");
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.join("protocol/schema/protocol.json")).unwrap(),
    )
    .unwrap();
    let mut out = Vec::new();
    for (topic, events) in schema["topics"].as_object().expect("topics") {
        for (event, payload) in events.as_object().expect("events") {
            let comment = payload
                .as_str()
                .and_then(|ty| schema["types"][ty]["$comment"].as_str())
                .unwrap_or_default()
                .to_lowercase();
            if comment.contains("notification") {
                out.push((topic.clone(), event.clone()));
            }
        }
    }
    out
}

/// One project a real `JobRunner` can settle at the top rung: a real repository for the job to
/// read, the refstate facts `remote` and `release` read seeded on its copy, and every presence
/// answer `present`.
fn seed_demotable(index: &std::sync::Mutex<Index>, repo: &TestRepo) -> (i64, LocationId) {
    let guard = index.lock().unwrap();
    let conn = guard.conn();
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, remote_key,
                              authored_by_user, created_at, updated_at)
         VALUES ('p', 'p', 'abc123', 'forge/o/r', 1, 0, 0)",
        [],
    )
    .unwrap();
    let project = conn.last_insert_rowid();
    let path = repo.path().to_string_lossy().into_owned();
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                               store_key, presence, repo_kind, tag_count, refstate_observed_at)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree', 1, 50)",
        rusqlite::params![project, path.as_bytes(), path],
    )
    .unwrap();
    let location = LocationId(conn.last_insert_rowid());
    content_scan(conn, project, "present");
    drop(guard);
    (project, location)
}

/// The product's job runner over `index`, reporting to `sink`, with git pointed at `repo`.
fn real_runner(
    index: &std::sync::Arc<std::sync::Mutex<Index>>,
    repo: &TestRepo,
    sink: &std::sync::Arc<RecordingSink>,
) -> std::sync::Arc<codotheca_core::jobs::scheduler::JobRunner> {
    use codotheca_core::git::{GitSlots, SystemGit};
    use std::sync::Arc;

    let clock = Arc::new(codotheca_core::testing::FakeClock::new(1_700_000_000));
    let deps = codotheca_core::jobs::JobDeps {
        git: Arc::new(SystemGit::new(
            Arc::new(repo.exec()),
            Arc::new(GitSlots::new(4)),
            clock.clone(),
        )),
        clock,
        cancel: CancelToken::new(),
        tz_offset_min: 0,
    };
    codotheca_core::jobs::scheduler::JobRunner::new(Arc::clone(index), deps, sink.clone())
}

/// **`AC-P3-31-14`, through the real settle.** A demotion driven by a real `JobRunner` job — the
/// evaluator's first hook site, with the event sink the product gives it — emits no notification
/// and no `health_delta` event, the restoration surge's only transport, and writes no row.
///
/// The job is one that chains no other: a chained job would re-read the tree and overwrite the
/// facts this test moves.
#[test]
fn ac_p3_31_14_a_demotion_through_a_real_settle_emits_no_notification() {
    use codotheca_core::jobs::{Job, JobKind, JobOrigin, Priority};
    use std::sync::{Arc, Mutex};

    let notifications = notification_events();
    eprintln!("AC-P3-31-14 notification events derived from the protocol: {notifications:?}");
    assert!(
        !notifications.is_empty(),
        "the schema declares no notification, so an assertion over it proves nothing"
    );

    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.commit("first");
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open_at(dir.path(), 0).unwrap()));
    let (project, location) = seed_demotable(&index, &repo);
    let projected = |db: &Mutex<Index>| projection(db.lock().unwrap().conn(), project);
    let deltas = |db: &Mutex<Index>| count_for(db.lock().unwrap().conn(), "health_delta", project);

    let sink = Arc::new(RecordingSink::default());
    let runner = real_runner(&index, &repo, &sink);
    let job = Job {
        kind: JobKind::J2Status,
        project_id: ProjectId(project),
        location_id: location,
        store_key: "store".to_owned(),
        store_kind: codotheca_core::mount::StoreClass::Local,
        priority: Priority::Interactive,
        not_before: 0,
        origin: JobOrigin::Interactive,
    };
    let settle_until = |want: &dyn Fn((Option<i64>, Option<i64>)) -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while std::time::Instant::now() < deadline && !(runner.is_idle() && want(projected(&index)))
        {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    };

    assert!(runner.enqueue(job.clone()));
    runner.start(1);
    settle_until(&|(lit, evaluable)| lit.is_some() && lit == evaluable);
    let (Some(lit_before), Some(evaluable_before)) = projected(&index) else {
        panic!("the first settle computed no completion");
    };
    assert_eq!(
        lit_before, evaluable_before,
        "the fixture must start at the top rung: {lit_before}/{evaluable_before}"
    );
    let deltas_before = deltas(&index);
    sink.0.lock().unwrap().clear();

    // The CI config goes away: `ci` fails and `ciGreen` stops applying — a demotion that moves no
    // debt item, settled by the same real job.
    content_scan(index.lock().unwrap().conn(), project, "absent");
    assert!(runner.enqueue(job));
    settle_until(&|(lit, evaluable)| lit < evaluable);
    runner.request_stop();
    runner.join();

    let (Some(lit_after), Some(evaluable_after)) = projected(&index) else {
        panic!("a demotion is still a measurement");
    };
    let emitted = sink.0.lock().unwrap().clone();
    eprintln!(
        "AC-P3-31-14 real settle: completion {lit_before}/{evaluable_before} -> \
         {lit_after}/{evaluable_after}; health_delta rows {deltas_before} -> {}; events {emitted:?}",
        deltas(&index)
    );
    assert!(
        lit_after < evaluable_after,
        "the fixture must demote: {lit_before}/{evaluable_before} -> {lit_after}/{evaluable_after}"
    );
    assert!(
        !emitted.is_empty(),
        "the settle emitted nothing, so the sink observed nothing"
    );
    for event in &emitted {
        assert!(
            !notifications.contains(event),
            "a demotion emitted a notification: {event:?}"
        );
        assert_ne!(
            (event.0.as_str(), event.1.as_str()),
            ("projects", "health_delta"),
            "a demotion announced a restoration surge"
        );
    }
    assert_eq!(
        deltas(&index),
        deltas_before,
        "a demotion wrote a health_delta row"
    );
}
