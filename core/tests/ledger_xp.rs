#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §38.3–§38.4: the one price table, the curve, the XP function by term, and the fingerprint
//! of the table that start-up records.

use std::sync::Arc;

use codotheca_core::assembly::{CoreDeps, CoreHandler};
use codotheca_core::index::Index;
use codotheca_core::ledger::curve::{cost, level_of, threshold, LEVEL_THRESHOLDS};
use codotheca_core::ledger::price::{
    is_revival_gap, ledger_fingerprint, record_ledger_fingerprint, FingerprintChange,
    LEDGER_FINGERPRINT_KEY, PRICES, REVIVAL_GAP_DAYS,
};
use codotheca_core::ledger::xp::{language_firsts, xp_by_term, LanguageFirst, XpTotals};
use codotheca_core::proto::dispatch::CommandHandler as _;
use codotheca_core::proto::pubsub::{Publisher, PublisherSink};
use codotheca_core::protocol::{ProjectId, XpTerm};
use rusqlite::Connection;

const SCHEMA: &str = include_str!("../../protocol/schema/protocol.json");

const NOW: i64 = 1_750_000_000;
const DAY: i64 = 86_400;

/// `name`'s variants as `protocol.json` declares them.
fn schema_variants(name: &str) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(SCHEMA).expect("protocol.json parses");
    let decl = doc["types"]
        .get(name)
        .unwrap_or_else(|| panic!("{name} is not declared in protocol.json"));
    assert_eq!(decl["kind"], "enum", "{name} is not an enum");
    let variants: Vec<String> = decl["variants"]
        .as_array()
        .unwrap_or_else(|| panic!("{name} declares no variants"))
        .iter()
        .map(|v| v.as_str().expect("a variant is a string").to_owned())
        .collect();
    assert!(!variants.is_empty(), "{name} declares no variants");
    variants
}

/// A fully migrated, empty library.
fn library() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open_at(dir.path(), NOW).unwrap();
    (dir, index)
}

fn project(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn set(conn: &Connection, project: i64, column: &str, value: i64) {
    conn.execute(
        &format!("UPDATE project SET {column} = ?2 WHERE id = ?1"),
        rusqlite::params![project, value],
    )
    .unwrap();
}

fn language(conn: &Connection, project: i64, name: &str) {
    conn.execute(
        "UPDATE project SET primary_language = ?2 WHERE id = ?1",
        rusqlite::params![project, name],
    )
    .unwrap();
}

/// One ledger row. The dedupe key only has to be unique, so it carries the next id.
fn row(
    conn: &Connection,
    project: Option<i64>,
    kind: &str,
    ts: i64,
    tz_offset_min: Option<i64>,
    meta: Option<&str>,
) {
    let track = if ["commit_day", "release"].contains(&kind) {
        "git"
    } else {
        "session"
    };
    let next: i64 = conn
        .query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM xp_events", [], |r| {
            r.get(0)
        })
        .unwrap();
    conn.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (?1, ?2, ?3, 'lineage:k|remote:', ?4, ?5, ?6, ?7)",
        rusqlite::params![
            ts,
            tz_offset_min,
            project,
            kind,
            format!("{kind}:test:{next}"),
            track,
            meta
        ],
    )
    .unwrap();
}

fn commit_day(conn: &Connection, project: i64, day: i64) {
    row(conn, Some(project), "commit_day", day * DAY, None, None);
}

fn release(conn: &Connection, project: i64, day: i64) {
    row(conn, Some(project), "release", day * DAY, None, None);
}

fn totals(conn: &Connection) -> XpTotals {
    xp_by_term(conn).unwrap()
}

