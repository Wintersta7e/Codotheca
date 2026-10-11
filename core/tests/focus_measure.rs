//! §39.3's Focus measure through the session store and the manager that write it: which segments
//! count, the wall-clock union across projects, the local day's bounds, and the launch that is
//! not input (§39.3a).

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::{Arc, Mutex};

use codotheca_core::clock::{day_bounds, LocalZone};
use codotheca_core::git::{RepoHandle, StoreKey};
use codotheca_core::index::Index;
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{CloseReason, ClosedBy, LocationId, ProjectId, SessionId, TargetId};
use codotheca_core::session::activity::FakeIgnoreCheck;
use codotheca_core::session::manager::{LaunchedSession, SessionManager};
use codotheca_core::session::measure::{
    focus_seconds, is_counted, marginal_seconds, DatedContribution,
};
use codotheca_core::session::segment::ClosedSegment;
use codotheca_core::session::store::{self, SessionOpen};
use codotheca_core::session::watch::{ActivityBatch, ActivitySource, FakeActivitySource};
use codotheca_core::session::{
    SessionError, DEFAULT_TICK_SECS, FOCUS_HEARTBEAT_SECS, ONE_TOUCH_CREDIT_BOUND_SECS,
    SEGMENT_IDLE_SECS,
};
use codotheca_core::testing::zone::{FixedZone, TransitionZone};
use codotheca_core::testing::{FakeClock, TempIndex};

/// A UTC midnight: local day [`DAY`] under [`UTC`] starts here.
const MIDNIGHT: i64 = 1_700_006_400;
/// The local day number of [`MIDNIGHT`].
const DAY: i64 = MIDNIGHT / 86_400;
/// One in the morning of [`DAY`]: every manager-driven session here stays inside that day.
const T0: i64 = MIDNIGHT + 3_600;
/// A reading long after every segment here.
const LATER: i64 = MIDNIGHT + 10 * 86_400;
const UTC: FixedZone = FixedZone(0);

/// Every payload the manager emits is checked against the schema, then dropped.
struct NoEvents;

impl EventSink for NoEvents {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        codotheca_core::testing::events::validated(topic, event, &payload);
    }
}

/// The manager takes `Box<dyn ActivitySource>`; the test keeps a handle to stand in for the OS.
#[derive(Debug, Clone, Default)]
struct SharedActivity(Arc<Mutex<FakeActivitySource>>);

impl ActivitySource for SharedActivity {
    fn watch(&mut self, session: SessionId, root: &std::path::Path) -> Result<(), SessionError> {
        self.0
            .lock()
            .map_err(|_| SessionError::Watch("the shared activity lock is poisoned".to_owned()))?
            .watch(session, root)
    }
    fn unwatch(&mut self, session: SessionId) {
        self.0.lock().unwrap().unwatch(session);
    }
    fn drain(&mut self) -> Vec<ActivityBatch> {
        self.0.lock().unwrap().drain()
    }
}

struct Rig {
    index: TempIndex,
    manager: SessionManager,
    clock: Arc<FakeClock>,
    activity: SharedActivity,
    alpha: ProjectId,
    beta: ProjectId,
    loc: LocationId,
    other_loc: LocationId,
    target: TargetId,
}

fn rig() -> Rig {
    let index = TempIndex::new();
    let alpha = index.insert_project();
    let beta = index.insert_project();
    let loc = index.insert_location(alpha, "/w/alpha");
    let other_loc = index.insert_location(beta, "/w/beta");
    index
        .index()
        .conn()
        .execute(
            "INSERT INTO launch_target (project_id, location_id, kind, name, exec_bytes,
                                        sort_index)
             VALUES (?1, ?2, 'editor', 'an editor', X'2F65', 0)",
            rusqlite::params![alpha.0, loc.0],
        )
        .unwrap();
    let target = TargetId(index.index().conn().last_insert_rowid());
    let clock = Arc::new(FakeClock::new(T0));
    let activity = SharedActivity::default();
    let manager = SessionManager::new(
        clock.clone(),
        Arc::new(NoEvents),
        Box::new(activity.clone()),
        Arc::new(FakeIgnoreCheck::new(&["dist/"])),
        Arc::new(FixedZone(0)),
    );
    Rig {
        index,
        manager,
        clock,
        activity,
        alpha,
        beta,
        loc,
        other_loc,
        target,
    }
}

impl Rig {
    fn launch(&mut self, project: ProjectId, location: LocationId) -> SessionId {
        let launched = LaunchedSession {
            project_id: project,
            location_id: Some(location),
            target_id: Some(self.target),
            repo: RepoHandle::bare(
                std::path::Path::new("/w/repo"),
                StoreKey::new("s"),
                StoreClass::Local,
            ),
            waiter: None,
        };
        self.manager
            .launch(self.index.index_mut(), launched)
            .unwrap()
    }

    /// Let `count` ticks of wall time pass, pumping the manager on each.
    fn ticks(&mut self, count: usize) {
        let tick = i64::try_from(DEFAULT_TICK_SECS).unwrap();
        for _ in 0..count {
            self.clock.advance(tick);
            self.manager.tick(self.index.index_mut()).unwrap();
        }
    }

