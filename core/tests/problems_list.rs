#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §11.1's scan summary, read back through `problems::list`.

use codotheca_core::index::Index;
use codotheca_core::protocol::ProblemKind;
use codotheca_core::surfaces::problems;

fn seed(conn: &rusqlite::Connection) {
    conn.execute_batch(
        "INSERT INTO scan_run (id, generation, started_at, ended_at, mode, roots_json,
                               walked_dirs, found_repos, cancelled)
         VALUES (1, 1, 100, 200, 'full', '[]', 214903, 147, 0);
         INSERT INTO scan_problem (scan_run_id, kind, path_display, detail, count)
         VALUES (1, 'permission_denied', '<root>/a', 'EACCES', 3);
         INSERT INTO project (id, name, seed_basename, created_at, updated_at,
                              last_touched_at, ambiguous_lineage, lineage_key, remote_key)
         VALUES (10, 'alpha', 'alpha', 1, 1, 1, 1, 'L1', NULL),
                (11, 'bravo', 'bravo', 1, 1, 1, 0, 'L1', 'r/bravo'),
                (12, 'delta', 'delta', 1, 1, 1, 0, 'L1', 'r/delta'),
                (13, 'echo',  'echo',  1, 1, 1, 0, 'L1', 'r/echo');",
    )
    .expect("seed");
}

fn seeded(dir: &std::path::Path) -> Index {
    let index = Index::open(dir).expect("open");
    seed(index.conn());
    index
}

#[test]
fn the_eight_groups_come_back_in_section_11_1_order_and_an_empty_one_is_absent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());

    let out = problems::list(index.conn(), None).expect("list");

    let kinds: Vec<ProblemKind> = out.groups.iter().map(|g| g.kind).collect();
    assert_eq!(
        kinds,
        vec![ProblemKind::PermissionDenied, ProblemKind::AmbiguousLineage]
    );
    assert!(
        !kinds.contains(&ProblemKind::UntrustedRepo),
        "count = 0 removes the group; a permanent UNTRUSTED 0 teaches a healthy machine as a broken one"
    );
    // The fixed order is the order of GROUP_ORDER, whichever groups survive.
    let positions: Vec<usize> = kinds
        .iter()
        .map(|k| {
            problems::GROUP_ORDER
                .iter()
                .position(|g| g == k)
                .expect("known kind")
        })
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "groups keep §11.1's order"
    );
}

#[test]
fn the_ambiguous_group_names_two_candidates_and_counts_the_rest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());

    let out = problems::list(index.conn(), None).expect("list");
    let group = out
        .groups
        .iter()
        .find(|g| g.kind == ProblemKind::AmbiguousLineage)
        .expect("ambiguous group");
    assert_eq!(group.count, 1);
    let item = group.items.first().expect("one row");
    assert_eq!(
        item.candidate_project_ids.len(),
        3,
        "every candidate is counted"
    );
    assert_eq!(
        item.candidate_names,
        vec!["bravo".to_owned(), "delta".to_owned()]
    );
    assert_eq!(item.project_id.map(|p| p.0), Some(10));
}

#[test]
fn a_candidate_that_loses_its_remote_stops_making_the_project_ambiguous() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());
    index
        .conn()
        .execute(
            "UPDATE project SET remote_key = NULL WHERE id IN (12, 13)",
            [],
        )
        .expect("drop remotes");

    let out = problems::list(index.conn(), None).expect("list");
    assert!(
        out.groups
            .iter()
            .all(|g| g.kind != ProblemKind::AmbiguousLineage),
        "the group is a live query re-evaluated each open; one candidate is not ambiguity"
    );
    assert_eq!(out.header.ambiguous_lineage_count, Some(0));
}

#[test]
fn a_running_scan_omits_both_clauses_rather_than_zeroing_them() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());
    index
        .conn()
        .execute("UPDATE scan_run SET ended_at = NULL WHERE id = 1", [])
        .expect("re-open the run");

    let out = problems::list(index.conn(), None).expect("list");
    assert_eq!(
        out.header.problem_count, None,
        "a scan that has not finished has found no problems yet"
    );
    assert_eq!(out.header.ambiguous_lineage_count, None);
    assert_eq!(out.header.walked_dirs, 214_903);
    assert_eq!(out.header.repositories, 147);
}

#[test]
fn deferred_slow_reads_project_job_state_and_not_scan_problem() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());
    index
        .conn()
        .execute_batch(
            "INSERT INTO location (id, project_id, kind, distro, path_bytes, path_key,
                                   path_display, volume_key, store_key, presence, repo_kind,
                                   last_seen_at)
             VALUES (1, 11, 'linux', '', X'2f61', X'2f61', '<root>/bravo', 'v', 's', 'present',
                     'worktree', 190);
             INSERT INTO project_job_state (project_id, job, state, fail_count, reason, at)
             VALUES (11, 'j3', 'deferred_slow', 1, 'over budget', 150);",
        )
        .expect("defer a job");

    let out = problems::list(index.conn(), None).expect("list");
    let group = out
        .groups
        .iter()
        .find(|g| g.kind == ProblemKind::DeferredSlow)
        .expect("deferred-slow group");
    let item = group.items.first().expect("one row");
    assert_eq!(item.path_display.as_str(), "<root>/bravo");
    assert_eq!(
        item.last_seen_at,
        Some(190),
        "the wire field exists so an offline row can date its observation; NULL is absent, never zero"
    );
}

#[test]
fn a_scan_problem_row_has_no_location_so_its_observation_time_is_absent_not_zero() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = seeded(dir.path());

    let out = problems::list(index.conn(), None).expect("list");
    let group = out
        .groups
        .iter()
        .find(|g| g.kind == ProblemKind::PermissionDenied)
        .expect("permission-denied group");
    assert_eq!(
        group.items.first().and_then(|i| i.last_seen_at),
        None,
        "scan_problem carries no location_id, so the observation time is absent rather than 0"
    );
}

#[test]
fn only_the_six_spellings_the_check_constraint_admits_map_to_a_kind() {
    for (stored, kind) in [
        ("permission_denied", ProblemKind::PermissionDenied),
        ("untrusted_repo", ProblemKind::UntrustedRepo),
        ("unreadable_repo", ProblemKind::UnreadableRepo),
        ("clock_skew", ProblemKind::ClockSkew),
        ("non_utf8_path", ProblemKind::NonUtf8Path),
        ("offline_store", ProblemKind::OfflineStore),
    ] {
        assert_eq!(problems::problem_kind_from_storage(stored), Some(kind));
    }
    assert_eq!(
        problems::problem_kind_from_storage("deferred_slow"),
        None,
        "deferred-slow lives in project_job_state, not scan_problem"
    );
    assert_eq!(
        problems::problem_kind_from_storage("ambiguous_lineage"),
        None,
        "ambiguity is a live query over project, not a stored problem"
    );
    assert_eq!(
        problems::problem_kind_from_storage("untrusted"),
        None,
        "migration 0005's CHECK admits only the wire spellings, so a short one cannot be stored"
    );
}
