//! §28.1's `todo_marker` builder, over §29's occurrences and §29's sweep state.
//!
//! **`ordinal` is per PROJECT and is not `BlobOccurrence.ordinal_in_blob`** (A10). It is the
//! 0-based index among the project's occurrences carrying the same `salient_sha256`, ordered by
//! `(path_bytes, line, column)` over the whole HEAD enumeration — **so a blob reachable at two
//! paths contributes its occurrences twice.** The two ordinals are never conflated: one is a
//! position inside a content-addressed blob shared library-wide, the other is a position inside
//! one project. This is the blob/project boundary R134 names, at the one place it decides an
//! identity.
//!
//! **Two consequences, accepted rather than hidden.** *Ordinal churn* — delete the first of two
//! identical markers and the survivor renumbers to ordinal 0, so the closure attributes to the
//! twin; the count is right and the payout is right, and a positional matcher buys nothing a user
//! can see. *Rewording pays* — an edited marker closes one item and opens another; §28.4's
//! per-day key bounds that to one payout a day.

use std::collections::HashMap;

use rusqlite::Transaction;

use super::identity::DebtKey;
use super::store::{DebtStore, ObservedItem, SweepEffect};
use super::sweep::{outcome_at_root, SweepObservation};
use super::{registry_for, DebtError};
use crate::jobs::j7_markers::{
    content_sweep_state, ContentGates, ContentOccurrence, ContentSweepOutcome, ContentSweepState,
};
use crate::protocol::{DebtSource, DebtSweepOutcome, LocationId, ObservationBasis, ProjectId};

/// §28.1's per-project ordinal, parallel to its input so an occurrence and its ordinal cannot
/// drift.
///
/// **Never `ordinal_in_blob`.** p3-29 hands the occurrences already ordered on
/// `(path_bytes, line, column)` and assigns no ordinal, because the per-project ordinal is
/// item-build-time material and item-build time is §28's phase.
#[must_use]
pub fn project_ordinals(occurrences: &[ContentOccurrence]) -> Vec<i64> {
    let mut counts: HashMap<&str, i64> = HashMap::new();
    let mut out = Vec::with_capacity(occurrences.len());
    for occurrence in occurrences {
        let next = counts
            .entry(occurrence.salient_sha256.as_str())
            .or_insert(0);
        out.push(*next);
        *next = next.saturating_add(1);
    }
    out
}

/// The one place §29's `&'static str` basis and its outcome are mapped onto the generated
/// [`ObservationBasis`] and [`DebtSweepOutcome`] §28 owns.
///
/// **`item_count` is `Some` only for `complete` and `partial`**, which is also what the DDL's
/// honesty CHECK enforces. §29's two gates produce `skipped_reference` and, now that p3-30 has
/// replaced `ContentGates.compute_suppressed` with the real predicate, `skipped_suppressed`.
///
/// **This read of the gate records the skip; it does not gate anything** (A11.2). The gate itself
/// is `ContentGates::reads_blobs`, and it stops the blob read alone.
#[must_use]
pub fn sweep_from_content(
    state: &ContentSweepState,
    gates: ContentGates,
    location: Option<LocationId>,
    item_count: Option<u32>,
    project: ProjectId,
    now: i64,
) -> SweepObservation {
    let outcome = if gates.runs() {
        if gates.compute_suppressed {
            DebtSweepOutcome::SkippedSuppressed
        } else {
            match state.outcome {
                ContentSweepOutcome::Complete => DebtSweepOutcome::Complete,
                ContentSweepOutcome::Partial => DebtSweepOutcome::Partial,
            }
        }
    } else {
        DebtSweepOutcome::SkippedReference
    };
    let observes = matches!(
        outcome,
        DebtSweepOutcome::Complete | DebtSweepOutcome::Partial
    );
    SweepObservation {
        project,
        source: DebtSource::TodoMarker,
        outcome,
        location,
        generation: None,
        basis: Some(basis_of(state.basis)),
        item_count: if observes { item_count } else { None },
        observed_at: now,
    }
}

/// §29 states its basis as a literal rather than storing it; this is where that string meets the
/// generated vocabulary. An unknown spelling falls back to `head`, which is the only basis J7
/// has — a `None` here would silently make every item incomparable and close nothing for ever.
fn basis_of(raw: &str) -> ObservationBasis {
    match raw {
        "index" => ObservationBasis::Index,
        "worktree" => ObservationBasis::Worktree,
        "refs" => ObservationBasis::Refs,
        "remote" => ObservationBasis::Remote,
        _ => ObservationBasis::Head,
    }
}

/// Turn §29's occurrences into items this project owns, and write the sweep that observed them.
///
/// **`occurrences` is a parameter and not a read, and that is forced by the tree rather than
/// chosen**: `occurrences_for_project` takes the HEAD enumeration, because nothing stores it —
/// §29.6 re-runs `ls-tree` per chunk deliberately. Only a caller inside a J7 run holds it.
///
/// **Returns [`SweepEffect::default`] and writes nothing when `content_sweep_state` is `None`**:
/// that is *never observed*, it is not an outcome, and a project with no `debt_sweep` row renders
/// as *not computed* rather than as zero.
pub fn build_items(
    tx: &Transaction<'_>,
    project: ProjectId,
    location: Option<LocationId>,
    gates: ContentGates,
    occurrences: &[ContentOccurrence],
    now: i64,
    store: &dyn DebtStore,
) -> Result<SweepEffect, DebtError> {
    let Some(state) = content_sweep_state(tx, project)? else {
        return Ok(SweepEffect::default());
    };

    let subject_key = crate::index::subject::subject_for_project(tx, project)?
        .map(|s| s.to_key())
        .unwrap_or_default();
    let ordinals = project_ordinals(occurrences);
    let row = registry_for(DebtSource::TodoMarker);

    let mut seen = Vec::with_capacity(occurrences.len());
    for (occurrence, ordinal) in occurrences.iter().zip(&ordinals) {
        seen.push(ObservedItem {
            key: DebtKey::content(&subject_key, &occurrence.salient_sha256, *ordinal),
            scoring: row.default_scoring,
            location,
            basis: row.basis,
            path_bytes: Some(occurrence.path_bytes.clone()),
            path_display: Some(String::from_utf8_lossy(&occurrence.path_bytes).into_owned()),
            line: Some(occurrence.line),
            column: Some(occurrence.column),
            salient_text: Some(occurrence.salient_text_capped.clone()),
        });
    }

    let count = u32::try_from(seen.len()).unwrap_or(u32::MAX);
    let mut obs = sweep_from_content(&state, gates, location, Some(count), project, now);
    // Rule 3, applied before the diff and never after: a sweep of a root that is not there is not
    // a sweep with zero results.
    obs.outcome = outcome_at_root(tx, location, obs.outcome)?;
    if !matches!(
        obs.outcome,
        DebtSweepOutcome::Complete | DebtSweepOutcome::Partial
    ) {
        obs.item_count = None;
    }

    store.observe(tx, &obs, &seen)
}
