#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §39.3 and §39.4 — the Focus and Ship rings, read from the rows the session manager and J1
//! write. Focus closes only on the counted union; Ship reads an unobservable record as not
//! computed, never as an open ring.

mod support;

use std::sync::{Arc, Mutex};

use codotheca_core::clock::{local_date, parse_local_date, week_start};
use codotheca_core::git::{RepoHandle, StoreKey};
use codotheca_core::mount::StoreClass;
use codotheca_core::protocol::{
    FocusRing, LocationId, ProjectId, RingStatus, SessionId, ShipRing, TargetId,
};
use codotheca_core::rings::focus::focus_ring;
use codotheca_core::rings::ship::ship_ring;
use codotheca_core::rings::targets::KEY_FOCUS_TARGET_MINUTES;
use codotheca_core::session::activity::FakeIgnoreCheck;
use codotheca_core::session::manager::{LaunchedSession, SessionManager};
use codotheca_core::session::watch::{ActivityBatch, ActivitySource, FakeActivitySource};
use codotheca_core::session::{SessionError, DEFAULT_TICK_SECS};
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{FakeClock, TempIndex};
use support::rings_rig::{Flags, RingsRig, UTC};

/// A UTC midnight: local day [`DAY`] under UTC starts here.
const MIDNIGHT: i64 = 1_700_006_400;
/// The local day number of [`MIDNIGHT`].
const DAY: i64 = MIDNIGHT / 86_400;
/// One in the morning of [`DAY`]: every manager-driven session here stays inside that day.
const T0: i64 = MIDNIGHT + 3_600;
/// A reading long after every segment here.
const LATER: i64 = MIDNIGHT + 10 * 86_400;

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

/// Two projects and a launch target, behind a `SessionManager` on a fake clock.
struct Rig {
    index: TempIndex,
    manager: SessionManager,
    clock: Arc<FakeClock>,
    activity: SharedActivity,
    alpha: (ProjectId, LocationId),
    beta: (ProjectId, LocationId),
    target: TargetId,
}

impl Rig {
    fn new() -> Self {
        let index = TempIndex::new();
        let alpha = index.insert_project();
        let beta = index.insert_project();
        let alpha_loc = index.insert_location(alpha, "/w/alpha");
        let beta_loc = index.insert_location(beta, "/w/beta");
        index
            .index()
            .conn()
            .execute(
                "INSERT INTO launch_target (project_id, location_id, kind, name, exec_bytes,
                                            sort_index)
                 VALUES (?1, ?2, 'editor', 'an editor', X'2F65', 0)",
                rusqlite::params![alpha.0, alpha_loc.0],
            )
            .unwrap();
        let target = TargetId(index.index().conn().last_insert_rowid());
        let clock = Arc::new(FakeClock::new(T0));
        let activity = SharedActivity::default();
        let manager = SessionManager::new(
            clock.clone(),
            Arc::new(ValidatingSink::default()),
            Box::new(activity.clone()),
            Arc::new(FakeIgnoreCheck::new(&["dist/"])),
            Arc::new(FixedZone(0)),
        );
        Self {
            index,
            manager,
            clock,
            activity,
            alpha: (alpha, alpha_loc),
            beta: (beta, beta_loc),
            target,
        }
    }

    fn launch(&mut self, (project, location): (ProjectId, LocationId)) -> SessionId {
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

    /// The Focus ring for [`DAY`] at a target of `minutes`.
    fn ring_at(&self, minutes: u32) -> FocusRing {
        let conn = self.index.index().conn();
        conn.execute(
            "INSERT INTO app_meta (k, v) VALUES (?1, ?2)
             ON CONFLICT (k) DO UPDATE SET v = excluded.v",
            [KEY_FOCUS_TARGET_MINUTES, &minutes.to_string()],
        )
        .unwrap();
        focus_ring(conn, DAY, &UTC, LATER).unwrap()
    }
}

