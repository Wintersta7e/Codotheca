//! §32.10's items: **shown is wider than scored, and a withdrawal closes without paying.**
//!
//! Every row here goes through §28's item writer. This module declares no second one.
//!
//! **The item's identity is `(subject_key, 'dependency_advisory', '<ecosystem>:<package>:<id>')`**
//! (A5). `subject_key` and not `project_id`, because §1.7 records that v1 keyed the ledger on
//! `project_id` and it broke on merges. **The version is deliberately not in the key**: a bump
//! from one vulnerable version to another must not close and reopen an identically-meaning item
//! and pay XP twice. The `advisory_id` is the GHSA id, never the CVE id.

use rusqlite::{Connection, OptionalExtension, Transaction};

use crate::advisories::verdict::verdict_for;
use crate::advisories::{eco_slug, AdvisoryError};
use crate::debt::identity::DebtKey;
use crate::debt::store::{DebtCloseReason, DebtStore, ObservedItem, SqliteDebtStore, SweepEffect};
use crate::debt::sweep::SweepObservation;
use crate::protocol::{
    AdvisoryDetail, DebtScoring, DebtSource, DebtSweepOutcome, DependencyVerdict, Ecosystem,
    HealthDetectedIn, LocationId, ObservationBasis, ProjectHealthDelta, ProjectId,
};

/// What one item sweep changed. The counts a test prints; **stored nowhere**.
///
/// `effect` is §28's own [`SweepEffect`] **with the withdrawals reclassified**, and it is what the
/// XP writer must be paid from. §28's `observe` closes every unobserved item `Fixed`, because
/// every source it sweeps is evidence the user controls — true for the eight other sources and
/// **false for this one**. §32 is the only producer that can tell a withdrawal from an upgrade, so
/// the correction is made here, at the one place that knows, rather than left to a caller to
/// remember.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdvisoryItemSweep {
    pub opened: usize,
    pub closed_fixed: usize,
    pub closed_invalidated: usize,
    pub unverified: usize,
    pub effect: SweepEffect,
}

/// The item fingerprint for one `(ecosystem, package, advisory)`.
#[must_use]
pub fn advisory_fingerprint(eco: Ecosystem, package: &str, advisory_id: &str) -> String {
    format!("{}:{package}:{advisory_id}", eco_slug(eco))
}

/// One advisory match, as this module reads it back.
struct Match {
    ecosystem: Ecosystem,
    package: String,
    advisory_id: String,
    fix_available: bool,
    withdrawn: bool,
}

fn matches_for(conn: &Connection, project: ProjectId) -> Result<Vec<Match>, AdvisoryError> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT m.ecosystem, m.package_name, m.advisory_id,
                    max(m.fix_available), a.withdrawn_at IS NOT NULL
               FROM advisory_match m
               JOIN project_dependency d
                 ON d.ecosystem = m.ecosystem
                AND d.package_name = m.package_name
                AND d.version = m.version
               JOIN advisory a ON a.advisory_id = m.advisory_id
              WHERE d.project_id = ?1
              GROUP BY m.ecosystem, m.package_name, m.advisory_id
              ORDER BY m.ecosystem, m.package_name, m.advisory_id",
        )
        .map_err(crate::index::IndexError::from)?;
    let rows = stmt
        .query_map([project.0], |row| {
            let raw: String = row.get(0)?;
            Ok((
                raw,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)? != 0,
                row.get::<_, i64>(4)? != 0,
            ))
        })
        .map_err(crate::index::IndexError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(crate::index::IndexError::from)?;
    Ok(rows
        .into_iter()
        .filter_map(|(raw, package, advisory_id, fix_available, withdrawn)| {
            Ecosystem::ALL
                .into_iter()
                .find(|e| eco_slug(*e) == raw)
                .map(|ecosystem| Match {
                    ecosystem,
                    package,
                    advisory_id,
                    fix_available,
                    withdrawn,
                })
        })
        .collect())
}

