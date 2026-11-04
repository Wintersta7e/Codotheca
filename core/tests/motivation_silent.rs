//! §43.5's silent class, each class driven through its production caller: a rise the user did
//! not just cause moves the number and makes no line, and a forward rise still does.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

#[path = "support/handoff_rig.rs"]
mod handoff_rig;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use codotheca_core::assembly::startup::run_startup;
use codotheca_core::clock::LocalInstant;
use codotheca_core::completion::evaluate_and_write;
use codotheca_core::index::pending::stage_pending;
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome};
use codotheca_core::index::sidecar::export;
use codotheca_core::index::subject::subject_for_project;
use codotheca_core::index::Index;
use codotheca_core::ledger::level::read_level;
use codotheca_core::ledger::price::testkit::override_prices;
use codotheca_core::ledger::price::PRICES;
use codotheca_core::motivation::keys::{level_line_since, level_shown, write_level_shown};
use codotheca_core::motivation::state::motivation_state;
use codotheca_core::protocol::{MotivationLine, MotivationLineForm, ProjectId, XpTerm};
use codotheca_core::scan::ScanLauncher;
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{core_handler, HeldScanLauncher};
use support::badges_rig::{BadgesRig, Flags, T0};

const DAY: i64 = 86_400;
/// 2024-05-01, the first fixture commit-day.
const D1: i64 = 19_844;

/// The price override is process-global: every test here takes this first.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The level, the level shown, the reason's mark and the pending line, read as the band reads
/// them.
#[derive(Debug)]
struct Seen {
    displayed: Option<u32>,
    shown: Option<u32>,
    since: Option<i64>,
    max_id: i64,
    line: Option<MotivationLine>,
}

fn seen(index: &Mutex<Index>, now: i64) -> Seen {
    let mut guard = index.lock().unwrap();
    let seen = guard
        .with_tx(|tx| {
            let displayed = read_level(tx, now)?.figures.map(|f| f.level);
            let line = motivation_state(tx, now, &FixedZone(0))?.pending_line;
            let max_id: i64 =
                tx.query_row("SELECT COALESCE(MAX(id), 0) FROM xp_events", [], |r| {
                    r.get(0)
                })?;
            Ok(Seen {
                displayed,
                shown: level_shown(tx)?,
                since: level_line_since(tx)?,
                max_id,
                line,
            })
        })
        .unwrap();
    drop(guard);
    eprintln!("{seen:?}");
    seen
}

/// Present the displayed level, as a click on its line would: no line pending afterwards.
fn present(index: &Mutex<Index>, now: i64) -> u32 {
    let mut guard = index.lock().unwrap();
    let level = guard
        .with_tx(|tx| {
            let level = read_level(tx, now)?.figures.expect("a level").level;
            write_level_shown(tx, level)?;
            Ok(level)
        })
        .unwrap();
    drop(guard);
    level
}

/// `n` rows of a retained `kind` with no project, each its own key.
fn library_rows(index: &Mutex<Index>, kind: &str, n: i64) {
    let guard = index.lock().unwrap();
    let base: i64 = guard
        .conn()
        .query_row("SELECT COALESCE(MAX(id), 0) FROM xp_events", [], |r| {
            r.get(0)
        })
        .unwrap();
    for i in 0..n {
        guard
            .conn()
            .execute(
                "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind,
                                        dedupe_key, track, meta)
                 VALUES (?1, 0, NULL, 'library', ?2, ?3, 'session', NULL)",
                rusqlite::params![D1 * DAY + i, kind, format!("{kind}:library:{}", base + i)],
            )
            .unwrap();
    }
    drop(guard);
}

/// One forward `debt_day` on `project`, its receipt naming it as of `day`.
fn debt_day(index: &Mutex<Index>, project: ProjectId, day: i64) {
    let guard = index.lock().unwrap();
    guard
        .conn()
        .execute(
            "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                    track, meta)
             VALUES (?1, 0, ?2, 'subject', 'debt_day', ?3, 'session', ?4)",
            rusqlite::params![
                day * DAY,
                project.0,
                format!("debt_day:{}:{day}", project.0),
                serde_json::json!({ "project_name": "forward", "when": "2024-06-01" }).to_string(),
            ],
        )
        .unwrap();
    drop(guard);
}

