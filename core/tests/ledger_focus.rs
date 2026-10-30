//! §38.8.3's `focus` row through the closes that write it: the session manager's and orphan
//! recovery's. Each close stores its share of the day's union (§39.3), so a date's rows add up to
//! that date's Focus whatever order the segments closed in.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use codotheca_core::clock::{LocalInstant, LocalZone};
use codotheca_core::git::{RepoHandle, StoreKey};
use codotheca_core::index::Index;
use codotheca_core::ledger::focus::credit_segment_close;
use codotheca_core::ledger::price::PRICES;
use codotheca_core::ledger::xp::xp_by_term;
use codotheca_core::mount::StoreClass;
use codotheca_core::protocol::{ClosedBy, LocationId, ProjectId, SessionId, TargetId, XpTerm};
use codotheca_core::session::activity::FakeIgnoreCheck;
use codotheca_core::session::manager::{LaunchedSession, SessionManager};
use codotheca_core::session::measure::focus_seconds;
use codotheca_core::session::orphan::close_orphans;
use codotheca_core::session::segment::ClosedSegment;
use codotheca_core::session::store::{self, SessionOpen};
use codotheca_core::session::watch::{ActivityBatch, ActivitySource, FakeActivitySource};
use codotheca_core::session::{SessionError, DEFAULT_TICK_SECS};
use codotheca_core::testing::events::ValidatingSink;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{FakeClock, TempIndex};

/// A UTC midnight: local day [`DAY`] under UTC starts here.
const MIDNIGHT: i64 = 1_700_006_400;
/// One in the morning of the UTC day [`MIDNIGHT`] starts.
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

/// One stored `focus` row.
#[derive(Debug, PartialEq, Eq)]
struct FocusRow {
    project: i64,
    ts: i64,
    offset_min: Option<i64>,
    seconds: i64,
    when: String,
}