/// Open, refresh and close this project's `dependency_advisory` items.
///
/// **An advisory with no fix available is visible debt that scores nothing** (A7's `scoring`
/// column, orthogonal to the item's state and not a fourth state). It renders, it lights the rust
/// layer, it ranks nowhere, and it is excluded from the health count and from any XP payout —
/// because the two wordings in the concept do not agree and **the narrower one is right**: the
/// ninth check is *"no known vulnerable dependencies with a fix available"*, while the unqualified
/// form is unachievable, a package's *latest* release having still matched four advisories.
///
/// **A withdrawn advisory closes its item as `invalidated`, never as `fixed`.** It pays no XP, and
/// XP already paid is never clawed back. A single `closed` state would pay the user for a fix they
/// did not perform and put an unearned event in an append-only ledger with a monotonic
/// `level_floor`.
///
/// **An item whose evidence cannot be re-read is `unverified`, not closed.** A lockfile the app is
/// simply unable to re-read — uninstalled, offline, unmounted — is **not evidence of a fix**.
/// Without this rule every uninstall silently pays out the project's whole open debt list.
///
/// # Errors
/// Fails when the index refuses a write.
pub fn sync_advisory_items(
    tx: &Transaction<'_>,
    project: ProjectId,
    location: Option<LocationId>,
    now: i64,
    store: &dyn DebtStore,
) -> Result<AdvisoryItemSweep, AdvisoryError> {
    // A project with neither a lineage key nor a remote key has no subject to key a ledger on,
    // and §1.7 records what keying it on `project_id` did instead: the ledger broke on merges.
    let Some(subject) =
        crate::index::subject::subject_for_project(tx, project)?.map(|s| s.to_key())
    else {
        return Ok(AdvisoryItemSweep::default());
    };
    let reading = verdict_for(tx, project, now)?;

    // **An uninstalled copy's empty walk is not an observation of an empty dependency set.**
    // This is the hole that costs the most and it is not closed by the verdict alone: a project
    // whose working copy is gone walks cleanly, finds no lockfile and no manifest, and reads
    // `clean` — at which point the diff would close every open item as `fixed` and the day's
    // payout would pay the user for an uninstall. *Unreadable is not evidence of a fix.*
    //
    // §24.6a's predicate: at least one location with `removed_at IS NULL` **and**
    // `presence = 'present'`. `removed_at` takes precedence over `presence` on every surface.
    let installed: bool = tx
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM location
                             WHERE project_id = ?1
                               AND removed_at IS NULL
                               AND presence = 'present')",
            [project.0],
            |row| row.get::<_, i64>(0),
        )
        .map_err(crate::index::IndexError::from)?
        != 0;

    // **Unknown is not an observation of an empty set either.** The verdict is `unknown` exactly
    // when the evidence could not be re-read or was never asked about, so the items are marked
    // `unverified` and **none is closed**.
    //
    // Through `observe`, as every other producer records an unobservable sweep: §28's diff marks
    // **this source's** open items `unverified` and closes nothing. A whole-project mark would
    // take the project's TODO and README items out of every count for a read that never looked
    // at them.
    if !installed || reading.verdict == DependencyVerdict::Unknown {
        let observation = SweepObservation {
            project,
            source: DebtSource::DependencyAdvisory,
            outcome: DebtSweepOutcome::Unobservable,
            location,
            generation: None,
            basis: Some(ObservationBasis::Worktree),
            // `None`: an unobservable sweep counted nothing, and a zero here would read as
            // *nobody has any advisory debt*.
            item_count: None,
            observed_at: now,
        };
        let effect = store.observe(tx, &observation, &[])?;
        return Ok(AdvisoryItemSweep {
            unverified: effect.unverified as usize,
            effect,
            ..AdvisoryItemSweep::default()
        });
    }

    let found = matches_for(tx, project)?;
    // **A withdrawn advisory is not observed**, so the diff closes its item — and it closes it
    // `invalidated`, which the store learns from the withdrawal list rather than from the absence.
    let withdrawn: Vec<DebtKey> = found
        .iter()
        .filter(|m| m.withdrawn)
        .map(|m| DebtKey::external(&subject, eco_slug(m.ecosystem), &m.package, &m.advisory_id))
        .collect();

    let seen: Vec<ObservedItem> = found
        .iter()
        .filter(|m| !m.withdrawn)
        .map(|m| ObservedItem {
            key: DebtKey::external(&subject, eco_slug(m.ecosystem), &m.package, &m.advisory_id),
            // **The one place `scoring` is overridden per item**, and the whole of A7's use here.
            scoring: if m.fix_available {
                DebtScoring::Scored
            } else {
                DebtScoring::ShownOnly
            },
            location,
            basis: Some(ObservationBasis::Worktree),
            path_bytes: None,
            path_display: None,
            line: None,
            column: None,
            salient_text: None,
        })
        .collect();

    let observation = SweepObservation {
        project,
        source: DebtSource::DependencyAdvisory,
        outcome: DebtSweepOutcome::Complete,
        location,
        generation: None,
        // **Worktree**, stated once, here. It travels on the sweep row §28 owns and is never a
        // column on `project_dependency`, whose value would not vary.
        basis: Some(ObservationBasis::Worktree),
        item_count: u32::try_from(seen.len()).ok(),
        observed_at: now,
    };
    let mut effect = store.observe(tx, &observation, &seen)?;

    // **The reclassification, and it is not cosmetic.** `pay_debt_day` pays exactly the closes
    // marked `Fixed`; leaving a withdrawal marked that way puts an unearned event in an
    // append-only ledger with a monotonic `level_floor`, for a fix the user did not perform.
    for (key, reason) in &mut effect.closed {
        if withdrawn.contains(key) {
            *reason = DebtCloseReason::Invalidated;
        }
    }

    let mut result = AdvisoryItemSweep {
        opened: effect.opened.len(),
        unverified: effect.unverified as usize,
        ..AdvisoryItemSweep::default()
    };
    for (_, reason) in &effect.closed {
        match reason {
            DebtCloseReason::Invalidated => result.closed_invalidated += 1,
            DebtCloseReason::Fixed => result.closed_fixed += 1,
        }
    }
    result.effect = effect;
    Ok(result)
}