/// **AC-P4-39-2-ring.** Two bare launches 20 minutes apart leave the ring open at a 30-minute
/// target — a measure that counted launch-only segments would read a 40-minute union and close
/// it. One launch plus one worktree change at +10 min credits 30 minutes: closed at 30, open at
/// 31. The floor's compile-time assert is read from its source line, so deleting it is a red test.
#[test]
fn ac_p4_39_2_one_touch_never_closes_focus() {
    let mut launches = Rig::new();
    launches.launch(launches.alpha);
    launches.ticks(80);
    launches.launch(launches.beta);
    launches.ticks(200);
    let bare = launches.ring_at(30);
    eprintln!("two bare launches, target 30 min: {bare:?}");
    assert_eq!(
        bare.status,
        RingStatus::Open,
        "a launch-only union closed Focus"
    );
    assert_eq!(bare.credited_seconds, 0);

    let mut touched = Rig::new();
    let session = touched.launch(touched.alpha);
    touched.ticks(39);
    touched
        .activity
        .0
        .lock()
        .unwrap()
        .push(session, "src/main.rs");
    touched.ticks(201);
    let at_30 = touched.ring_at(30);
    let at_31 = touched.ring_at(31);
    eprintln!("one change at +10 min: target 30 {at_30:?}, target 31 {at_31:?}");
    assert_eq!(
        at_30,
        FocusRing {
            credited_seconds: 1_800,
            target_seconds: 1_800,
            status: RingStatus::Closed,
        }
    );
    assert_eq!(at_31.status, RingStatus::Open);
    assert_eq!(at_31.target_seconds, 1_860);

    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/session/mod.rs"),
    )
    .unwrap();
    assert!(!source.is_empty(), "read nothing from session/mod.rs");
    let floor = source.lines().find(|line| {
        line.contains("assert!(")
            && line.contains("FOCUS_TARGET_MIN_MINUTES")
            && line.contains("ONE_TOUCH_CREDIT_BOUND_SECS")
    });
    eprintln!("the floor assert: {floor:?}");
    assert!(
        floor.is_some(),
        "the Focus floor's compile-time assert is gone"
    );
}

