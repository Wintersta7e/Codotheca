//! §32.8's verdict: **three values, two clocks, and an asymmetric expiry.**
//!
//! The verdict is derived from the tables and **stored nowhere**. It is not a fourth column,
//! because every input already has an owner. The per-project answer is the join, and the join is a
//! **pure recompute** — produced for an offline, frozen, backlogged or uninstalled project
//! **without reading a byte from that project's disk**.
//!
//! **`unknown` is stored, rendered and scored as unknown** — never as clean, never as a zero debt
//! count, never as a dark tick. The trap, stated because it is the inverted form of the invariant:
//! a health reading over *how many debt items are open* reads an unknown source as **zero open
//! items**, which is a silently *better* score than the truth. An unknown verdict removes this
//! source from the reading's inputs entirely, numerator and denominator both. The arithmetic is
//! §30's; the input state is this module's.

use rusqlite::Connection;

use crate::index::IndexError;
use crate::protocol::{DependencyVerdict, ProjectId};

/// How long a `clean` verdict stands before it expires to `unknown`.
///
/// **This is this section's own constant and is not §6's staleness-marker threshold** (A16.4).
/// §6's is *the age at which a marker appears beside a value*; this is *the age at which a clean
/// claim stops being made*. `WORKTREE_STALE_AFTER_SECS` stays at exactly one site, in the
/// renderer, and nothing here reads it.
///
/// **Thirty days because the sweep's cadence is daily**: a clean verdict aged past thirty days
/// means roughly thirty consecutive attempts failed to reach the source, not that one request did.
pub const CLEAN_VERDICT_EXPIRY_SECS: i64 = 30 * 24 * 60 * 60;

/// A verdict and the clock it was computed at.
///
/// **A Rust shape, not a wire type**: §32's type delta is +5 and this is not one of them. What
/// crosses the wire is the verdict and the timestamp, as two fields on `ProjectDetail`.
///
/// `observed_at` is the **older** of the two clocks — see [`verdict_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DependencyReading {
    /// §32.8's verdict for the project.
    pub verdict: DependencyVerdict,
    /// When the facts behind the verdict were observed, in unix seconds. `None` only when no
    /// `project_dependency_scan` row was read.
    pub observed_at: Option<i64>,
}

