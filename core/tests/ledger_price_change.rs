//! §38.6.3: a change to the price table or the curve never lowers the displayed level — the floor
//! holds it, and `floored` says so while it does. `AC-P4-38-8`'s price and curve clauses; the
//! criterion's own walk over every step is `acceptance_ledger.rs`'s.
//!
//! The overrides are process-global, so this binary holds one test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::index::Index;
use codotheca_core::ledger::curve::LEVEL_THRESHOLDS;
use codotheca_core::ledger::level::read_level;
use codotheca_core::ledger::price::testkit::{override_curve, override_prices};
use codotheca_core::ledger::price::{PriceTable, PRICES};
use codotheca_core::protocol::{LevelFigures, LevelReading, LevelState};

const T0: i64 = 1_700_000_000;

fn read(index: &mut Index, now: i64) -> LevelReading {
    index.with_tx(|tx| read_level(tx, now)).unwrap()
}

/// A computed reading's figures.
const fn figures(reading: &LevelReading) -> &LevelFigures {
    reading
        .figures
        .as_ref()
        .expect("a computed reading carries its figures")
}

/// Every price halved; the caps and the focus rate, which are limits rather than prices, kept.
const fn halved(p: PriceTable) -> PriceTable {
    PriceTable {
        commit_day: p.commit_day / 2,
        revival: p.revival / 2,
        language: p.language / 2,
        release: p.release / 2,
        debt_day: p.debt_day / 2,
        completion_check: p.completion_check / 2,
        amnesty_verdict: p.amnesty_verdict / 2,
        clean_archive: p.clean_archive / 2,
        ..p
    }
}

#[test]
fn a_price_or_curve_change_never_lowers_the_displayed_level() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open_at(dir.path(), T0).unwrap();
    let conn = index.conn();
    // Answered, and computed: the one project has no copy left to walk, so nothing is pending.
    conn.execute(
        "INSERT INTO identity (email, is_user, source, confirmed_at)
         VALUES ('a@example.invalid', 1, 'gitconfig', ?1)",
        [T0],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES ('p', 'p', 0, 0)",
        [],
    )
    .unwrap();
    let project = conn.last_insert_rowid();
    for i in 0..30 {
        conn.execute(
            "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                    track)
             VALUES (?1, 0, ?2, 's', 'completion_check', ?3, 'session')",
            rusqlite::params![T0, project, format!("completion_check:{project}:{i}")],
        )
        .unwrap();
    }

    let earned = read(&mut index, T0);
    eprintln!("as ruled: {earned:?}");
    assert_eq!(earned.state, LevelState::Computed);
    assert!(!figures(&earned).floored);
    let level = figures(&earned).level;
    assert!(
        level > 2,
        "the fixture's level must be worth protecting, got {level}"
    );

    {
        let _prices = override_prices(halved(PRICES));
        let reading = read(&mut index, T0 + 1);
        eprintln!("prices halved: {reading:?}");
        assert!(
            figures(&reading).xp < figures(&earned).xp,
            "halving every price moved no XP"
        );
        assert_eq!(
            figures(&reading).level,
            level,
            "a price change lowered the level"
        );
        assert!(figures(&reading).floored);
    }
    let restored = read(&mut index, T0 + 2);
    assert_eq!(restored, earned, "the override's drop restores the reading");
    assert!(!figures(&restored).floored);

    {
        let _curve = override_curve(LEVEL_THRESHOLDS.map(|t| t * 2));
        let reading = read(&mut index, T0 + 3);
        eprintln!("thresholds doubled: {reading:?}");
        assert_eq!(figures(&reading).xp, figures(&earned).xp);
        assert_eq!(
            figures(&reading).level,
            level,
            "a curve change lowered the level"
        );
        assert!(figures(&reading).floored);
    }
    let uncurved = read(&mut index, T0 + 4);
    assert_eq!(
        uncurved, earned,
        "the curve override's drop restores the reading"
    );
    assert!(!figures(&uncurved).floored);
}