/// **`AC-P4-38-6`.** The checked-in `T(1..=200)` is the formula, regenerated here from `cost`;
/// §38.4's named thresholds; `level_of` at every threshold and one below it.
#[test]
fn ac_p4_38_6_the_curve() {
    let mut regenerated = [0_i64; 200];
    let mut sum = 0_i64;
    for (n, slot) in (1_u32..).zip(regenerated.iter_mut()) {
        *slot = sum;
        sum += cost(n);
    }
    let mismatched: Vec<usize> = (0..200)
        .filter(|&i| regenerated[i] != LEVEL_THRESHOLDS[i])
        .collect();
    assert_eq!(
        mismatched,
        Vec::<usize>::new(),
        "LEVEL_THRESHOLDS differs from the formula at these indices; regenerated: {regenerated:?}"
    );

    for (level, at) in [
        (1, 0),
        (2, 120),
        (5, 1_200),
        (10, 5_400),
        (11, 6_600),
        (12, 8_630),
        (15, 16_220),
        (20, 34_150),
        (25, 59_180),
        (30, 91_860),
        (40, 181_810),
        (50, 306_970),
        (200, 7_625_400),
    ] {
        assert_eq!(threshold(level), at, "T({level})");
    }

    let mut checked = 0_u32;
    for level in 1_u32..=201 {
        assert_eq!(level_of(threshold(level)), level, "level_of(T({level}))");
        if level > 1 {
            assert_eq!(
                level_of(threshold(level) - 1),
                level - 1,
                "level_of(T({level}) - 1)"
            );
        }
        checked += 1;
    }
    assert_eq!(level_of(0), 1, "no XP is level 1");
    assert_eq!(
        threshold(201),
        LEVEL_THRESHOLDS[199] + cost(200),
        "above the table the formula continues"
    );
    eprintln!("thresholds compared: 200; level_of checked at {checked} thresholds");
}

/// **`AC-P4-38-10`.** Commit-days, revivals and languages, with every constant read from the
/// ledger module.
#[test]
fn ac_p4_38_10_commit_days_revival_languages() {
    let gap = REVIVAL_GAP_DAYS;
    assert!(!is_revival_gap(0, gap - 1), "one day short is no revival");
    assert!(is_revival_gap(0, gap), "the constant itself is a revival");
    counted_three_revivals_first();
    the_gap_is_from_the_preceding_commit_day();
    a_first_commit_day_is_never_a_revival();
    a_language_pays_once();
    a_shallow_history_counts_for_nothing();
}

/// Five projects on one date count three, revivals first. Four of them come back after the
/// gap; the fourth revival falls outside the counted three and pays nothing.
fn counted_three_revivals_first() {
    let gap = REVIVAL_GAP_DAYS;
    let (_dir, index) = library();
    let conn = index.conn();
    let day = 1_000 + gap;
    let ids: Vec<i64> = (1..=5).map(|i| project(conn, &format!("p{i}"))).collect();
    for &id in &ids {
        commit_day(conn, id, day);
    }
    for &id in ids.iter().skip(1) {
        commit_day(conn, id, 1_000);
    }
    let t = totals(conn);
    // 1 000: four projects, three counted, none a revival (each is a first commit-day).
    // `day`: five projects, three counted — the three lowest of the four revivals.
    assert_eq!(t.of(XpTerm::CommitDay), 6 * PRICES.commit_day);
    assert_eq!(
        t.of(XpTerm::Revival),
        3 * PRICES.revival,
        "the counted three are revivals first, and the fourth revival pays nothing"
    );
}

/// The gap is measured from the immediately preceding commit-day, and the constant is in.
fn the_gap_is_from_the_preceding_commit_day() {
    let gap = REVIVAL_GAP_DAYS;
    let (_dir, index) = library();
    let conn = index.conn();
    let short = project(conn, "short");
    commit_day(conn, short, 10);
    commit_day(conn, short, 10 + gap - 1);
    let exact = project(conn, "exact");
    commit_day(conn, exact, 20);
    commit_day(conn, exact, 20 + gap);
    let recent = project(conn, "recent");
    commit_day(conn, recent, 30);
    commit_day(conn, recent, 130);
    commit_day(conn, recent, 130 + gap - 1);
    let t = totals(conn);
    assert_eq!(t.of(XpTerm::CommitDay), 7 * PRICES.commit_day);
    assert_eq!(
        t.of(XpTerm::Revival),
        PRICES.revival,
        "only `exact` is a revival"
    );
}

/// A project's first commit-day is never a revival, however late it comes.
fn a_first_commit_day_is_never_a_revival() {
    let (_dir, index) = library();
    let conn = index.conn();
    let late = project(conn, "late");
    commit_day(conn, late, 50_000);
    let t = totals(conn);
    assert_eq!(t.of(XpTerm::CommitDay), PRICES.commit_day);
    assert_eq!(t.of(XpTerm::Revival), 0);
}