    /// Every segment as `(started_at, credited_seconds, signalled)`, in the order opened.
    fn segments(&self) -> Vec<(i64, i64, Option<i64>)> {
        segments(self.index.index())
    }

    fn focus(&self) -> i64 {
        focus_seconds(self.index.index().conn(), &UTC, DAY, LATER).unwrap()
    }
}

fn segments(index: &Index) -> Vec<(i64, i64, Option<i64>)> {
    let mut stmt = index
        .conn()
        .prepare("SELECT started_at, credited_seconds, signalled FROM session_segment ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows
}

/// One session of `project` holding one stored segment per `(started_at, credited, signalled)`,
/// each closed unless `credited` is `None`. Returns the segments' ids.
fn stored(
    index: &mut TempIndex,
    project: ProjectId,
    segments: &[(i64, Option<i64>, Option<bool>)],
) -> Vec<i64> {
    let location = index.insert_location(project, &format!("/w/{}-{}", project.0, segments[0].0));
    let tx = index.index_mut().conn_mut().transaction().unwrap();
    let session = store::open_session(
        &tx,
        &SessionOpen {
            project_id: project,
            location_id: Some(location),
            target_id: None,
            started_at: segments[0].0,
        },
    )
    .unwrap();
    let mut ids = Vec::new();
    for &(started_at, credited, signalled) in segments {
        let id = store::open_segment(&tx, session, started_at, signalled.unwrap_or(false)).unwrap();
        if signalled.is_none() {
            tx.execute(
                "UPDATE session_segment SET signalled = NULL WHERE id = ?1",
                [id],
            )
            .unwrap();
        }
        if let Some(credited) = credited {
            close(&tx, id, started_at, credited);
        }
        ids.push(id);
    }
    tx.commit().unwrap();
    ids
}

fn close(tx: &rusqlite::Transaction<'_>, id: i64, started_at: i64, credited: i64) {
    store::close_segment(
        tx,
        id,
        &ClosedSegment {
            started_at,
            ended_at: started_at + credited,
            credited_seconds: credited,
            closed_by: ClosedBy::Idle,
        },
    )
    .unwrap();
}

fn focus_on(index: &TempIndex, zone: &dyn LocalZone, day: i64) -> i64 {
    focus_seconds(index.index().conn(), zone, day, LATER).unwrap()
}

/// One session whose segments are legacy rows (`signalled` NULL), each `credited` long and an
/// hour apart: the day's Focus.
fn legacy(credits: &[i64]) -> i64 {
    let mut index = TempIndex::new();
    let project = index.insert_project();
    let rows: Vec<_> = credits
        .iter()
        .zip(0_i64..)
        .map(|(&credited, n)| (T0 + n * 3_600, Some(credited), None))
        .collect();
    stored(&mut index, project, &rows);
    focus_on(&index, &UTC, DAY)
}

#[test]
fn ac_p4_39_2_launch_only_never_counts() {
    let mut launches = rig();
    launches.launch(launches.alpha, launches.loc);
    launches.ticks(80);
    launches.launch(launches.beta, launches.other_loc);
    launches.ticks(200);
    let rows = launches.segments();
    eprintln!(
        "two launches 20 minutes apart, no signal: {rows:?}, focus {}",
        launches.focus()
    );
    assert_eq!(
        rows,
        [
            (T0, SEGMENT_IDLE_SECS, Some(0)),
            (T0 + SEGMENT_IDLE_SECS, SEGMENT_IDLE_SECS, Some(0))
        ]
    );
    assert_eq!(launches.focus(), 0, "launch-only segments are not in C");

    let mut touched = rig();
    let s = touched.launch(touched.alpha, touched.loc);
    touched.ticks(39);
    touched.activity.0.lock().unwrap().push(s, "src/main.rs");
    touched.ticks(1);
    touched.ticks(200);
    assert_eq!(touched.segments(), [(T0, 1_800, Some(1))]);
    assert_eq!(
        touched.focus(),
        1_800,
        "one change at +10 min credits its tail"
    );

    // §39.3's legacy table, for rows written before the column.
    assert_eq!(
        legacy(&[SEGMENT_IDLE_SECS]),
        0,
        "a first segment at the bound"
    );
    assert_eq!(legacy(&[SEGMENT_IDLE_SECS + 1]), SEGMENT_IDLE_SECS + 1);
    assert_eq!(
        legacy(&[SEGMENT_IDLE_SECS, SEGMENT_IDLE_SECS]),
        SEGMENT_IDLE_SECS,
        "a later segment at the bound was opened by a signal"
    );
    assert!(!is_counted(None, true, SEGMENT_IDLE_SECS));
    assert!(is_counted(None, true, SEGMENT_IDLE_SECS + 1));
    assert!(is_counted(None, false, SEGMENT_IDLE_SECS));
    assert!(is_counted(Some(true), true, 0));
    assert!(!is_counted(Some(false), false, 7_200));
}

#[test]
fn ac_p4_39_3_union_and_midnight() {
    let mut overlap = TempIndex::new();
    let alpha = overlap.insert_project();
    let beta = overlap.insert_project();
    let a = stored(&mut overlap, alpha, &[(T0, Some(1_800), Some(true))])[0];
    // Beta stays open, its watermark at +2 400.
    let b = stored(&mut overlap, beta, &[(T0 + 600, None, Some(true))])[0];
    let watermark = overlap.index_mut().conn_mut().transaction().unwrap();
    store::mark_activity(&watermark, b, T0 + 2_400).unwrap();
    watermark.commit().unwrap();
    assert_eq!(
        focus_on(&overlap, &UTC, DAY),
        2_400,
        "the union, never the sum"
    );
    let conn = overlap.index().conn();
    assert_eq!(
        marginal_seconds(conn, &UTC, a).unwrap(),
        [DatedContribution {
            day: DAY,
            seconds: 1_800,
            first_instant: T0
        }],
        "an open segment is no other segment's cover"
    );
    let closing = overlap.index_mut().conn_mut().transaction().unwrap();
    close(&closing, b, T0 + 600, 1_800);
    closing.commit().unwrap();
    assert_eq!(
        marginal_seconds(overlap.index().conn(), &UTC, b).unwrap(),
        [DatedContribution {
            day: DAY,
            seconds: 600,
            first_instant: T0 + 1_800
        }],
        "what the earlier close already counted"
    );

    // Across local midnight: both dates, split where the zone says the day ends.
    let mut midnight = TempIndex::new();
    let late = midnight.insert_project();
    let m = stored(
        &mut midnight,
        late,
        &[(MIDNIGHT - 600, Some(1_500), Some(true))],
    )[0];
    assert_eq!(focus_on(&midnight, &UTC, DAY - 1), 600);
    assert_eq!(focus_on(&midnight, &UTC, DAY), 900);
    assert_eq!(
        marginal_seconds(midnight.index().conn(), &UTC, m).unwrap(),
        [
            DatedContribution {
                day: DAY - 1,
                seconds: 600,
                first_instant: MIDNIGHT - 600
            },
            DatedContribution {
                day: DAY,
                seconds: 900,
                first_instant: MIDNIGHT
            }
        ]
    );

    // A 23-hour day: an hour skipped at 02:00 local.
    let zone = TransitionZone {
        at: MIDNIGHT + 3_600,
        before_min: 60,
        after_min: 120,
    };
    let (start, end) = day_bounds(&zone, DAY);
    assert_eq!(end - start, 82_800, "the fixture is a 23-hour day");
    let mut short_day = TempIndex::new();
    let early = short_day.insert_project();
    stored(
        &mut short_day,
        early,
        &[(start - 100, Some(end - start + 200), Some(true))],
    );
    let days = [DAY - 1, DAY, DAY + 1].map(|d| focus_on(&short_day, &zone, d));
    eprintln!("a segment over a 23-hour day: {days:?}");
    assert_eq!(days, [100, 82_800, 100]);
}

#[test]
fn ac_p4_39_4_core_halves() {
    // A claim reported a second before the launch is the launch's own click: it never extends
    // the segment the launch opened.
    let mut early = rig();
    early.manager.set_focus(Some(early.alpha));
    early.clock.advance(1);
    early.launch(early.alpha, early.loc);
    early.ticks(200);
    let early_rows = early.segments();
    eprintln!("a claim then a launch: {early_rows:?}");
    assert_eq!(early_rows, [(T0 + 1, SEGMENT_IDLE_SECS, Some(0))]);

    // A claim after the launch extends it, by no more than the bound from the last input.
    let mut late = rig();
    late.launch(late.alpha, late.loc);
    late.clock.advance(60);
    late.manager.set_focus(Some(late.alpha));
    let claim = T0 + 60;
    late.ticks(200);
    let late_rows = late.segments();
    eprintln!("a launch then a claim at +60: {late_rows:?}");
    let (started_at, credited, signalled) = late_rows[0];
    assert_eq!(late_rows.len(), 1);
    assert_eq!(signalled, Some(1));
    assert!(
        credited > SEGMENT_IDLE_SECS,
        "the claim extended it: {credited}"
    );
    assert!(
        started_at + credited <= claim + ONE_TOUCH_CREDIT_BOUND_SECS - FOCUS_HEARTBEAT_SECS,
        "credited end {} past the claim's bound",
        started_at + credited
    );
}

#[test]
fn the_launch_segment_and_a_reopened_one_record_how_they_opened() {
    let mut r = rig();
    let s = r.launch(r.alpha, r.loc);
    r.ticks(100);
    r.activity.0.lock().unwrap().push(s, "src/main.rs");
    r.ticks(1);
    r.manager.stop(r.index.index_mut(), s).unwrap();
    let flags: Vec<_> = r.segments().into_iter().map(|(_, _, f)| f).collect();
    assert_eq!(flags, [Some(0), Some(1)]);
    assert_eq!(
        store::session_ref(r.index.index().conn(), s)
            .unwrap()
            .close_reason,
        Some(CloseReason::Stop)
    );
}