fn silent_rows(index: &Mutex<Index>, project: ProjectId) -> (i64, i64) {
    let guard = index.lock().unwrap();
    let counts = guard
        .conn()
        .query_row(
            "SELECT count(*), count(*) FILTER (WHERE json_extract(meta, '$.silent') IS 1)
               FROM xp_events WHERE project_id = ?1 AND track = 'git'",
            [project.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    drop(guard);
    counts
}

/// Class 1 through J4 over a real repository: a project's first read raises the level with no
/// line and marks every row it writes silent; its next settle on a new commit-day is forward.
#[test]
fn ac_p4_43_5_first_read() {
    let _serial = serial();
    let mut rig = BadgesRig::new();
    // 110 XP before the read (level 1), 350 after it: one more commit-day reaches level 3 (360).
    library_rows(&rig.index, "amnesty_verdict", 11);
    let shown = present(&rig.index, T0);
    let days: Vec<i64> = (0..24).map(|i| D1 + i).collect();
    let project = rig.project("read", &days, Flags::default());
    rig.settle_j4(project);

    let after = seen(&rig.index, T0);
    let (rows, silent) = silent_rows(&rig.index, project);
    eprintln!("rows the first read wrote: {rows}, silent: {silent}");
    assert!(rows > 0 && rows == silent, "every first-read row is silent");
    assert!(
        after.displayed.unwrap() > shown,
        "the first read raised the level"
    );
    assert_eq!(after.line, None, "the first read made a line");
    assert_eq!(after.shown, after.displayed);

    rig.commit_file(project, "b.txt", b"two\n", D1 + 40);
    rig.settle_j4(project);
    let forward = seen(&rig.index, T0);
    let line = forward.line.expect("a forward commit-day makes a line");
    assert_eq!(line.form, MotivationLineForm::Rise);
    assert_eq!(line.level, forward.displayed);
    assert!(forward.displayed > after.displayed);
    assert_eq!(line.reason.expect("a reason").term, Some(XpTerm::CommitDay));
}

/// The first answer of the identity card, the floor not yet ratcheting.
fn unanswered(rig: &BadgesRig) {
    rig.sql("UPDATE identity SET confirmed_at = NULL");
}

/// Class 2: a narrowing `identity.confirm {apply: true}` through the first-run dispatcher, and
/// `projects.merge` through the identity dispatcher, each lower the displayed level and the level
/// shown follows it down with no line.
#[test]
fn ac_p4_43_5_confirmed() {
    let _serial = serial();
    let mut rig = BadgesRig::new();
    unanswered(&rig);
    library_rows(&rig.index, "amnesty_verdict", 1);
    let project = rig.project(
        "mine",
        &(0..13).map(|i| D1 + i).collect::<Vec<_>>(),
        Flags::default(),
    );
    rig.settle_j4(project);
    let before = present(&rig.index, T0);

    let home = tempfile::tempdir().unwrap();
    let env = codotheca_core::firstrun::FirstRunEnv {
        sources: codotheca_core::firstrun::sources::SourceEnv {
            home: home.path().to_path_buf(),
            app_data: None,
            xdg_config: None,
        },
        classifier: Arc::new(codotheca_core::firstrun::classify::FixedClassifier::new(
            vec![],
        )),
        distros: Arc::new(codotheca_core::firstrun::classify::NoDistros),
        platform: codotheca_core::index::path::PathPlatform::Unix,
        skip: codotheca_core::scan::skiplist::SkipList::default(),
        cache: codotheca_core::firstrun::roots::SuggestionCache::new(),
    };
    let mut guard = rig.index.lock().unwrap();
    codotheca_core::firstrun::dispatch(
        guard.conn_mut(),
        &env,
        "identity.confirm",
        &serde_json::json!({ "emails": [], "apply": true }),
        T0,
    )
    .expect("identity.confirm is first run's")
    .expect("identity.confirm answered");
    drop(guard);
    let narrowed = seen(&rig.index, T0);
    assert!(
        narrowed.displayed.unwrap() < before,
        "the narrowing lowered the level"
    );
    assert_eq!(
        narrowed.shown, narrowed.displayed,
        "the level shown followed it down"
    );
    assert_eq!(narrowed.line, None);

    // A merge of two of the user's projects deletes both sides' git rows.
    let mut merging = BadgesRig::new();
    unanswered(&merging);
    library_rows(&merging.index, "amnesty_verdict", 1);
    let a = merging.project(
        "a",
        &(0..7).map(|i| D1 + i).collect::<Vec<_>>(),
        Flags::default(),
    );
    let b = merging.project(
        "b",
        &(20..27).map(|i| D1 + i).collect::<Vec<_>>(),
        Flags::default(),
    );
    merging.settle_j4(a);
    merging.settle_j4(b);
    let unmerged = present(&merging.index, T0);
    let held = merging.index.lock().unwrap();
    let ctx = codotheca_core::identity::commands::IdentityCtx {
        index: &held,
        events: merging.events.as_ref(),
        now: T0,
    };
    codotheca_core::identity::commands::dispatch_identity_command(
        &ctx,
        "projects.merge",
        serde_json::json!({ "a": a.0, "b": b.0 }),
    )
    .expect("projects.merge is identity's")
    .expect("the merge commits");
    drop(held);
    let merged = seen(&merging.index, T0);
    assert!(
        merged.displayed.unwrap() < unmerged,
        "the merge lowered the level"
    );
    assert_eq!(merged.shown, merged.displayed);
    assert_eq!(merged.line, None);
}

/// An index under a core handler, its fingerprint recorded by a first start, holding 13
/// `amnesty_verdict` rows (130 XP, level 2) and that level presented.
fn started() -> (
    tempfile::TempDir,
    Arc<Mutex<Index>>,
    codotheca_core::testing::CoreHandlerParts,
) {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open_at(dir.path(), T0).unwrap()));
    let launcher: Arc<dyn ScanLauncher> = Arc::new(HeldScanLauncher::new(Arc::clone(&index)));
    let mut parts = core_handler(dir.path(), &index, T0, launcher);
    library_rows(&index, "amnesty_verdict", 13);
    let first = run_startup(&mut parts.handler);
    assert!(
        !first.ledger.unwrap().changed,
        "a first record is not a change"
    );
    assert_eq!(present(&index, T0), 2);
    (dir, index, parts)
}

