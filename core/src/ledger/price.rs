//! §38.3: the one price table — every price, cap and gap the ledger applies. Every writer, the
//! XP function and the marginal price read it here; no price is written anywhere else.
//!
//! **Every cap is a limit, never a multiplier.** What passes a cap is dropped; nothing tapers.
//!
//! The table and the curve are the arbiter's default and the user may change them, so start-up
//! records a fingerprint of both (`record_ledger_fingerprint`): a change is then something a
//! reader can see, rather than a level that silently moved.

use super::marginal::RowDue;
use crate::health::acknowledge::read_acknowledged_at;
use crate::health::enrolment::is_enrolled;
use crate::index::subject::subject_for_project;
use crate::index::IndexError;
use crate::protocol::ProjectId;
use rusqlite::{Connection, OptionalExtension as _};

/// What each source of XP is worth, and the limits on how much of it counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriceTable {
    /// One counted commit-day.
    pub commit_day: i64,
    /// How many commit-days count per local date, across the whole library.
    pub commit_days_per_date: i64,
    /// A counted commit-day that ends a dormancy of at least `REVIVAL_GAP_DAYS`.
    pub revival: i64,
    /// The first project in a language, once per language ever.
    pub language: i64,
    /// One release date of a project.
    pub release: i64,
    /// How many release dates of one project count per local calendar month.
    pub releases_per_month: i64,
    /// Seconds of focused session time per XP point.
    pub focus_seconds_per_xp: i64,
    /// The most focus XP one local date pays, across the whole library.
    pub focus_per_date: i64,
    /// One `debt_day` row: a day on which a scored health item was fixed.
    pub debt_day: i64,
    /// A completion check passing, once per project and check, ever.
    pub completion_check: i64,
    /// One amnesty verdict, or one pre-pass group approval.
    pub amnesty_verdict: i64,
    /// A project archived cleanly, once ever.
    pub clean_archive: i64,
}

/// §38.3's table, as ruled.
pub const PRICES: PriceTable = PriceTable {
    commit_day: 10,
    commit_days_per_date: 3,
    revival: 150,
    language: 50,
    release: 40,
    releases_per_month: 2,
    focus_seconds_per_xp: 180,
    focus_per_date: 40,
    debt_day: 15,
    completion_check: 40,
    amnesty_verdict: 10,
    clean_archive: 150,
};

/// The one dormancy number: a commit-day this many days or more after its project's previous
/// one is a revival. In days, never "six months" — calendar months vary in length.
pub const REVIVAL_GAP_DAYS: i64 = 183;

/// Whether a commit-day on local day `day` revives a project whose previous commit-day was
/// `prev_day`. The bound is inclusive.
#[must_use]
pub const fn is_revival_gap(prev_day: i64, day: i64) -> bool {
    day - prev_day >= REVIVAL_GAP_DAYS
}

/// The table every ledger reader prices through: `PRICES`, unless a test has overridden it.
#[must_use]
pub fn prices() -> PriceTable {
    #[cfg(feature = "testkit")]
    if let Some(table) = testkit::prices() {
        return table;
    }
    PRICES
}

/// The `app_meta` key holding the fingerprint of the table and curve the index was last opened
/// under.
pub const LEDGER_FINGERPRINT_KEY: &str = "ledger_fingerprint";

/// Lowercase hex SHA-256 of a canonical text of the price table, `REVIVAL_GAP_DAYS` and the
/// level thresholds — every number that decides what a ledger is worth.
#[must_use]
pub fn ledger_fingerprint() -> String {
    use sha2::Digest as _;
    use std::fmt::Write as _;
    let p = prices();
    let mut text = format!(
        "prices:commit_day={},commit_days_per_date={},revival={},language={},release={},\
         releases_per_month={},focus_seconds_per_xp={},focus_per_date={},debt_day={},\
         completion_check={},amnesty_verdict={},clean_archive={};revival_gap_days={};thresholds=",
        p.commit_day,
        p.commit_days_per_date,
        p.revival,
        p.language,
        p.release,
        p.releases_per_month,
        p.focus_seconds_per_xp,
        p.focus_per_date,
        p.debt_day,
        p.completion_check,
        p.amnesty_verdict,
        p.clean_archive,
        REVIVAL_GAP_DAYS,
    );
    for level in 1..=200 {
        let _ = write!(text, "{},", super::curve::threshold(level));
    }
    crate::index::sidecar::hex(&sha2::Sha256::digest(text.as_bytes()))
}

/// What `record_ledger_fingerprint` found stored before it ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintChange {
    /// The fingerprint stored before this call; `None` on an index that never recorded one.
    pub previous: Option<String>,
    /// A different fingerprint was stored: the table or the curve changed since it was recorded.
    /// A first record is not a change — nothing was priced under another table.
    pub changed: bool,
}

/// Store the current fingerprint when it is absent or different, and say which it was.
///
/// # Errors
/// Returns [`IndexError`] when the read or the write fails.
pub fn record_ledger_fingerprint(conn: &Connection) -> Result<FingerprintChange, IndexError> {
    let current = ledger_fingerprint();
    let previous: Option<String> = conn
        .query_row(
            "SELECT v FROM app_meta WHERE k = ?1",
            [LEDGER_FINGERPRINT_KEY],
            |r| r.get(0),
        )
        .optional()?;
    if previous.as_deref() != Some(current.as_str()) {
        conn.execute(
            "INSERT INTO app_meta (k, v) VALUES (?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            [LEDGER_FINGERPRINT_KEY, current.as_str()],
        )?;
    }
    let changed = previous.as_ref().is_some_and(|p| *p != current);
    Ok(FingerprintChange { previous, changed })
}

