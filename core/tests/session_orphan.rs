#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §9 and §11.2: a session left open by a crash is closed at its last observed activity, never
//! silently extended to now.

use codotheca_core::protocol::{CloseReason, ClosedBy, LocationId, ProjectId, SessionId, TargetId};
use codotheca_core::session::orphan;
use codotheca_core::session::segment::ClosedSegment;
use codotheca_core::session::store::{self, SessionOpen};
use codotheca_core::testing::TempIndex;

const T0: i64 = 1_700_000_000;
const THREE_DAYS: i64 = 259_200;

struct Fixture {
    index: TempIndex,
    project: ProjectId,
    location: LocationId,
    target: TargetId,
    session: SessionId,
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

fn fixture_with_no_open_session() -> Fixture {
    let mut index = TempIndex::new();
    let project = index.insert_project();
    let location = index.insert_location(project, "/w");
    let tgt = target(&index, project, location);
    // A job has to have succeeded for §5.4's band to be computed at all, so that closing the
    // session can move a condition rather than leaving one uncomputed.
    index.mark_job_done(project, codotheca_core::jobs::JobKind::J4History);

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
    store::close_session(&tx, session, T0, CloseReason::Stop, T0).unwrap();
    tx.commit().unwrap();

    Fixture {
        index,
        project,
        location,
        target: tgt,
        session,
    }
}

fn fixture() -> Fixture {
    let mut h = fixture_with_no_open_session();
    h.session = h.open_session(T0);
    h
}

impl Fixture {
    /// A session left open, as a crash would leave it.
    fn open_session(&mut self, started_at: i64) -> SessionId {
        let open = SessionOpen {
            project_id: self.project,
            location_id: Some(self.location),
            target_id: Some(self.target),
            started_at,
        };
        let tx = self.index.index_mut().conn_mut().transaction().unwrap();
        let id = store::open_session(&tx, &open).unwrap();
        tx.commit().unwrap();
        id
    }

    fn open_segment(&mut self, at: i64) -> i64 {
        let session = self.session;
        let tx = self.index.index_mut().conn_mut().transaction().unwrap();
        let id = store::open_segment(&tx, session, at).unwrap();
        tx.commit().unwrap();
        id
    }

    fn mark_activity(&mut self, segment: i64, at: i64) {
        let tx = self.index.index_mut().conn_mut().transaction().unwrap();
        store::mark_activity(&tx, segment, at).unwrap();
        tx.commit().unwrap();
    }

    fn close_segment(&mut self, start: i64, end: i64) {
        let session = self.session;
        let tx = self.index.index_mut().conn_mut().transaction().unwrap();
        let id = store::open_segment(&tx, session, start).unwrap();
        store::close_segment(
            &tx,
            id,
            &ClosedSegment {
                started_at: start,
                ended_at: end,
                credited_seconds: end - start,
                closed_by: ClosedBy::Idle,
            },
        )
        .unwrap();
        tx.commit().unwrap();
    }