/// Sync every scanned project's items and pay its closes, at the close of a sweep that has asked
/// about every triple. Returns the health deltas to announce **once the caller's transaction has
/// committed**.
///
/// **Before the settle, never at its alert**: §31's `deps` check reads these items, and the
/// settle runs its completion evaluator before the alert, so a sync placed after it would answer
/// every `deps` check from the previous sweep.
///
/// **Unanchored** (`location: None`): the triples are a fact about the project, not about the
/// copy they were read from, so an item is not stranded as `unverified` when the primary moves.
///
/// Each project's closes are paid in the same transaction as the deletions, as every other
/// producer's are; `pay_debt_day` excludes the `invalidated` ones. **Each project's write is
/// wrapped by §34's producer** (A15: a delta from every debt-set transition), with the closes
/// passed so a decrease that is only withdrawals is written and not announced (R135). A scheduled
/// sweep observes in the `background`.
///
/// # Errors
/// Fails when the index refuses a read or a write.
pub fn settle_advisory_items(
    tx: &Transaction<'_>,
    now: i64,
    tz_offset_min: i32,
) -> Result<Vec<ProjectHealthDelta>, AdvisoryError> {
    // §32.12 rule 3: a project's first computation is seeded before the sync marks it computed,
    // so no later settle can decide on a pair it never seeded.
    crate::advisories::notify::seed_first_computations(tx, now)?;
    let projects: Vec<i64> = {
        let mut stmt = tx
            .prepare("SELECT project_id FROM project_dependency_scan ORDER BY project_id")
            .map_err(crate::index::IndexError::from)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(crate::index::IndexError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::index::IndexError::from)?;
        rows
    };
    let mut deltas = Vec::new();
    for id in &projects {
        let project = ProjectId(*id);
        // A snapshot that cannot be read records no delta and costs the item write nothing, as
        // at the producer's other callers.
        let before = crate::restoration::LayerValues::read(tx, project).ok();
        let swept = sync_advisory_items(tx, project, None, now, &SqliteDebtStore)?;
        let subject = crate::index::subject::subject_for_project(tx, project)?
            .map(|s| s.to_key())
            .unwrap_or_default();
        crate::debt::xp::pay_debt_day(tx, project, &subject, &swept.effect, now, tz_offset_min)?;
        if let Some(before) = before {
            if let Some(delta) = crate::restoration::record_after_write(
                tx,
                project,
                &before,
                &swept.effect.closed,
                HealthDetectedIn::Background,
                now,
            )? {
                deltas.push(delta);
            }
        }
    }
    Ok(deltas)
}

/// Fill `DebtItem.advisory` for one item, **including every CVE id**.
///
/// R118: one nullable struct, not six nullable fields NULL for eight of the nine sources. `None`
/// for a fingerprint no advisory backs, which is every item of every other source.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn advisory_detail_for(
    conn: &Connection,
    fingerprint: &str,
) -> Result<Option<AdvisoryDetail>, AdvisoryError> {
    let mut parts = fingerprint.splitn(3, ':');
    let (Some(eco_raw), Some(package), Some(advisory_id)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return Ok(None);
    };
    let Some(ecosystem) = Ecosystem::ALL.into_iter().find(|e| eco_slug(*e) == eco_raw) else {
        return Ok(None);
    };

    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT a.severity, (SELECT m.fixed_version FROM advisory_match m
                                  WHERE m.advisory_id = a.advisory_id
                                    AND m.package_name = ?2
                                    AND m.fixed_version IS NOT NULL
                                  LIMIT 1)
               FROM advisory a WHERE a.advisory_id = ?1",
            rusqlite::params![advisory_id, package],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        // `optional`, not `ok`: no advisory row is *no detail*, and an index fault is an error —
        // read as the first, it would show an advisory item with its severity silently missing.
        .optional()
        .map_err(crate::index::IndexError::from)?;
    let Some((severity, fixed_version)) = row else {
        return Ok(None);
    };

    let mut stmt = conn
        .prepare("SELECT cve_id FROM advisory_cve WHERE advisory_id = ?1 ORDER BY cve_id")
        .map_err(crate::index::IndexError::from)?;
    let cve_ids: Vec<String> = stmt
        .query_map([advisory_id], |r| r.get(0))
        .map_err(crate::index::IndexError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(crate::index::IndexError::from)?;

    Ok(Some(AdvisoryDetail {
        ecosystem,
        package_name: package.to_owned(),
        advisory_id: advisory_id.to_owned(),
        cve_ids,
        severity,
        fixed_version,
    }))
}