/// Class 3 through `run_startup`: a changed price table raises the level with no line; a forward
/// row already past the mark keeps its line, at the new level.
#[test]
fn ac_p4_43_5_rebalance() {
    let _serial = serial();
    {
        let (_dir, index, mut parts) = started();
        let _prices = override_prices(codotheca_core::ledger::price::PriceTable {
            amnesty_verdict: 30,
            ..PRICES
        });
        let summary = run_startup(&mut parts.handler);
        assert!(
            summary.ledger.unwrap().changed,
            "the start detected the change"
        );
        let after = seen(&index, T0);
        assert_eq!(after.displayed, Some(3), "390 XP is level 3");
        assert_eq!(after.shown, Some(3));
        assert_eq!(after.line, None, "a rebalance made a line");
    }

    let (_dir, index, mut parts) = started();
    library_rows(&index, "clean_archive", 2);
    assert_eq!(
        seen(&index, T0).line.map(|l| l.form),
        Some(MotivationLineForm::Rise)
    );
    let _prices = override_prices(codotheca_core::ledger::price::PriceTable {
        amnesty_verdict: 30,
        ..PRICES
    });
    assert!(run_startup(&mut parts.handler).ledger.unwrap().changed);
    let kept = seen(&index, T0);
    assert_eq!(kept.shown, Some(2), "a pending line is not absorbed");
    let line = kept.line.expect("the forward line stands");
    assert_eq!(line.form, MotivationLineForm::Rise);
    assert_eq!(line.level, kept.displayed, "at the rebalanced level");
}

/// Export, close, corrupt and rebuild the index in `data`, through the production entry.
fn rebuilt(index: Arc<Mutex<Index>>, data: &std::path::Path, now: i64) -> Arc<Mutex<Index>> {
    index.lock().unwrap().export_sidecar(now).unwrap();
    drop(Arc::try_unwrap(index).unwrap());
    std::fs::write(Index::db_path(data), b"this is not a database").unwrap();
    match rebuild_in_place(data, now + 1) {
        Ok(RebuildOutcome::Rebuilt(report)) => eprintln!("restored {:?}", report.restored),
        other => panic!("expected a rebuild, got {other:?}"),
    }
    Arc::new(Mutex::new(Index::open_at(data, now + 2).unwrap()))
}

