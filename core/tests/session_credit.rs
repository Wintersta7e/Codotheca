#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Two assertions no unit test can make, because both are about the *shape* of the session
//! module rather than a value it computes.
//!
//! **Two ledgers, never merged (§9, §8.5.5).** Git-derived facts recompute from history;
//! playtime counts only what this app launched. They are displayed side by side and never
//! summed. The way that invariant dies is not in the chart — it is a `JOIN` in the session store
//! that makes a merged number expressible at all, and a grep over this module's source is the
//! cheapest place to catch it.

use std::path::PathBuf;

use codotheca_core::protocol::{CloseReason, ClosedBy, LocationId, ProjectId, TargetId};
use codotheca_core::session::segment::ClosedSegment;
use codotheca_core::session::store::{self, SessionOpen};
use codotheca_core::testing::TempIndex;

const T0: i64 = 1_700_000_000;

/// Every `.rs` file in `core/src/session`, with its text.
fn session_sources() -> Vec<(PathBuf, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("session");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{} must exist: {e}", dir.display()))
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            out.push((path, text));
        }
    }
    out
}

// A run that scanned nothing would pass both greps below while proving nothing — the failure
// shape this project keeps hitting. The module has seven files and cannot shrink below them.
#[test]
fn the_audit_actually_reads_the_module() {
    let sources = session_sources();
    assert!(
        sources.len() >= 7,
        "the audit found only {} source files; a gate that scans nothing is a failing gate",
        sources.len()
    );
    let bytes: usize = sources.iter().map(|(_, text)| text.len()).sum();
    assert!(bytes > 10_000, "only {bytes} bytes of source scanned");
}

#[test]
fn the_session_ledger_never_reads_the_git_ledger() {
    // A session store that folds in git-derived facts is what makes a blended figure
    // expressible in the first place.
    for (path, text) in session_sources() {
        for banned in [
            "xp_events",
            "commit_day",
            "last_commit_at",
            "last_user_commit_at",
        ] {
            assert!(
                !text.contains(banned),
                "{}: the session ledger must not read `{banned}`",
                path.display()
            );
        }
    }
}

#[test]
fn nothing_here_counts_lines_or_commits() {
    // Never reward volume: J4 produces commit-days and nothing in this schema stores a count.
    for (path, text) in session_sources() {
        for banned in ["commit_count", "line_count", "lines_added", "lines_changed"] {
            assert!(!text.contains(banned), "{}: {banned}", path.display());
        }
    }
}

struct Ledgers {
    index: TempIndex,
    project: ProjectId,
    untouched: ProjectId,
}

fn commit_days(index: &TempIndex, project: ProjectId, days: usize) {
    for day in 0..days {
        index
            .index()
            .conn()
            .execute(
                "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track)
                 VALUES (?1, ?2, 'subject', 'commit_day', ?3, 'git')",
                rusqlite::params![
                    T0 + i64::try_from(day).unwrap() * 86_400,
                    project.0,
                    format!("commit_day:{}:{day}", project.0)
                ],
            )
            .unwrap();
    }
}

fn target(index: &TempIndex, project: ProjectId, location: LocationId) -> TargetId {
    index
        .index()
        .conn()
        .execute(
            "INSERT INTO launch_target (project_id, location_id, kind, name, exec_bytes,
                                        sort_index)
             VALUES (?1, ?2, 'editor', 'an editor', X'2F65', 0)",
            rusqlite::params![project.0, location.0],
        )
        .unwrap();
    TargetId(index.index().conn().last_insert_rowid())
}

/// One project with four commit-days *and* an hour of playtime; a second with commit-days and
/// no session at all.
fn fixture_with_commit_days_and_sessions() -> Ledgers {
    let mut index = TempIndex::new();
    let project = index.insert_project();
    let untouched = index.insert_project();
    let location = index.insert_location(project, "/w");
    let tgt = target(&index, project, location);
    commit_days(&index, project, 4);
    commit_days(&index, untouched, 4);

    let tx = index.index_mut().conn_mut().transaction().unwrap();
    let session = store::open_session(
        &tx,
        &SessionOpen {
            project_id: project,
            location_id: Some(location),
            target_id: Some(tgt),
            started_at: T0,
        },
    )
    .unwrap();
    let segment = store::open_segment(&tx, session, T0).unwrap();
    store::close_segment(
        &tx,
        segment,
        &ClosedSegment {
            started_at: T0,
            ended_at: T0 + 3_600,
            credited_seconds: 3_600,
            closed_by: ClosedBy::SessionEnd,
        },
    )
    .unwrap();
    store::close_session(&tx, session, T0 + 3_600, CloseReason::Stop, T0 + 3_600).unwrap();
    tx.commit().unwrap();

    Ledgers {
        index,
        project,
        untouched,
    }
}

impl Ledgers {
    fn commit_days(&self, project: ProjectId) -> i64 {
        self.index
            .index()
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM xp_events
                  WHERE project_id = ?1 AND kind = 'commit_day' AND track = 'git'",
                [project.0],
                |r| r.get(0),
            )
            .unwrap()
    }
}

#[test]
fn playtime_and_commit_days_are_reported_side_by_side_and_never_summed() {
    // The two numbers come from two functions over two tables, and there is no function
    // anywhere that returns their sum.
    let h = fixture_with_commit_days_and_sessions();
    assert_eq!(
        store::playtime_seconds(h.index.index().conn(), h.project).unwrap(),
        3_600
    );
    assert_eq!(
        h.commit_days(h.project),
        4,
        "the git ledger is unchanged by the session ledger"
    );
}

#[test]
fn a_project_that_was_never_launched_has_a_playtime_of_zero_and_that_zero_is_real() {
    // Never render unknown as zero -- and its mirror: this zero is *known*. No session row
    // exists, so "0 seconds of launched time" is a fact, not an absence of computation. The
    // project has four commit-days, so it is emphatically not an untouched project.
    let h = fixture_with_commit_days_and_sessions();
    assert_eq!(
        store::playtime_seconds(h.index.index().conn(), h.untouched).unwrap(),
        0
    );
    assert_eq!(h.commit_days(h.untouched), 4);
}

#[test]
fn a_session_on_one_project_credits_no_other_project() {
    let h = fixture_with_commit_days_and_sessions();
    let total: i64 = h
        .index
        .index()
        .conn()
        .query_row(
            "SELECT COALESCE(SUM(credited_seconds), 0) FROM session",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        total,
        store::playtime_seconds(h.index.index().conn(), h.project).unwrap(),
        "every credited second in the database belongs to the one project that was launched"
    );
}