/// A language pays once across projects, dated at the earliest qualifying first commit-day;
/// a NULL language, a Reference project, a merged one and one with no commit-day never count.
fn a_language_pays_once() {
    let (_dir, index) = library();
    let conn = index.conn();
    let first = project(conn, "first");
    language(conn, first, "Rust");
    commit_day(conn, first, 400);
    let earlier = project(conn, "earlier");
    language(conn, earlier, "Rust");
    commit_day(conn, earlier, 300);
    commit_day(conn, earlier, 500);
    let unnamed = project(conn, "unnamed");
    commit_day(conn, unnamed, 600);
    let reference = project(conn, "reference");
    language(conn, reference, "Go");
    set(conn, reference, "is_reference", 1);
    commit_day(conn, reference, 700);
    let merged = project(conn, "merged");
    language(conn, merged, "Haskell");
    set(conn, merged, "merged_into", first);
    commit_day(conn, merged, 800);
    let idle = project(conn, "idle");
    language(conn, idle, "Zig");

    assert_eq!(totals(conn).of(XpTerm::Language), PRICES.language);
    assert_eq!(
        language_firsts(conn).unwrap(),
        vec![LanguageFirst {
            language: "Rust".to_owned(),
            day: 300,
            project_id: ProjectId(earlier),
        }]
    );
}

/// A shallow project's commit-days, gap and language count for nothing, and it takes
/// no counted slot from a full history on the same date.
fn a_shallow_history_counts_for_nothing() {
    let gap = REVIVAL_GAP_DAYS;
    let (_dir, index) = library();
    let conn = index.conn();
    let shallow = project(conn, "shallow");
    set(conn, shallow, "is_shallow", 1);
    language(conn, shallow, "Elixir");
    commit_day(conn, shallow, 100);
    commit_day(conn, shallow, 100 + gap);
    assert_eq!(
        totals(conn).total(),
        0,
        "a shallow history alone earns nothing"
    );
    assert_eq!(language_firsts(conn).unwrap(), vec![]);

    for i in 1..=3 {
        let full = project(conn, &format!("full{i}"));
        commit_day(conn, full, 100 + gap);
    }
    let t = totals(conn);
    assert_eq!(t.of(XpTerm::CommitDay), 3 * PRICES.commit_day);
    assert_eq!(t.of(XpTerm::Revival), 0);
    assert_eq!(t.of(XpTerm::Language), 0);
}