fn focus_rows(index: &Index) -> Vec<FocusRow> {
    let mut stmt = index
        .conn()
        .prepare(
            "SELECT project_id, ts, tz_offset_min, json_extract(meta, '$.seconds'),
                    json_extract(meta, '$.when')
               FROM xp_events WHERE kind = 'focus' ORDER BY id",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok(FocusRow {
                project: r.get(0)?,
                ts: r.get(1)?,
                offset_min: r.get(2)?,
                seconds: r.get(3)?,
                when: r.get(4)?,
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows
}

/// Every column of every `focus` row but `meta`, as text.
fn columns_but_meta(index: &Index) -> Vec<String> {
    let mut stmt = index
        .conn()
        .prepare(
            "SELECT json_array(id, ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                               track)
               FROM xp_events WHERE kind = 'focus' ORDER BY id",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows
}

/// The library's focus XP, as the ledger prices it.
fn focus_xp(index: &Index) -> i64 {
    let totals = xp_by_term(index.conn()).unwrap();
    totals
        .terms
        .iter()
        .find(|(term, _)| *term == XpTerm::Focus)
        .unwrap()
        .1
}

/// The local date of an instant framed in `zone`, as a receipt writes it.
fn date(zone: &dyn LocalZone, unix: i64) -> String {
    zone.at(unix).date()
}

struct Rig {
    index: TempIndex,
    manager: SessionManager,
    clock: Arc<FakeClock>,
    activity: SharedActivity,
    target: Option<TargetId>,
}

/// A manager whose clock starts at `at`, framing every date in UTC.
fn rig(at: i64) -> Rig {
    let clock = Arc::new(FakeClock::new(at));
    let activity = SharedActivity::default();
    let manager = SessionManager::new(
        clock.clone(),
        Arc::new(ValidatingSink::default()),
        Box::new(activity.clone()),
        Arc::new(FakeIgnoreCheck::new(&["dist/"])),
        Arc::new(FixedZone(0)),
    );
    Rig {
        index: TempIndex::new(),
        manager,
        clock,
        activity,
        target: None,
    }
}

impl Rig {
    fn project(&mut self, path: &str) -> (ProjectId, LocationId) {
        let project = self.index.insert_project();
        let location = self.index.insert_location(project, path);
        if self.target.is_none() {
            self.index
                .index()
                .conn()
                .execute(
                    "INSERT INTO launch_target (project_id, location_id, kind, name, exec_bytes,
                                                sort_index)
                     VALUES (?1, ?2, 'editor', 'an editor', X'2F65', 0)",
                    rusqlite::params![project.0, location.0],
                )
                .unwrap();
            self.target = Some(TargetId(self.index.index().conn().last_insert_rowid()));
        }
        (project, location)
    }

    fn launch(&mut self, project: ProjectId, location: LocationId) -> SessionId {
        let launched = LaunchedSession {
            project_id: project,
            location_id: Some(location),
            target_id: self.target,
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

    /// One tick `secs` later, with a worktree change in each of `touched`.
    fn step(&mut self, secs: i64, touched: &[SessionId]) {
        self.clock.advance(secs);
        for &session in touched {
            self.activity.0.lock().unwrap().push(session, "src/main.rs");
        }
        self.manager.tick(self.index.index_mut()).unwrap();
    }

    /// Long enough at the default tick for every open segment to idle out.
    fn idle(&mut self) {
        let tick = i64::try_from(DEFAULT_TICK_SECS).unwrap();
        for _ in 0..100 {
            self.step(tick, &[]);
        }
    }

    fn rows(&self) -> Vec<FocusRow> {
        focus_rows(self.index.index())
    }
}

#[test]
fn ac_p4_38_19_focus_through_the_session_manager() {
    one_date_at_its_cap();
    two_projects_priced_once();
    nothing_for_untouched_or_reference();
    one_row_per_date_across_midnight();
    a_second_close_moves_only_meta();
}

/// 200 minutes of union on one date: 66 points by the rate, 40 by the date's cap.
fn one_date_at_its_cap() {
    let mut long = rig(T0);
    let (a, at_a) = long.project("/w/a");
    let s = long.launch(a, at_a);
    for _ in 0..18 {
        long.step(600, &[s]);
    }
    long.idle();
    let rows = long.rows();
    eprintln!("200 minutes: {rows:?}");
    assert_eq!(
        rows,
        [FocusRow {
            project: a.0,
            ts: T0,
            offset_min: Some(0),
            seconds: 12_000,
            when: date(&FixedZone(0), T0),
        }]
    );
    assert_eq!(focus_xp(long.index.index()), PRICES.focus_per_date);
}

/// Two projects worked on at once: one person's time, priced once.
fn two_projects_priced_once() {
    let mut both = rig(T0);
    let (a, at_a) = both.project("/w/a");
    let (b, at_b) = both.project("/w/b");
    let sa = both.launch(a, at_a);
    both.step(300, &[]);
    let sb = both.launch(b, at_b);
    both.step(300, &[sa]);
    both.step(300, &[sb]);
    both.idle();
    let shares: Vec<(i64, i64)> = both.rows().iter().map(|r| (r.project, r.seconds)).collect();
    eprintln!("two overlapping sessions: {shares:?}");
    assert_eq!(
        shares,
        [(a.0, 1_800), (b.0, 300)],
        "the union, never the sum"
    );
    assert_eq!(
        focus_xp(both.index.index()),
        2_100 / PRICES.focus_seconds_per_xp
    );
}

/// A launch nobody touched, and a Reference project's work, write nothing.
fn nothing_for_untouched_or_reference() {
    let mut untouched = rig(T0);
    let (a, at_a) = untouched.project("/w/a");
    untouched.launch(a, at_a);
    untouched.idle();
    assert_eq!(untouched.rows(), []);
    let mut reference = rig(T0);
    let (r, at_r) = reference.project("/w/r");
    reference
        .index
        .index()
        .conn()
        .execute("UPDATE project SET is_reference = 1 WHERE id = ?1", [r.0])
        .unwrap();
    let sr = reference.launch(r, at_r);
    reference.step(600, &[sr]);
    reference.idle();
    assert_eq!(reference.rows(), []);
    for (what, rig) in [("untouched", &untouched), ("reference", &reference)] {
        let closed: i64 = rig
            .index
            .index()
            .conn()
            .query_row(
                "SELECT count(*) FROM session_segment WHERE closed_by IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(closed, 1, "{what}: the segment did close");
    }
}

/// Across midnight: one row per date, each at its first credited instant.
fn one_row_per_date_across_midnight() {
    let mut night = rig(MIDNIGHT - 600);
    let (a, at_a) = night.project("/w/a");
    let s = night.launch(a, at_a);
    night.step(900, &[s]);
    night.idle();
    let utc = FixedZone(0);
    assert_eq!(
        night.rows(),
        [
            FocusRow {
                project: a.0,
                ts: MIDNIGHT - 600,
                offset_min: Some(0),
                seconds: 600,
                when: date(&utc, MIDNIGHT - 600),
            },
            FocusRow {
                project: a.0,
                ts: MIDNIGHT,
                offset_min: Some(0),
                seconds: 1_500,
                when: date(&utc, MIDNIGHT),
            }
        ]
    );
}

/// A second close the same date moves `meta` and nothing else.
fn a_second_close_moves_only_meta() {
    let mut twice = rig(T0);
    let (a, at_a) = twice.project("/w/a");
    let s = twice.launch(a, at_a);
    twice.step(600, &[s]);
    twice.idle();
    let first = columns_but_meta(twice.index.index());
    twice.step(600, &[s]);
    twice.idle();
    assert_eq!(columns_but_meta(twice.index.index()), first);
    assert_eq!(first.len(), 1);
    assert_eq!(twice.rows()[0].seconds, 1_800 + 1_200);
}

/// One closed or open segment of the fixture below: `(project, started_at, credited, signalled)`.
type Planned = (usize, i64, i64, bool);

#[test]
fn ac_p4_39_3_stored_marginals_sum_to_the_union() {
    // An hour east of UTC, so a local date is never a UTC one.
    let zone = FixedZone(60);
    let midnight = MIDNIGHT - 3_600;
    let base = midnight - 20_000;
    let mut index = TempIndex::new();
    let projects: Vec<ProjectId> = (0..3).map(|_| index.insert_project()).collect();
    let locations: Vec<LocationId> = projects
        .iter()
        .enumerate()
        .map(|(n, p)| index.insert_location(*p, &format!("/w/p{n}")))
        .collect();
    index
        .index()
        .conn()
        .execute(
            "UPDATE project SET is_reference = 1 WHERE id = ?1",
            [projects[2].0],
        )
        .unwrap();

    let closed: [Planned; 8] = [
        (0, base, 3_000, true),
        (1, base + 1_000, 1_000, true),
        (1, base + 2_500, 2_500, true),
        (0, midnight - 1_000, 3_000, true),
        (1, midnight - 500, 1_000, true),
        (2, base + 100, 8_900, true),
        (0, base + 4_000, 1_200, false),
        (1, midnight + 1_500, 2_500, true),
    ];
    let orphaned: [Planned; 2] = [
        (0, midnight + 3_000, 2_000, true),
        (1, midnight + 4_500, 1_500, true),
    ];
    let mut ids: Vec<i64> = Vec::new();
    {
        let tx = index.index_mut().conn_mut().transaction().unwrap();
        for &(p, started_at, _, signalled) in closed.iter().chain(&orphaned) {
            let session = store::open_session(
                &tx,
                &SessionOpen {
                    project_id: projects[p],
                    location_id: Some(locations[p]),
                    target_id: None,
                    started_at,
                },
            )
            .unwrap();
            ids.push(store::open_segment(&tx, session, started_at, signalled).unwrap());
        }
        // An orphan's watermark: its last activity, as the store wrote it before the crash.
        for (n, &(_, started_at, credited, _)) in orphaned.iter().enumerate() {
            store::mark_activity(&tx, ids[closed.len() + n], started_at + credited).unwrap();
        }
        tx.commit().unwrap();
    }

    // Closed out of start order — the inner one before its container, the midnight pair split —
    // then the rest by orphan recovery.
    for n in [4, 1, 3, 0, 7, 2, 5, 6] {
        let (_, started_at, credited, _) = closed[n];
        let tx = index.index_mut().conn_mut().transaction().unwrap();
        store::close_segment(
            &tx,
            ids[n],
            &ClosedSegment {
                started_at,
                ended_at: started_at + credited,
                credited_seconds: credited,
                closed_by: ClosedBy::Idle,
            },
        )
        .unwrap();
        credit_segment_close(&tx, &zone, ids[n]).unwrap();
        tx.commit().unwrap();
    }
    let report = close_orphans(index.index_mut(), LATER, &zone).unwrap();
    assert_eq!(report.segments_closed, 2);

    let mut stored: BTreeMap<i64, i64> = BTreeMap::new();
    for row in focus_rows(index.index()) {
        let at = LocalInstant {
            unix: row.ts,
            offset_min: i32::try_from(row.offset_min.unwrap()).unwrap(),
        };
        assert_eq!(row.when, at.date(), "{row:?}");
        *stored.entry(at.day()).or_default() += row.seconds;
    }
    let mut dates = 0;
    for day in zone.at(base).day()..=zone.at(LATER).day() {
        let union = focus_seconds(index.index().conn(), &zone, day, LATER).unwrap();
        let sum = stored.get(&day).copied().unwrap_or(0);
        if union > 0 || sum > 0 {
            eprintln!("day {day}: stored {sum}, union {union}");
            dates += 1;
        }
        assert_eq!(sum, union, "day {day}");
    }
    assert!(dates >= 2, "the fixture touched {dates} dates");
}