/// Class 4: a rebuild restores the ledger with no line and the mark at the restored ledger's
/// top, and a copy's records the hand-off applies later do the same.
#[test]
fn ac_p4_43_5_restore() {
    let _serial = serial();
    let mut rig = handoff_rig::Rig::new();
    let now = handoff_rig::NOW;
    let data = rig.dir.path().join("index");
    let path = rig.repo("kept", "first");
    let found = rig.hand_off(&path);
    let subject = {
        let guard = rig.index.lock().unwrap();
        let key = subject_for_project(guard.conn(), found.project)
            .unwrap()
            .unwrap()
            .to_key();
        drop(guard);
        key
    };
    // 300 XP on the copy (level 2), 10 with no project.
    {
        let guard = rig.index.lock().unwrap();
        for i in 0..2 {
            guard
                .conn()
                .execute(
                    "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind,
                                            dedupe_key, track, meta)
                     VALUES (?1, 0, ?2, ?3, 'clean_archive', ?4, 'session', NULL)",
                    rusqlite::params![D1 * DAY + i, found.project.0, subject, format!("ca:{i}")],
                )
                .unwrap();
        }
        drop(guard);
    }
    library_rows(&rig.index, "amnesty_verdict", 1);
    assert_eq!(present(&rig.index, now), 2);

    // The rebuild: the copy's records wait for it; the library row comes back at once.
    let placeholder = tempfile::tempdir().unwrap();
    let held = std::mem::replace(
        &mut rig.index,
        Arc::new(Mutex::new(Index::open_at(placeholder.path(), now).unwrap())),
    );
    rig.index = rebuilt(held, &data, now + 10);
    let restored = seen(&rig.index, now + 20);
    assert_eq!(restored.line, None, "the rebuild made a line");
    assert_eq!(
        restored.since,
        Some(restored.max_id),
        "the mark is the restored ledger's top"
    );

    // The hand-off finds the copy and applies its records.
    rig.hand_off(&path);
    let applied = seen(&rig.index, now + 30);
    assert_eq!(applied.displayed, Some(2), "the copy's rows came back");
    assert_eq!(applied.line, None, "the hand-off's restore made a line");
    assert_eq!(applied.shown, Some(2));
    assert_eq!(applied.since, Some(applied.max_id));
}

/// The evaluator as a settle hook runs it, on a project whose jobs have read its authorship and
/// its refs, so `remote` is answered rather than unknown.
fn evaluate(index: &Mutex<Index>, project: ProjectId, now: i64) {
    let mut guard = index.lock().unwrap();
    guard
        .conn()
        .execute(
            "UPDATE project SET authored_by_user = 1 WHERE id = ?1",
            [project.0],
        )
        .unwrap();
    guard
        .conn()
        .execute(
            "UPDATE location SET refstate_observed_at = ?2 WHERE project_id = ?1",
            [project.0, now],
        )
        .unwrap();
    guard
        .with_tx(|tx| {
            evaluate_and_write(
                tx,
                project,
                LocalInstant {
                    unix: now,
                    offset_min: 0,
                },
            )
        })
        .unwrap();
    drop(guard);
}

/// Class 4 at the evaluator: a copy's record staged for a project the evaluator has never
/// written rows for is applied by that first projection write. A level was shown, the record's
/// rows raise it, and no line is made: the level shown follows them.
#[test]
fn ac_p4_43_5_restore_at_completion() {
    let _serial = serial();
    let mut rig = handoff_rig::Rig::new();
    let now = handoff_rig::NOW;
    let path = rig.repo("kept", "first");
    // The copy's record: 300 XP on it (level 2), exported by the index that wrote them.
    let found = rig.hand_off(&path);
    let doc = {
        let guard = rig.index.lock().unwrap();
        let subject = subject_for_project(guard.conn(), found.project)
            .unwrap()
            .unwrap()
            .to_key();
        for i in 0..2 {
            guard
                .conn()
                .execute(
                    "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind,
                                            dedupe_key, track, meta)
                     VALUES (?1, 0, ?2, ?3, 'clean_archive', ?4, 'session', NULL)",
                    rusqlite::params![D1 * DAY + i, found.project.0, subject, format!("ca:{i}")],
                )
                .unwrap();
        }
        let doc = export(guard.conn(), 1, now).unwrap();
        drop(guard);
        doc
    };

    // Another index finds the same copy and never evaluates it; 10 XP of its own (level 1) is
    // the level shown. Then the copy's record is staged.
    let other = tempfile::tempdir().unwrap();
    rig.index = Arc::new(Mutex::new(Index::open_at(other.path(), now).unwrap()));
    let project = rig.hand_off(&path).project;
    library_rows(&rig.index, "amnesty_verdict", 1);
    assert_eq!(present(&rig.index, now), 1);
    let staged = rig
        .index
        .lock()
        .unwrap()
        .with_tx(|tx| stage_pending(tx, &doc, now + 10))
        .unwrap();
    assert!(staged > 0, "the export staged nothing");
    let waiting = seen(&rig.index, now + 20);
    assert_eq!(
        (waiting.displayed, waiting.shown, waiting.line),
        (Some(1), Some(1), None),
        "nothing applies the record before the evaluator"
    );

    evaluate(&rig.index, project, now + 30);
    let applied = seen(&rig.index, now + 40);
    assert_eq!(applied.displayed, Some(2), "the copy's rows came back");
    assert_eq!(applied.line, None, "the evaluator's restore made a line");
    assert_eq!(
        applied.shown,
        Some(2),
        "the level shown follows the restore"
    );
    assert_eq!(applied.since, Some(applied.max_id));
}