/// **`AC-P4-38-15`.** The one class of row the writer before §38.8.1's gates paid for a shown-only closure
/// prices 0; the same source beside a scored one prices as it stands. The list is parsed, so
/// spacing does not decide it.
#[test]
fn ac_p4_38_15_legacy_pricing() {
    let (_dir, index) = library();
    let conn = index.conn();
    let p = project(conn, "p");
    row(
        conn,
        Some(p),
        "debt_day",
        NOW,
        Some(0),
        Some(r#"{"sources":["abandoned_with_debt"]}"#),
    );
    row(
        conn,
        Some(p),
        "debt_day",
        NOW + DAY,
        Some(0),
        Some(r#"{ "sources" : [ "abandoned_with_debt" ] }"#),
    );
    assert_eq!(totals(conn).of(XpTerm::DebtDay), 0);

    row(
        conn,
        Some(p),
        "debt_day",
        NOW + 2 * DAY,
        Some(0),
        Some(r#"{"sources":["abandoned_with_debt","todo_marker"]}"#),
    );
    assert_eq!(totals(conn).of(XpTerm::DebtDay), PRICES.debt_day);
}

/// Changes `conn` so that, against what `base` left, exactly `term`'s addend moves.
fn light(conn: &Connection, term: XpTerm) {
    let gap = REVIVAL_GAP_DAYS;
    match term {
        XpTerm::CommitDay => {
            let p = project(conn, "p");
            commit_day(conn, p, 10);
        }
        // The two commit-days stay two; only the second one's distance from the first crosses
        // the gap.
        XpTerm::Revival => {
            conn.execute(
                "UPDATE xp_events SET ts = ?1 WHERE kind = 'commit_day' AND ts = ?2",
                [(10 + gap) * DAY, (10 + gap - 1) * DAY],
            )
            .unwrap();
        }
        XpTerm::Release => {
            let p = project(conn, "p");
            release(conn, p, 10);
        }
        XpTerm::Language => {
            conn.execute(
                "UPDATE project SET primary_language = 'Rust' WHERE name = 'p'",
                [],
            )
            .unwrap();
        }
        XpTerm::Focus => row(
            conn,
            None,
            "focus",
            NOW,
            Some(0),
            Some(r#"{"seconds":1800}"#),
        ),
        XpTerm::DebtDay => row(
            conn,
            None,
            "debt_day",
            NOW,
            Some(0),
            Some(r#"{"sources":["todo_marker"]}"#),
        ),
        XpTerm::CompletionCheck => row(conn, None, "completion_check", NOW, Some(0), None),
        XpTerm::AmnestyVerdict => row(conn, None, "amnesty_verdict", NOW, Some(0), None),
        XpTerm::CleanArchive => row(conn, None, "clean_archive", NOW, Some(0), None),
    }
}

/// What `term`'s lighting needs in place first: a revival and a language each need a
/// commit-day, and that commit-day's own price is the base, not the term.
fn base(conn: &Connection, term: XpTerm) {
    if matches!(term, XpTerm::Revival | XpTerm::Language) {
        let p = project(conn, "p");
        commit_day(conn, p, 10);
        if term == XpTerm::Revival {
            commit_day(conn, p, 10 + REVIVAL_GAP_DAYS - 1);
        }
    }
}

/// `XpTerm` is the XP function's own list: the schema's variants are exactly the terms
/// `XpTotals` carries, in order, and each term can be lit alone.
#[test]
fn xp_term_mirrors_the_xp_function() {
    let (_dir, index) = library();
    let declared = schema_variants("XpTerm");
    let carried: Vec<String> = totals(index.conn())
        .terms
        .iter()
        .map(|(term, _)| {
            serde_json::to_value(term)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(carried, declared);

    let mut lit = 0_usize;
    for term in XpTerm::ALL {
        let (_term_dir, term_index) = library();
        let conn = term_index.conn();
        base(conn, term);
        let before = totals(conn);
        light(conn, term);
        let after = totals(conn);
        for other in XpTerm::ALL {
            let moved = after.of(other) - before.of(other);
            if other == term {
                assert!(moved > 0, "lighting {term:?} did not move its own term");
            } else {
                assert_eq!(moved, 0, "lighting {term:?} moved {other:?}");
            }
        }
        assert_eq!(
            after.total(),
            after.terms.iter().map(|(_, xp)| xp).sum::<i64>(),
            "the terms sum to the XP"
        );
        lit += 1;
    }
    eprintln!("terms lit alone: {lit}");
    assert_eq!(lit, declared.len());
}

/// Every cap is a limit: what passes it is dropped, never scaled.
#[test]
fn caps_are_limits() {
    let (_dir, index) = library();
    let conn = index.conn();

    // 200 minutes of focus on one local date: 66 by the rate, 40 by the cap.
    for minutes in [50, 50, 50, 50] {
        row(
            conn,
            None,
            "focus",
            NOW,
            Some(0),
            Some(&format!(r#"{{"seconds":{}}}"#, minutes * 60)),
        );
    }
    assert_eq!(totals(conn).of(XpTerm::Focus), PRICES.focus_per_date);

    // Three release dates in one calendar month pay two.
    let p = project(conn, "p");
    let first_of_month = 19_723; // 2024-01-01
    for offset in [0, 9, 20] {
        release(conn, p, first_of_month + offset);
    }
    assert_eq!(
        totals(conn).of(XpTerm::Release),
        PRICES.releases_per_month * PRICES.release
    );

    // A NULL-project award counts.
    row(conn, None, "amnesty_verdict", NOW, Some(0), None);
    assert_eq!(
        totals(conn).of(XpTerm::AmnestyVerdict),
        PRICES.amnesty_verdict
    );
}

/// A `CoreHandler` over `dir`, with fakes for everything start-up does not read.
fn handler(dir: &std::path::Path) -> CoreHandler {
    let index = Arc::new(std::sync::Mutex::new(Index::open_at(dir, NOW).unwrap()));
    let clock = Arc::new(codotheca_core::testing::FakeClock::new(NOW));
    let events = Arc::new(PublisherSink::new(Publisher::detached()));
    let http: Arc<dyn codotheca_core::http::HttpTransport> =
        Arc::new(codotheca_core::testing::FakeTransport::new());
    let sync_observing = Arc::new(codotheca_core::sync::http::ObservingTransport::new(
        Arc::clone(&http),
        clock.clone(),
    ));
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    CoreHandler::new(CoreDeps {
        index: Arc::clone(&index),
        provider: Arc::new(codotheca_core::provider::GitHubProvider::new(
            Arc::clone(&http),
            codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
        )),
        tokens: Arc::new(codotheca_core::testing::FakeTokenStore::unavailable()),
        http,
        client_id: "test-client-id".to_owned(),
        clock: clock.clone(),
        git: Arc::new(codotheca_core::testing::FakeGitBackend::new()),
        write_git: Arc::new(codotheca_core::testing::FakeMutatingGit::new(
            codotheca_core::testing::CloneBehaviour::Succeed,
        )),
        trash: Arc::new(codotheca_core::testing::CountingTrash::new()),
        mount: Arc::new(codotheca_core::testing::FakeMountResolver::default()),
        spawner: Box::new(codotheca_core::launch::spawn::RecordingSpawner::new()),
        sessions: codotheca_core::session::manager::SessionManager::new(
            clock.clone(),
            events.clone(),
            Box::new(codotheca_core::session::watch::FakeActivitySource::new()),
            Arc::new(codotheca_core::session::activity::FakeIgnoreCheck::new(&[])),
        ),
        scans: codotheca_core::scan::ScanSupervisor::new(Arc::new(
            codotheca_core::testing::ScanLauncherFake::new(),
        )),
        scan_store: Arc::new(codotheca_core::testing::MemScanStore::new()),
        firstrun: codotheca_core::firstrun::FirstRunEnv {
            sources: codotheca_core::firstrun::sources::SourceEnv {
                home,
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
        },
        jobs: codotheca_core::assembly::jobs::JobPump::start(
            Arc::clone(&index),
            Arc::new(codotheca_core::testing::FakeGitBackend::new()),
            clock.clone(),
            events.clone(),
            Arc::new(codotheca_core::testing::zone::FixedZone(0)),
        ),
        sync: codotheca_core::assembly::sync::SyncPump::start(
            Arc::clone(&index),
            codotheca_core::sync::SyncDeps {
                provider: Arc::new(codotheca_core::provider::GitHubProvider::new(
                    sync_observing.clone(),
                    codotheca_core::provider::listing::GITHUB_CANONICAL_HOST.to_owned(),
                )),
                transport: sync_observing,
                tokens: Arc::new(codotheca_core::testing::FakeTokenStore::unavailable()),
                clock,
                cancel: codotheca_core::cancel::CancelToken::new(),
                zone: Arc::new(codotheca_core::testing::zone::FixedZone(0)),
            },
            events.clone(),
        ),
        events,
        tz_offset_min: 0,
        zone: Arc::new(codotheca_core::testing::zone::FixedZone(0)),
    })
}

fn stored_fingerprint(h: &CoreHandler) -> Option<String> {
    let guard = h.index().lock().unwrap();
    guard
        .conn()
        .query_row(
            "SELECT v FROM app_meta WHERE k = ?1",
            [LEDGER_FINGERPRINT_KEY],
            |r| r.get(0),
        )
        .ok()
}

/// Start-up records the fingerprint once, and reports a change only against a different
/// stored one. A first record is not a change: nothing was priced under another table.
#[test]
fn the_fingerprint_is_recorded_at_start() {
    let fingerprint = ledger_fingerprint();
    assert_eq!(fingerprint.len(), 64, "sha-256 hex: {fingerprint}");

    let dir = tempfile::tempdir().unwrap();
    let mut h = handler(dir.path());
    assert_eq!(stored_fingerprint(&h), None);

    let first = codotheca_core::assembly::startup::run_startup(&mut h);
    assert_eq!(
        first.ledger,
        Some(FingerprintChange {
            previous: None,
            changed: false,
        })
    );
    assert_eq!(stored_fingerprint(&h), Some(fingerprint.clone()));

    let second = codotheca_core::assembly::startup::run_startup(&mut h);
    assert_eq!(
        second.ledger,
        Some(FingerprintChange {
            previous: Some(fingerprint.clone()),
            changed: false,
        })
    );
    {
        let guard = h.index().lock().unwrap();
        let before = guard.conn().total_changes();
        record_ledger_fingerprint(guard.conn()).unwrap();
        assert_eq!(
            guard.conn().total_changes(),
            before,
            "an unchanged fingerprint writes nothing"
        );
        guard
            .conn()
            .execute(
                "UPDATE app_meta SET v = 'an-older-table' WHERE k = ?1",
                [LEDGER_FINGERPRINT_KEY],
            )
            .unwrap();
    }

    let third = codotheca_core::assembly::startup::run_startup(&mut h);
    assert_eq!(
        third.ledger,
        Some(FingerprintChange {
            previous: Some("an-older-table".to_owned()),
            changed: true,
        })
    );
    assert_eq!(stored_fingerprint(&h), Some(fingerprint));
    h.shutdown();
}