/// How many `commit_day` rows a project needs before a completion check can latch (§38.8.2).
pub const COMPLETION_GATE_COMMIT_DAYS: i64 = 3;

/// §38.8.1's project gates for a `debt_day`: a subject to key the row on, enrolled, and not
/// Reference. The closure's own gates — `Fixed`, of a `scored` item — are the writer's.
///
/// `None` when the row may be written; otherwise what the row is instead, read by the writer and
/// by the marginal price alike: [`RowDue::Refused`] (price `0`) or [`RowDue::NotComputable`] (no
/// price, distinct from `0`).
///
/// # Errors
/// Returns [`IndexError`] when a read fails.
pub fn debt_day_gate(conn: &Connection, project: ProjectId) -> Result<Option<RowDue>, IndexError> {
    // A project with no subject is paid nothing (§38.2): never an empty key, which would make
    // every such project collide on `debt_day::<date>`.
    if subject_for_project(conn, project)?.is_none() {
        return Ok(Some(RowDue::NotComputable));
    }
    // *Enrolled*, never *not suppressed*: both suppression predicates also read `is_archived`,
    // and an acknowledged, archived project is enrolled and is paid.
    if !is_enrolled(read_acknowledged_at(conn, project)?) {
        return Ok(Some(RowDue::Refused));
    }
    // The column, not `jobs::scheduler::is_reference`: that reader answers `None` for a project
    // J1.5 has not classified, whose column is `0` and which is paid. A Reference project earns
    // nothing new (§38.3a).
    let is_reference: Option<i64> = conn
        .query_row(
            "SELECT is_reference FROM project WHERE id = ?1",
            [project.0],
            |r| r.get(0),
        )
        .optional()?;
    Ok((is_reference != Some(0)).then_some(RowDue::Refused))
}

/// §38.8.2's gate for a completion check's latch, answered as [`debt_day_gate`] answers.
///
/// Open (`None`) when authored by the user, not Reference, not merged away, and with at least
/// [`COMPLETION_GATE_COMMIT_DAYS`] `commit_day` rows. Not computable with no subject or with
/// authorship not yet classified.
///
/// # Errors
/// Returns [`IndexError`] when a read fails.
pub fn completion_gate(
    conn: &Connection,
    project: ProjectId,
) -> Result<Option<RowDue>, IndexError> {
    if subject_for_project(conn, project)?.is_none() {
        return Ok(Some(RowDue::NotComputable));
    }
    let row: Option<(Option<i64>, i64, bool, i64)> = conn
        .query_row(
            "SELECT p.authored_by_user, p.is_reference, p.merged_into IS NULL,
                    (SELECT count(*) FROM xp_events x
                      WHERE x.project_id = p.id AND x.kind = 'commit_day')
               FROM project p WHERE p.id = ?1",
            [project.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((Some(authored), is_reference, unmerged, commit_days)) = row else {
        return Ok(Some(RowDue::NotComputable));
    };
    let open = authored == 1
        && is_reference == 0
        && unmerged
        && commit_days >= COMPLETION_GATE_COMMIT_DAYS;
    Ok((!open).then_some(RowDue::Refused))
}

/// A test-only override of the price table and the curve, so a test can show what a change to
/// either does to a library without editing the constants.
///
/// The slot is process-global, so a test binary that uses it must serialise its tests.
#[cfg(feature = "testkit")]
pub mod testkit {
    use super::PriceTable;
    use std::sync::{Mutex, PoisonError};

    static PRICES: Mutex<Option<PriceTable>> = Mutex::new(None);
    static CURVE: Mutex<Option<[i64; 200]>> = Mutex::new(None);

    /// Puts back what an override replaced, when it is dropped.
    #[derive(Debug)]
    #[must_use = "the override ends when the guard is dropped"]
    pub struct OverrideGuard {
        previous: Previous,
    }

    /// The slot an override replaced, and what it held.
    #[derive(Debug)]
    enum Previous {
        Prices(Option<PriceTable>),
        Curve(Box<Option<[i64; 200]>>),
    }

    impl Drop for OverrideGuard {
        fn drop(&mut self) {
            match &self.previous {
                Previous::Prices(p) => {
                    *PRICES.lock().unwrap_or_else(PoisonError::into_inner) = *p;
                }
                Previous::Curve(c) => {
                    *CURVE.lock().unwrap_or_else(PoisonError::into_inner) = **c;
                }
            }
        }
    }

    /// Price the ledger by `table` until the guard drops.
    pub fn override_prices(table: PriceTable) -> OverrideGuard {
        let previous = PRICES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(table);
        OverrideGuard {
            previous: Previous::Prices(previous),
        }
    }

    /// Read `thresholds` as `T(1..=200)` until the guard drops.
    pub fn override_curve(thresholds: [i64; 200]) -> OverrideGuard {
        let previous = CURVE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(thresholds);
        OverrideGuard {
            previous: Previous::Curve(Box::new(previous)),
        }
    }

    pub(super) fn prices() -> Option<PriceTable> {
        *PRICES.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(in crate::ledger) fn curve() -> Option<[i64; 200]> {
        *CURVE.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