/// One planted copy: its project's flags, its `push_record` and its `(last_push_at, offset)`.
type Copy = (Flags, Option<&'static str>, Option<(i64, i32)>);

/// Write J1's three push columns onto `location`.
fn record(rig: &RingsRig, location: LocationId, slug: Option<&str>, push: Option<(i64, i32)>) {
    rig.conn()
        .execute(
            "UPDATE location SET push_record = ?2, last_push_at = ?3, last_push_tz_offset_min = ?4
             WHERE id = ?1",
            rusqlite::params![location.0, slug, push.map(|p| p.0), push.map(|p| p.1)],
        )
        .unwrap();
}

/// A fresh live project with one present location carrying `slug` and `push`.
fn copy(rig: &RingsRig, flags: Flags, slug: Option<&str>, push: Option<(i64, i32)>) -> LocationId {
    // `removed_at` removes the copy alone; the project stays live.
    let project = rig.project(Flags {
        removed_at: None,
        ..flags
    });
    let location = rig.location(project, "present", flags.removed_at);
    record(rig, location, slug, push);
    location
}

/// **AC-P4-39-5-ring.** An `R` wholly unobservable reads `notComputed`, never `open`; an empty
/// `R` reads `notApplicable`; a mix of read and unreadable with no push this week reads `open`
/// with both counts. A push is dated in its own offset, so Monday 00:30 at `+0530` — Sunday 19:00
/// UTC — closes this week. A shallow copy's push counts; a Reference project's and a removed
/// copy's do not.
#[test]
fn ac_p4_39_5_the_ship_ring_never_reads_unobservable_as_open() {
    let monday = week_start(parse_local_date("2026-11-04").unwrap());
    assert_eq!(local_date(monday), "2026-11-02");
    let cases = ship_cases(monday);
    for (name, copies, expected) in &cases {
        let rig = RingsRig::new();
        for &(flags, slug, push) in copies {
            copy(&rig, flags, slug, push);
        }
        let read = ship_ring(rig.conn(), monday + 2).unwrap();
        eprintln!("{name}: {read:?}");
        assert_eq!(&read, expected, "{name}");
        if expected.status == RingStatus::NotComputed {
            assert_ne!(
                read.status,
                RingStatus::Open,
                "an unobservable record read as open"
            );
        }
    }
    eprintln!("ship cases: {}", cases.len());
}

/// The Ship ring's cases for the week of `monday`, each `(name, copies, expected ring)`.
fn ship_cases(monday: i64) -> [(&'static str, Vec<Copy>, ShipRing); 8] {
    // Monday 00:30 at +0530 is Sunday 19:00 UTC: the line's own offset dates it this week.
    let pushed = Some((monday * 86_400 + 30 * 60 - 330 * 60, 330));
    let last_week = Some(((monday - 3) * 86_400, 0));
    let live = Flags::default();
    let shallow = Flags {
        shallow: true,
        ..live
    };
    let reference = Flags {
        reference: true,
        ..live
    };
    let removed = Flags {
        removed_at: Some(1),
        ..live
    };
    let want = |status, pushed_on: Option<&str>, observable, unobservable| ShipRing {
        status,
        pushed_on: pushed_on.map(str::to_owned),
        observable,
        unobservable,
    };
    let closed_monday = |observable, unobservable| {
        want(
            RingStatus::Closed,
            Some("2026-11-02"),
            observable,
            unobservable,
        )
    };
    [
        (
            "R wholly not yet observed",
            vec![(live, None, None), (live, None, None)],
            want(RingStatus::NotComputed, None, 0, 2),
        ),
        (
            "R wholly unobservable",
            vec![
                (live, Some("unreadable"), None),
                (live, Some("absent"), None),
                (live, Some("unsupported"), None),
                (live, None, None),
            ],
            want(RingStatus::NotComputed, None, 0, 4),
        ),
        (
            "only no_remote",
            vec![
                (live, Some("no_remote"), None),
                (live, Some("no_remote"), None),
            ],
            want(RingStatus::NotApplicable, None, 0, 0),
        ),
        (
            "no location",
            Vec::new(),
            want(RingStatus::NotApplicable, None, 0, 0),
        ),
        (
            "read (last week's push) + unreadable",
            vec![
                (live, Some("read"), last_week),
                (live, Some("unreadable"), None),
            ],
            want(RingStatus::Open, None, 1, 1),
        ),
        (
            "Monday 00:30 at +0530",
            vec![
                (live, Some("read"), pushed),
                (live, Some("unreadable"), None),
            ],
            closed_monday(1, 1),
        ),
        (
            "a shallow copy's push",
            vec![(shallow, Some("read"), pushed)],
            closed_monday(1, 0),
        ),
        (
            "a Reference project's and a removed copy's pushes",
            vec![
                (live, Some("read"), None),
                (reference, Some("read"), pushed),
                (removed, Some("read"), pushed),
            ],
            want(RingStatus::Open, None, 1, 0),
        ),
    ]
}

/// §39.11: the credit crosses the wire floored to a whole minute; closure is unaffected.
#[test]
fn credited_seconds_are_floored_to_the_minute() {
    for (credited, floored, status) in [
        (89, 60, RingStatus::Open),
        (1_859, 1_800, RingStatus::Closed),
    ] {
        let rig = RingsRig::new();
        let project = rig.project(Flags::default());
        rig.segment(project, T0, credited, Some(true));
        rig.meta(KEY_FOCUS_TARGET_MINUTES, "30");
        let focus = focus_ring(rig.conn(), DAY, &UTC, LATER).unwrap();
        eprintln!("{credited} s credited: {focus:?}");
        assert_eq!(focus.credited_seconds, floored);
        assert_eq!(focus.status, status);
    }
}