/// One project's verdict, recomputed from the join.
///
/// **It opens no file and spawns no child**, so an uninstalled, offline, unmounted or missing
/// project still gets one — carrying the age of the facts it was computed from, which is what
/// §32.14's four non-installed states need: they are **frozen** at the last computed state with
/// the time it was computed, and a reading that was never computed is *absent* rather than frozen.
///
/// `now` is a parameter so the expiry is clock-driven and never `SystemTime::now`.
///
/// **The composite clock is the OLDER of the two** — `advisory_triple.observed_at` and
/// `project_dependency.observed_at`. A sweep that ran an hour ago, joined against a triple set
/// read six months ago, otherwise produces a fresh-looking answer about a lockfile nobody has
/// looked at since. Neither §6 nor §21 has a rule for a composite, so §32.9 rules this one.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn verdict_for(
    conn: &Connection,
    project: ProjectId,
    now: i64,
) -> Result<DependencyReading, IndexError> {
    // *The scan has not run* is the **absence** of this row, and it is `unknown`. Collapsing it
    // into *ran and found nothing* makes every unscanned project claim to have no dependencies.
    let scan: Option<(i64, i64, i64, i64)> = conn
        .query_row(
            "SELECT observed_at, files_matched, unresolved_manifests, complete
               FROM project_dependency_scan WHERE project_id = ?1",
            [project.0],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .ok();
    let Some((scan_at, files_matched, unresolved_manifests, complete)) = scan else {
        return Ok(unknown(None));
    };

    // Any lockfile the read could not take is `unknown`, whatever the others said: a cap
    // exceedance, an unreadable file and a construct the parser did not understand all produce a
    // **short** triple set, and a short set is a false clean.
    let not_read: i64 = conn.query_row(
        "SELECT count(*) FROM project_lockfile WHERE project_id = ?1 AND read_state = 'notRead'",
        [project.0],
        |row| row.get(0),
    )?;
    if not_read > 0 || complete == 0 || unresolved_manifests > 0 {
        return Ok(unknown(Some(scan_at)));
    }

    // Ran, found no lockfile of any ecosystem this build parses, and declared no manifest either:
    // there is nothing to be vulnerable through. **This is the only row `clean` is lit on
    // without a single triple**, and it is lit only because *ran and found nothing* is
    // distinguishable from *has not run*.
    if files_matched == 0 {
        return Ok(DependencyReading {
            verdict: DependencyVerdict::Clean,
            observed_at: Some(scan_at),
        });
    }

    let triples_at: Option<i64> = conn
        .query_row(
            "SELECT min(observed_at) FROM project_dependency WHERE project_id = ?1",
            [project.0],
            |row| row.get(0),
        )
        .unwrap_or(None);
    // The read matched files but wrote no triple: every lockfile parsed to nothing. That is an
    // answer — an empty dependency graph — and its clock is the scan's.
    let dependency_at = triples_at.unwrap_or(scan_at);

    // **Any triple this sweep has not answered makes the whole verdict unknown.** A project is
    // vulnerable through the dependency nobody asked about exactly as easily as through the one
    // that was asked about, and a partial answer rendered as `clean` is the section's own defect.
    let unanswered: i64 = conn.query_row(
        "SELECT count(*) FROM project_dependency d
          WHERE d.project_id = ?1
            AND NOT EXISTS (
                  SELECT 1 FROM advisory_triple t
                   WHERE t.ecosystem = d.ecosystem
                     AND t.package_name = d.package_name
                     AND t.version = d.version
                     AND t.answered = 1)",
        [project.0],
        |row| row.get(0),
    )?;
    if unanswered > 0 {
        return Ok(unknown(Some(dependency_at)));
    }

    let sweep_at: Option<i64> = conn
        .query_row(
            "SELECT min(t.observed_at) FROM advisory_triple t
               JOIN project_dependency d
                 ON d.ecosystem = t.ecosystem
                AND d.package_name = t.package_name
                AND d.version = t.version
              WHERE d.project_id = ?1",
            [project.0],
            |row| row.get(0),
        )
        .unwrap_or(None);
    let composite = sweep_at.map_or(dependency_at, |at| at.min(dependency_at));

    let matches: i64 = conn.query_row(
        "SELECT count(*) FROM advisory_match m
           JOIN project_dependency d
             ON d.ecosystem = m.ecosystem
            AND d.package_name = m.package_name
            AND d.version = m.version
           JOIN advisory a ON a.advisory_id = m.advisory_id
          WHERE d.project_id = ?1 AND a.withdrawn_at IS NULL",
        [project.0],
        |row| row.get(0),
    )?;

    if matches > 0 {
        // **`vulnerable` never expires.** It asserts *this project resolves package P at version
        // V, and published advisory A says V is affected* — both halves recorded facts, neither
        // refuted by time passing. Expiring it would silently unopen real debt.
        return Ok(DependencyReading {
            verdict: DependencyVerdict::Vulnerable,
            observed_at: Some(composite),
        });
    }

    // **`clean` expires at thirty days.** It asserts *no published advisory matches any of this
    // project's triples* — a claim about a database that gains entries daily, which time refutes
    // on its own.
    if now.saturating_sub(composite) >= CLEAN_VERDICT_EXPIRY_SECS {
        return Ok(unknown(Some(composite)));
    }
    Ok(DependencyReading {
        verdict: DependencyVerdict::Clean,
        observed_at: Some(composite),
    })
}

const fn unknown(observed_at: Option<i64>) -> DependencyReading {
    DependencyReading {
        verdict: DependencyVerdict::Unknown,
        observed_at,
    }
}
