#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §9's ledger: `credited_seconds` is the sum of the segments' credits, whatever ends the
//! session, and the durable watermark on the open segment is what a crash can still credit.

use codotheca_core::protocol::{CloseReason, ClosedBy, LocationId, ProjectId, SessionId, TargetId};
use codotheca_core::session::segment::ClosedSegment;
use codotheca_core::session::store::{self, SessionOpen};
use codotheca_core::session::{close_reason_str, closed_by_str};
use codotheca_core::testing::TempIndex;

const T0: i64 = 1_700_000_000;

/// A launch target scoped to one project, so `session.target_id` has something to point at.
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

struct Fixture {
    index: TempIndex,
    project: ProjectId,
    location: LocationId,
    target: TargetId,
}

fn fixture() -> Fixture {
    let index = TempIndex::new();
    let project = index.insert_project();
    let location = index.insert_location(project, "/w");
    let target = target(&index, project, location);
    Fixture {
        index,
        project,
        location,
        target,
    }
}

impl Fixture {
    fn open_session(&mut self) -> SessionId {
        let tx = self.index.index_mut().conn_mut().transaction().unwrap();
        let id = store::open_session(
            &tx,
            &SessionOpen {
                project_id: self.project,
                location_id: Some(self.location),
                target_id: Some(self.target),
                started_at: T0,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        id
    }
}

#[test]
fn credited_seconds_is_the_sum_of_segments_whatever_ended_the_session() {
    let mut h = fixture();
    let s = h.open_session();

    let tx = h.index.index_mut().conn_mut().transaction().unwrap();
    for (start, end) in [(T0, T0 + 1_200), (T0 + 2_400, T0 + 3_000)] {
        let seg = store::open_segment(&tx, s, start).unwrap();
        store::close_segment(
            &tx,
            seg,
            &ClosedSegment {
                started_at: start,
                ended_at: end,
                credited_seconds: end - start,
                closed_by: ClosedBy::Idle,
            },
        )
        .unwrap();
    }
    // The session closes eight hours later; the end mechanism must not decide the credit.
    let closed = store::close_session(&tx, s, T0 + 28_800, CloseReason::Stop, T0 + 28_800).unwrap();
    tx.commit().unwrap();

    assert_eq!(
        closed.credited_seconds, 1_800,
        "1200 + 600, not the 8-hour span"
    );
    assert_eq!(
        store::credited_seconds(h.index.index().conn(), s).unwrap(),
        1_800
    );
    assert_eq!(
        store::playtime_seconds(h.index.index().conn(), h.project).unwrap(),
        1_800
    );
}

#[test]
fn the_stored_total_and_the_live_sum_agree_once_the_session_is_closed() {
    // One value with two homes is this repo's most-repeated defect. `session.credited_seconds`
    // is a materialised copy of the segment sum, so the two must be read back equal, and
    // `playtime_seconds` -- which reads the column, not the segments -- must agree with both.
    let mut h = fixture();
    let s = h.open_session();

    let tx = h.index.index_mut().conn_mut().transaction().unwrap();
    let seg = store::open_segment(&tx, s, T0).unwrap();
    store::close_segment(
        &tx,
        seg,
        &ClosedSegment {
            started_at: T0,
            ended_at: T0 + 742,
            credited_seconds: 742,
            closed_by: ClosedBy::Idle,
        },
    )
    .unwrap();
    store::close_session(&tx, s, T0 + 742, CloseReason::Idle, T0 + 742).unwrap();
    tx.commit().unwrap();

    let conn = h.index.index().conn();
    let column: i64 = conn
        .query_row(
            "SELECT credited_seconds FROM session WHERE id = ?1",
            [s.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(column, 742);
    assert_eq!(store::credited_seconds(conn, s).unwrap(), 742);
    assert_eq!(store::session_ref(conn, s).unwrap().credited_seconds, 742);
    assert_eq!(store::playtime_seconds(conn, h.project).unwrap(), 742);
}

#[test]
fn an_open_segment_carries_a_durable_watermark() {
    let mut h = fixture();
    let s = h.open_session();

    let tx = h.index.index_mut().conn_mut().transaction().unwrap();
    let seg = store::open_segment(&tx, s, T0).unwrap();
    store::mark_activity(&tx, seg, T0 + 300).unwrap();
    tx.commit().unwrap();

    let rows = store::open_sessions(h.index.index().conn()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows.first().unwrap().open_segment_id, Some(seg));
    assert_eq!(
        rows.first().unwrap().last_activity_at,
        Some(T0 + 300),
        "a crash after this point must still credit the 300 seconds observed"
    );

    // The watermark lives in `credited_seconds` alone: §1.6 puts
    // `CHECK ((ended_at IS NULL) = (closed_by IS NULL))` on `session_segment`, so an open row
    // cannot carry an `ended_at`. Nothing is lost -- `ended_at` is `started_at + credit`.
    let (ended, credited): (Option<i64>, i64) = h
        .index
        .index()
        .conn()
        .query_row(
            "SELECT ended_at, credited_seconds FROM session_segment WHERE id = ?1",
            [seg],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((ended, credited), (None, 300));
}

#[test]
fn a_watermark_never_moves_backwards_or_credits_a_negative_span() {
    let mut h = fixture();
    let s = h.open_session();

    let tx = h.index.index_mut().conn_mut().transaction().unwrap();
    let seg = store::open_segment(&tx, s, T0).unwrap();
    store::mark_activity(&tx, seg, T0 - 900).unwrap();
    tx.commit().unwrap();

    let credited: i64 = h
        .index
        .index()
        .conn()
        .query_row(
            "SELECT credited_seconds FROM session_segment WHERE id = ?1",
            [seg],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(credited, 0, "a wall clock that moved back credits nothing");
}

#[test]
fn a_closed_segment_is_not_reopened_by_a_late_watermark_or_a_second_close() {
    let mut h = fixture();
    let s = h.open_session();

    let tx = h.index.index_mut().conn_mut().transaction().unwrap();
    let seg = store::open_segment(&tx, s, T0).unwrap();
    store::close_segment(
        &tx,
        seg,
        &ClosedSegment {
            started_at: T0,
            ended_at: T0 + 600,
            credited_seconds: 600,
            closed_by: ClosedBy::SessionEnd,
        },
    )
    .unwrap();
    // A drained activity batch can arrive after the machine has already closed the segment.
    store::mark_activity(&tx, seg, T0 + 9_000).unwrap();
    store::close_segment(
        &tx,
        seg,
        &ClosedSegment {
            started_at: T0,
            ended_at: T0 + 9_000,
            credited_seconds: 9_000,
            closed_by: ClosedBy::Crash,
        },
    )
    .unwrap();
    tx.commit().unwrap();

    assert_eq!(
        store::credited_seconds(h.index.index().conn(), s).unwrap(),
        600,
        "both writes are guarded on closed_by IS NULL"
    );
}

#[test]
fn closing_a_session_moves_last_interaction_at() {
    // §5.1: last_interaction_at = max(reflog tail, last session end, worktree newest mtime).
    let mut h = fixture();
    let s = h.open_session();

    let tx = h.index.index_mut().conn_mut().transaction().unwrap();
    let seg = store::open_segment(&tx, s, T0).unwrap();
    store::close_segment(
        &tx,
        seg,
        &ClosedSegment {
            started_at: T0,
            ended_at: T0 + 600,
            credited_seconds: 600,
            closed_by: ClosedBy::SessionEnd,
        },
    )
    .unwrap();
    store::close_session(&tx, s, T0 + 600, CloseReason::Stop, T0 + 600).unwrap();
    tx.commit().unwrap();

    assert_eq!(
        h.index.project_i64("last_interaction_at", h.project),
        Some(T0 + 600)
    );
}

#[test]
fn a_session_ref_reads_back_exactly_what_was_written() {
    let mut h = fixture();
    let s = h.open_session();
    let r = store::session_ref(h.index.index().conn(), s).unwrap();
    assert_eq!(r.id, s);
    assert_eq!(r.project_id, h.project);
    assert_eq!(r.location_id, h.location);
    assert_eq!(r.target_id, h.target);
    assert_eq!(r.started_at, T0);
    assert_eq!(r.ended_at, None);
    assert_eq!(r.close_reason, None);
    assert_eq!(
        r.credited_seconds, 0,
        "zero here is a real zero: no segment has closed"
    );
}

#[test]
fn a_session_ref_for_a_session_that_is_not_there_names_the_session() {
    let h = fixture();
    let err = store::session_ref(h.index.index().conn(), SessionId(4_242)).unwrap_err();
    assert!(matches!(
        err,
        codotheca_core::session::SessionError::NoSuchSession(4_242)
    ));
}

#[test]
fn every_column_spelling_this_module_writes_is_one_the_ddl_check_accepts() {
    // One value with two homes: §1.6's CHECK lists the spellings and `close_reason_str` /
    // `closed_by_str` produce them. This is the test that reads the other side, so the two
    // cannot drift without a red suite.
    let mut h = fixture();
    for reason in [
        CloseReason::Stop,
        CloseReason::Idle,
        CloseReason::ProcessExit,
        CloseReason::AppExit,
        CloseReason::Crash,
        CloseReason::Orphaned,
    ] {
        let s = h.open_session();
        let tx = h.index.index_mut().conn_mut().transaction().unwrap();
        store::close_session(&tx, s, T0 + 1, reason, T0 + 1)
            .unwrap_or_else(|err| panic!("the DDL rejected {}: {err}", close_reason_str(reason)));
        tx.commit().unwrap();
        assert_eq!(
            store::session_ref(h.index.index().conn(), s)
                .unwrap()
                .close_reason,
            Some(reason)
        );
    }

    let s = h.open_session();
    for closed_by in [
        ClosedBy::Idle,
        ClosedBy::SessionEnd,
        ClosedBy::AppExit,
        ClosedBy::Crash,
    ] {
        let tx = h.index.index_mut().conn_mut().transaction().unwrap();
        let seg = store::open_segment(&tx, s, T0).unwrap();
        store::close_segment(
            &tx,
            seg,
            &ClosedSegment {
                started_at: T0,
                ended_at: T0 + 1,
                credited_seconds: 1,
                closed_by,
            },
        )
        .unwrap_or_else(|err| panic!("the DDL rejected {}: {err}", closed_by_str(closed_by)));
        tx.commit().unwrap();
    }
}

#[test]
fn playtime_counts_only_launched_sessions() {
    // Two ledgers, never merged (§9). A project with git history and no session has no playtime.
    let h = fixture();
    h.index.set_project_i64(h.project, "last_commit_at", T0);
    assert_eq!(
        store::playtime_seconds(h.index.index().conn(), h.project).unwrap(),
        0
    );
}