    fn segment_closed_by(&self, segment: i64) -> String {
        self.index
            .index()
            .conn()
            .query_row(
                "SELECT closed_by FROM session_segment WHERE id = ?1",
                [segment],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn last_interaction_at(&self) -> Option<i64> {
        self.index.project_i64("last_interaction_at", self.project)
    }

    fn ended_at(&self) -> Option<i64> {
        store::session_ref(self.index.index().conn(), self.session)
            .unwrap()
            .ended_at
    }

    fn close_reason(&self) -> Option<String> {
        self.index
            .index()
            .conn()
            .query_row(
                "SELECT close_reason FROM session WHERE id = ?1",
                [self.session.0],
                |r| r.get(0),
            )
            .unwrap()
    }
}

#[test]
fn an_orphaned_session_is_closed_at_its_last_observed_activity_not_at_now() {
    // Criterion 11, clause 4.
    let mut h = fixture();
    let seg = h.open_segment(T0);
    h.mark_activity(seg, T0 + 600); // the watermark the crash left behind

    let report = orphan::close_orphans(h.index.index_mut(), T0 + THREE_DAYS).unwrap();
    assert_eq!((report.sessions_closed, report.segments_closed), (1, 1));
    assert_eq!(report.credited_seconds, 600);

    assert_eq!(h.close_reason().as_deref(), Some("orphaned"));
    assert_eq!(h.ended_at(), Some(T0 + 600), "not extended to now");
    assert_eq!(
        store::credited_seconds(h.index.index().conn(), h.session).unwrap(),
        600
    );
    assert_eq!(
        h.last_interaction_at(),
        Some(T0 + 600),
        "a crash three days ago must not light the tile today"
    );
}

#[test]
fn a_completed_segment_and_the_open_one_are_both_credited() {
    // §9: orphan recovery credits completed segments AND the final open segment up to its last
    // observed activity, rather than discarding it.
    let mut h = fixture();
    h.close_segment(T0, T0 + 1_200);
    let seg = h.open_segment(T0 + 2_400);
    h.mark_activity(seg, T0 + 2_700);

    let report = orphan::close_orphans(h.index.index_mut(), T0 + THREE_DAYS).unwrap();
    assert_eq!(report.credited_seconds, 1_500);
    assert_eq!(
        store::credited_seconds(h.index.index().conn(), h.session).unwrap(),
        1_500
    );
    assert_eq!(h.ended_at(), Some(T0 + 2_700), "the latest segment end");
}

#[test]
fn a_segment_that_never_saw_activity_credits_nothing_rather_than_a_guess() {
    let mut h = fixture();
    h.open_segment(T0);
    orphan::close_orphans(h.index.index_mut(), T0 + THREE_DAYS).unwrap();
    assert_eq!(
        store::credited_seconds(h.index.index().conn(), h.session).unwrap(),
        0
    );
    assert_eq!(h.ended_at(), Some(T0));
}

#[test]
fn the_open_segment_is_marked_crash_and_the_session_orphaned() {
    let mut h = fixture();
    let seg = h.open_segment(T0);
    h.mark_activity(seg, T0 + 60);
    orphan::close_orphans(h.index.index_mut(), T0 + THREE_DAYS).unwrap();
    assert_eq!(
        h.segment_closed_by(seg),
        "crash",
        "§1.6's closed_by has no `orphaned` value"
    );
    assert_eq!(h.close_reason().as_deref(), Some("orphaned"));
}

#[test]
fn recovery_is_idempotent_and_leaves_a_closed_session_alone() {
    let mut h = fixture();
    let seg = h.open_segment(T0);
    h.mark_activity(seg, T0 + 600);
    orphan::close_orphans(h.index.index_mut(), T0 + THREE_DAYS).unwrap();
    let second = orphan::close_orphans(h.index.index_mut(), T0 + THREE_DAYS + 10).unwrap();
    assert_eq!((second.sessions_closed, second.credited_seconds), (0, 0));
    assert_eq!(h.ended_at(), Some(T0 + 600));
}

#[test]
fn a_clean_database_closes_nothing() {
    let mut h = fixture_with_no_open_session();
    let report = orphan::close_orphans(h.index.index_mut(), T0).unwrap();
    assert_eq!(report.sessions_closed, 0);
    assert_eq!(report.segments_closed, 0);
}

#[test]
fn every_orphaned_session_is_closed_not_only_the_first() {
    // A loop that returned after the first row would pass every test above.
    let mut h = fixture();
    let first = h.session;
    let seg = h.open_segment(T0);
    h.mark_activity(seg, T0 + 300);

    let second = h.open_session(T0 + 5_000);
    let tx = h.index.index_mut().conn_mut().transaction().unwrap();
    let seg2 = store::open_segment(&tx, second, T0 + 5_000).unwrap();
    store::mark_activity(&tx, seg2, T0 + 5_400).unwrap();
    tx.commit().unwrap();

    let report = orphan::close_orphans(h.index.index_mut(), T0 + THREE_DAYS).unwrap();
    assert_eq!((report.sessions_closed, report.segments_closed), (2, 2));
    assert_eq!(report.credited_seconds, 300 + 400);
    for session in [first, second] {
        assert!(
            store::session_ref(h.index.index().conn(), session).is_ok_and(|r| r.ended_at.is_some()),
            "session {session:?} was left open"
        );
    }
    assert!(store::open_sessions(h.index.index().conn())
        .unwrap()
        .is_empty());
}

#[test]
fn recovery_runs_before_any_session_of_this_process_and_says_how_much_it_recovered() {
    // §11.2 puts this in the startup lane, before a session can be opened in this process, so
    // every `ended_at IS NULL` row it finds is from a previous one. The report is what the lane
    // logs; a report that always said zero would hide a recovery that silently did nothing.
    let mut h = fixture();
    let seg = h.open_segment(T0);
    h.mark_activity(seg, T0 + 900);
    let report = orphan::close_orphans(h.index.index_mut(), T0 + THREE_DAYS).unwrap();
    assert_eq!(report.credited_seconds, 900);
    assert_eq!(report.sessions_closed, 1);
}
