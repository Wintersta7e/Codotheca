//! §32.12's **one** notification: five conjuncts, a seeding ledger, and no token naming an absence.
//!
//! It is one of exactly three the product may ever fire — the Sunday one-liner, *a critical
//! advisory in a project you have installed*, and *your wrap is ready* — and it is the **first
//! that may fire at all**. The footer `THREE NOTIFICATIONS EXIST · NONE MENTIONS ABSENCE` states a
//! **ceiling**, and phase 3 honours it by firing one of the three and not a fourth.
//!
//! **It never states why it did not fire.** Every reason for not firing is an absence, and no
//! notification mentions absence. A line reading *"3 critical advisories, 1 in an uninstalled
//! project"* breaches the contract. Excluded projects are excluded **silently**; their debt items
//! still exist on their own pages, dated, which is where absence is allowed to be visible.
//!
//! **The shell posts it; the renderer never does.** The renderer's `notifications` permission
//! stays denied — the same invariant as *the renderer may never originate a filesystem path or an
//! executable*, applied to the one outward-facing interruption the product has.

use rusqlite::Transaction;

use crate::advisories::{eco_slug, AdvisoryError};
use crate::index::IndexError;
use crate::protocol::{AdvisoryAlert, Ecosystem, ProjectId};

/// The one severity that may interrupt, **compared against the source's own word and never
/// re-scored** (§32.13). A build that mapped the vocabulary to its own scale would decide for the
/// source which advisories are critical.
pub const NOTIFIABLE_SEVERITY: &str = "critical";

/// One candidate: a project, an advisory, and what the copy would name.
struct Candidate {
    project: ProjectId,
    advisory_id: String,
    cve_id: Option<String>,
    package_name: String,
    ecosystem: Ecosystem,
}