/// AC-P4-43-5: a forward `debt_day` crossing a threshold is one `rise` naming it; a new root's
/// first read while that line is pending raises the line's level and leaves its reason at one
/// transition, because every row the read wrote is silent.
#[test]
fn ac_p4_43_5() {
    let _serial = serial();
    let mut rig = BadgesRig::new();
    let first = rig.project(
        "first",
        &(0..5).map(|i| D1 + i).collect::<Vec<_>>(),
        Flags::default(),
    );
    rig.settle_j4(first);
    // 50 XP of commit-days and 60 of verdicts: 110, so one debt day (15) crosses into level 2.
    library_rows(&rig.index, "amnesty_verdict", 6);
    let shown = present(&rig.index, T0);
    debt_day(&rig.index, first, D1 + 30);
    let pending = seen(&rig.index, T0);
    let line = pending.line.expect("the debt day crossed a threshold");
    assert_eq!(line.form, MotivationLineForm::Rise);
    let reason = line.reason.expect("a reason");
    assert_eq!(
        (reason.transitions, reason.term),
        (1, Some(XpTerm::DebtDay))
    );
    assert!(pending.displayed.unwrap() > shown);

    let second = rig.project(
        "second",
        &(0..30).map(|i| D1 + 100 + i).collect::<Vec<_>>(),
        Flags::default(),
    );
    rig.settle_j4(second);
    let (rows, silent) = silent_rows(&rig.index, second);
    eprintln!("the new root's first read wrote {rows} rows, {silent} silent");
    let folded = seen(&rig.index, T0);
    let raised = folded.line.expect("the line stands");
    assert!(
        folded.displayed > pending.displayed,
        "the first read raised the level"
    );
    assert_eq!(
        raised.level, folded.displayed,
        "the line carries the raised level"
    );
    assert_eq!(folded.shown, Some(shown), "a pending line is not absorbed");
    let folded_reason = raised.reason.expect("a reason");
    eprintln!("reason after the fold: {folded_reason:?}");
    assert_eq!(
        folded_reason.transitions, 1,
        "a silent row entered the reason"
    );
    assert_eq!(folded_reason.term, Some(XpTerm::DebtDay));
}

static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn record(sql: &str) {
    TRACE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(sql.to_owned());
}

/// Statements touching `level_floor` while `rig` settles `project` with `traced` on.
fn floor_reads(rig: &BadgesRig, project: ProjectId) -> (usize, usize) {
    TRACE.lock().unwrap().clear();
    rig.index.lock().unwrap().conn_mut().trace(Some(record));
    rig.settle_j4(project);
    rig.index.lock().unwrap().conn_mut().trace(None);
    let all = std::mem::take(&mut *TRACE.lock().unwrap());
    let floor = all.iter().filter(|s| s.contains("level_floor")).count();
    (all.len(), floor)
}

/// While no level was presented, a first read's absorb reads no level: no statement touches
/// the floor. Once one was, the same settle does.
#[test]
fn ac_p4_43_5_inert_while_unpresented() {
    let _serial = serial();
    let mut rig = BadgesRig::new();
    let unpresented = rig.project("quiet", &[D1, D1 + 1], Flags::default());
    let (statements, floor) = floor_reads(&rig, unpresented);
    eprintln!("unpresented settle: {statements} statements, {floor} on the floor");
    assert!(statements > 0, "the trace saw nothing");
    assert_eq!(floor, 0);

    present(&rig.index, T0);
    let presented = rig.project("seen", &[D1 + 5], Flags::default());
    let (traced, touched) = floor_reads(&rig, presented);
    eprintln!("presented settle: {traced} statements, {touched} on the floor");
    assert!(
        touched > 0,
        "the absorb read no level once one was presented"
    );
}