/// Every `(project, advisory)` pair that satisfies the four conjuncts this module can evaluate.
///
/// The fifth — **not `surface_suppressed`** — is §30's and is supplied by the caller. A11.2 rules
/// that suppression gates rendering, ranking **and notification**, and §30 owns the predicate;
/// reimplementing it here would be a second one that could disagree with the first, so it arrives
/// as a parameter. **Until §30 lands, the caller passes a predicate that suppresses nothing**,
/// which is the honest state of a build that declares no suppression.
fn candidates(tx: &Transaction<'_>, now: i64) -> Result<Vec<Candidate>, AdvisoryError> {
    let _ = now;
    let mut stmt = tx
        .prepare(
            "SELECT DISTINCT d.project_id, a.advisory_id, m.ecosystem, m.package_name,
                    (SELECT c.cve_id FROM advisory_cve c
                      WHERE c.advisory_id = a.advisory_id ORDER BY c.cve_id LIMIT 1)
               FROM advisory_match m
               JOIN advisory a ON a.advisory_id = m.advisory_id
               JOIN project_dependency d
                 ON d.ecosystem = m.ecosystem
                AND d.package_name = m.package_name
                AND d.version = m.version
              WHERE a.severity = ?1
                AND a.withdrawn_at IS NULL
                AND m.fix_available = 1
                AND EXISTS (SELECT 1 FROM location l
                             WHERE l.project_id = d.project_id
                               AND l.removed_at IS NULL
                               AND l.presence = 'present')
                AND NOT EXISTS (SELECT 1 FROM advisory_notified n
                                 WHERE n.project_id = d.project_id
                                   AND n.advisory_id = a.advisory_id)
              ORDER BY d.project_id, a.advisory_id",
        )
        .map_err(IndexError::from)?;
    let rows = stmt
        .query_map([NOTIFIABLE_SEVERITY], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(IndexError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(IndexError::from)?;
    Ok(rows
        .into_iter()
        .filter_map(|(project, advisory_id, eco_raw, package_name, cve_id)| {
            Ecosystem::ALL
                .into_iter()
                .find(|e| eco_slug(*e) == eco_raw)
                .map(|ecosystem| Candidate {
                    project: ProjectId(project),
                    advisory_id,
                    cve_id,
                    package_name,
                    ecosystem,
                })
        })
        .collect())
}

/// A project's **first** computation writes `seeded = 1` rows and fires **nothing**.
///
/// Without it, first run toasts once per critical advisory across the whole library — the guilt
/// firehose in its purest form, on the one surface a user cannot dismiss before reading.
///
/// Returns the number of pairs seeded.
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn seed_notified(
    tx: &Transaction<'_>,
    project: ProjectId,
    now: i64,
) -> Result<usize, AdvisoryError> {
    let mut seeded = 0usize;
    for candidate in candidates(tx, now)? {
        if candidate.project != project {
            continue;
        }
        seeded += tx
            .execute(
                "INSERT INTO advisory_notified (project_id, advisory_id, at, seeded)
                 VALUES (?1, ?2, ?3, 1)
                 ON CONFLICT(project_id, advisory_id) DO NOTHING",
                rusqlite::params![project.0, candidate.advisory_id, now],
            )
            .map_err(IndexError::from)?;
    }
    Ok(seeded)
}

/// **At most one payload per settle**, naming the project when there is one and the project count
/// when there are several.
///
/// It writes the `advisory_notified` rows it consumes **in the same transaction**, so a crash
/// between deciding and recording cannot re-fire.
///
/// `suppressed` is §30's predicate, passed in rather than reimplemented — see [`candidates`].
///
/// # Errors
/// Fails when SQLite refuses the write.
pub fn notifiable(
    tx: &Transaction<'_>,
    now: i64,
    suppressed: &dyn Fn(ProjectId) -> bool,
) -> Result<Option<AdvisoryAlert>, AdvisoryError> {
    let found: Vec<Candidate> = candidates(tx, now)?
        .into_iter()
        .filter(|c| !suppressed(c.project))
        .collect();
    if found.is_empty() {
        return Ok(None);
    }

    // Consumed whether or not each one is named in the copy: the ledger is *this pair has been
    // told about*, not *this pair was in a sentence*.
    for candidate in &found {
        tx.execute(
            "INSERT INTO advisory_notified (project_id, advisory_id, at, seeded)
             VALUES (?1, ?2, ?3, 0)
             ON CONFLICT(project_id, advisory_id) DO NOTHING",
            rusqlite::params![candidate.project.0, candidate.advisory_id, now],
        )
        .map_err(IndexError::from)?;
    }

    let mut projects: Vec<i64> = found.iter().map(|c| c.project.0).collect();
    projects.sort_unstable();
    projects.dedup();
    let mut advisories: Vec<&str> = found.iter().map(|c| c.advisory_id.as_str()).collect();
    advisories.sort_unstable();
    advisories.dedup();

    let project_count = i64::try_from(projects.len()).unwrap_or(i64::MAX);
    let advisory_count = i64::try_from(advisories.len()).unwrap_or(i64::MAX);
    // **The five nullable fields are `Some` only when exactly one project and one advisory
    // fired.** With several, naming one of them would be a sentence about the others' absence.
    let single = (projects.len() == 1 && advisories.len() == 1)
        .then(|| found.first())
        .flatten();

    Ok(Some(AdvisoryAlert {
        project_id: single.map(|c| c.project),
        project_count,
        advisory_id: single.map(|c| c.advisory_id.clone()),
        // **The notifiable unit is the advisory, not the CVE.** One advisory carries several CVE
        // ids or none — unreviewed and malware advisories have none — so *"a critical CVE"* taken
        // literally would make a critical advisory with no CVE id un-notifiable, which is a rule
        // about identifier bookkeeping wearing the costume of a rule about severity.
        cve_id: single.and_then(|c| c.cve_id.clone()),
        package_name: single.map(|c| c.package_name.clone()),
        ecosystem: single.map(|c| c.ecosystem),
        advisory_count,
    }))
}

/// A predicate that suppresses nothing, for a build that declares no suppression.
///
/// Named rather than written as a closure at each call site, so the day §30 lands there is one
/// place to look for what has to change.
pub fn nothing_suppressed() -> impl Fn(ProjectId) -> bool {
    |_| false
}
