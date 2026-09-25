//! §28.2's singleton item evaluator — **R124**.
//!
//! Of A4's nine sources only `todo_marker` and `dependency_advisory` had a producer that accepted
//! the assignment: §29's handover gives §28 markers and a sweep row and routes the tri-states to
//! §31, and §31 declines outright. **The failure this avoids is silent**: six of nine sources
//! built nowhere, health reading `unknown` for them for ever, and every section's own criteria
//! green.
//!
//! **§28.2a is the rule this module exists to hold: a producer must distinguish *absent* from
//! *unreadable* before it may OPEN an item.** `missing_readme` is the case that proves it. J6
//! sets `readme_path` when a candidate file exists and then assigns the excerpt from a read that
//! returns `None` on **any** open or read failure; the cached row is then written with a NULL
//! excerpt whose convention is *"no README in this repository"*. **A debt producer reading that
//! would open `missing_readme` on a repository that has one.** `readme` therefore reads J7's
//! HEAD-basis path predicate, where presence is decided by the path existing and **no file is
//! opened at all** (A13.1). `core/tests/debt_producers.rs` holds a gate asserting this module
//! reads that cache nowhere.
//!
//! **A singleton source opens an item only on a positive observation that its predicate is
//! false.** An input that was never observed opens nothing **and marks nothing**, and the sweep
//! for that source records `unobservable`. Writing the item anyway is *never render unknown as
//! zero* in its own words.

use rusqlite::{OptionalExtension as _, Transaction};

use super::identity::DebtKey;
use super::store::{DebtStore, ObservedItem, SweepEffect};
use super::sweep::{outcome_at_root, SweepObservation};
use super::{registry_for, DebtError, SourceRow};
use crate::jobs::j7_markers::presence_for_project;
use crate::jobs::presence::PresenceState;
use crate::projects::rows::{locations_of, pick_primary};
use crate::protocol::{DebtSource, DebtSweepOutcome, LocationId, ProjectId};

/// The three-way answer that makes *absent* and *unreadable* unsayable as one value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArmReading {
    /// A positive observation that the predicate is false: open an item.
    PredicateFalse(ObservedItem),
    /// A positive observation that the predicate holds: close any item this source has.
    PredicateTrue,
    /// **Never observed.** Opens nothing, marks nothing, and the sweep records `unobservable`.
    Unobservable,
}

/// One source's predicate.
///
/// Declared **with all its production implementations in the same change** — R1, and five
/// recorded instances of the opposite, each of which compiled and passed against a fake.
pub trait SingletonArm: Sync {
    /// The one singleton source this arm decides, which picks its registry row and sweep row.
    fn source(&self) -> DebtSource;

    /// # Errors
    /// Fails when SQLite refuses a read.
    fn observe(&self, tx: &Transaction<'_>, project: ProjectId) -> Result<ArmReading, DebtError>;
}

/// The project's primary copy, or `None` when it has none.
fn primary_of(tx: &Transaction<'_>, project: ProjectId) -> Result<Option<LocationId>, DebtError> {
    let locations =
        locations_of(tx, project).map_err(|e| DebtError::Codec(format!("locations_of: {e}")))?;
    Ok(pick_primary(&locations).map(|l| l.id))
}

/// The anchor a source's sweep and its item both carry.
///
/// **One owner.** A `NULL` basis has exactly one meaning and exactly one source —
/// `abandoned_with_debt` observes nothing itself — so a source with no basis has no anchor
/// either, and every other one anchors at the primary copy. An arm that computed its own could
/// hand `may_close` a pair that never compares, and the item would never close.
fn anchor_for(
    tx: &Transaction<'_>,
    project: ProjectId,
    row: SourceRow,
) -> Result<Option<LocationId>, DebtError> {
    if row.basis.is_none() {
        return Ok(None);
    }
    primary_of(tx, project)
}

/// The item an arm opens, with its anchor, basis and scoring taken from the registry rather than
/// from the arm.
fn item_for(project_subject: &str, row: SourceRow) -> ObservedItem {
    ObservedItem {
        key: DebtKey::singleton(project_subject, row.source),
        scoring: row.default_scoring,
        location: None,
        basis: row.basis,
        path_bytes: None,
        path_display: None,
        line: None,
        column: None,
        salient_text: None,
    }
}

// ---------------------------------------------------------------------------------------------
// The presence arms — §29's HEAD-basis path predicates
// ---------------------------------------------------------------------------------------------

/// Which of §29's four answers this arm reads.
#[derive(Debug, Clone, Copy)]
enum Which {
    Readme,
    License,
    Tests,
}

#[derive(Debug)]
struct PresenceArm {
    source: DebtSource,
    which: Which,
}

impl SingletonArm for PresenceArm {
    fn source(&self) -> DebtSource {
        self.source
    }

    fn observe(&self, tx: &Transaction<'_>, project: ProjectId) -> Result<ArmReading, DebtError> {
        // **`None` is row-absent, which is neither `absent` nor `not_read`** — J7 has never
        // observed this project. It reaches the same outcome as `not_read` and is a different
        // fact; both are *never observed* and neither is a false.
        let Some(answers) = presence_for_project(tx, project)? else {
            return Ok(ArmReading::Unobservable);
        };
        let state = match self.which {
            Which::Readme => answers.readme,
            Which::License => answers.license,
            Which::Tests => answers.tests,
        };
        Ok(match state {
            PresenceState::Present => ArmReading::PredicateTrue,
            PresenceState::Absent => {
                ArmReading::PredicateFalse(item_for("", *registry_for(self.source)))
            }
            PresenceState::NotRead => ArmReading::Unobservable,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// `unpushed_commits` — J1's `ahead` on the primary copy
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
struct UnpushedArm;

impl SingletonArm for UnpushedArm {
    fn source(&self) -> DebtSource {
        DebtSource::UnpushedCommits
    }

    fn observe(&self, tx: &Transaction<'_>, project: ProjectId) -> Result<ArmReading, DebtError> {
        let locations = locations_of(tx, project)
            .map_err(|e| DebtError::Codec(format!("locations_of: {e}")))?;
        let Some(primary) = pick_primary(&locations) else {
            return Ok(ArmReading::Unobservable);
        };
        // **NULL is never observed, and `0` would be a claim nobody measured.**
        let Some(ahead) = primary.ahead else {
            return Ok(ArmReading::Unobservable);
        };
        Ok(if ahead > 0 {
            ArmReading::PredicateFalse(item_for("", *registry_for(DebtSource::UnpushedCommits)))
        } else {
            ArmReading::PredicateTrue
        })
    }
}

// ---------------------------------------------------------------------------------------------
// `ci_red` — §25.4's run record, on the branch the primary copy is on
// ---------------------------------------------------------------------------------------------

/// **R145's recorded fallback, because no column holds a default branch.** `remote_repo` declares
/// none and `remote_ci_run` stores `branch` per run with nothing marking which is the default —
/// the fifth instance of §27.7's shape. The fallback is **the latest concluded run
/// (`started_at` descending, `conclusion IS NOT NULL`) on the primary copy's `location.branch`,
/// and `Unobservable` otherwise**. An author who invents a different one silently changes which
/// runs this reads.
#[derive(Debug)]
struct CiRedArm;

impl SingletonArm for CiRedArm {
    fn source(&self) -> DebtSource {
        DebtSource::CiRed
    }

    fn observe(&self, tx: &Transaction<'_>, project: ProjectId) -> Result<ArmReading, DebtError> {
        let locations = locations_of(tx, project)
            .map_err(|e| DebtError::Codec(format!("locations_of: {e}")))?;
        let Some(branch) = pick_primary(&locations).and_then(|l| l.branch.clone()) else {
            return Ok(ArmReading::Unobservable);
        };
        // A run still in flight carries a NULL conclusion and is not a reading.
        let conclusion: Option<String> = tx
            .query_row(
                "SELECT r.conclusion
                   FROM remote_ci_run r
                   JOIN project p
                     ON p.provider = r.provider AND p.provider_repo_id = r.provider_repo_id
                  WHERE p.id = ?1 AND r.branch = ?2 AND r.conclusion IS NOT NULL
                  ORDER BY r.started_at DESC, r.run_id DESC
                  LIMIT 1",
                rusqlite::params![project.0, branch],
                |r| r.get(0),
            )
            .optional()?;
        let Some(conclusion) = conclusion else {
            return Ok(ArmReading::Unobservable);
        };
        // The forge's vocabulary is stored verbatim and is not mirrored by an enum (§25.4), so
        // the predicate is *the run concluded well*, and everything else is the item.
        Ok(if conclusion == "success" {
            ArmReading::PredicateTrue
        } else {
            ArmReading::PredicateFalse(item_for("", *registry_for(DebtSource::CiRed)))
        })
    }
}

// ---------------------------------------------------------------------------------------------
// `no_release` — J1's tag count on the primary copy, against the project's shallowness
// ---------------------------------------------------------------------------------------------

/// §31.2's rule, **implemented once and implemented here** (R124). §31's evaluator reads the
/// result and re-derives nothing.
///
/// ```text
/// Unobservable    the primary copy is absent, or tag_count IS NULL,
///                 or (tag_count = 0 AND project.is_shallow = 1)
/// PredicateTrue   tag_count >= 1                     -> §31's `release` = pass
/// PredicateFalse  tag_count = 0 AND is_shallow = 0   -> §31's `release` = fail, one item
/// ```
///
/// **It is a two-table read** (A13.2): `tag_count` is a `location` column added by
/// `0015_completion.sql` and `is_shallow` is a `project` one
/// (`core/migrations/0001_meta_and_projects.sql:57`). A depth-1 clone fetches no tags, so a
/// stored `0` there says *not fetched* and not *no release* — reading it as a failure opens a
/// `no_release` item on a repository whose tags nobody downloaded.
///
/// **The primary copy's value alone**, per §5.1. A modelling defect sits underneath and is not
/// phase-3 scope: shallowness is a property of a particular working copy and the flag is per
/// project, so a project with one shallow and one full copy carries a single flag that can only
/// be right about one of them. The outcome is pinned toward `Unobservable` — *not observed* is
/// the only direction §31.2 permits.
#[derive(Debug)]
struct NoReleaseArm;

impl SingletonArm for NoReleaseArm {
    fn source(&self) -> DebtSource {
        DebtSource::NoRelease
    }

    fn observe(&self, tx: &Transaction<'_>, project: ProjectId) -> Result<ArmReading, DebtError> {
        let Some(primary) = primary_of(tx, project)? else {
            return Ok(ArmReading::Unobservable);
        };
        let (tag_count, is_shallow): (Option<i64>, i64) = tx.query_row(
            "SELECT l.tag_count, p.is_shallow
               FROM location l JOIN project p ON p.id = l.project_id
              WHERE l.id = ?1",
            [primary.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        // **NULL is never observed** — J1 has not persisted a refstate for this copy — and it
        // stays `Unobservable` whatever `is_shallow` says.
        let Some(tag_count) = tag_count else {
            return Ok(ArmReading::Unobservable);
        };
        Ok(if tag_count >= 1 {
            ArmReading::PredicateTrue
        } else if is_shallow != 0 {
            ArmReading::Unobservable
        } else {
            ArmReading::PredicateFalse(item_for("", *registry_for(DebtSource::NoRelease)))
        })
    }
}

const README_ARM: PresenceArm = PresenceArm {
    source: DebtSource::MissingReadme,
    which: Which::Readme,
};
const LICENSE_ARM: PresenceArm = PresenceArm {
    source: DebtSource::MissingLicense,
    which: Which::License,
};
const TESTS_ARM: PresenceArm = PresenceArm {
    source: DebtSource::MissingTests,
    which: Which::Tests,
};

/// The exhaustive list; `.len()` is a tripwire on its own growth.
pub const SINGLETON_ARMS: [&dyn SingletonArm; 7] = [
    &README_ARM,
    &LICENSE_ARM,
    &TESTS_ARM,
    &NoReleaseArm,
    &UnpushedArm,
    &CiRedArm,
    // Last, and the order is load-bearing: its conjunct counts the other sources' **stored** open
    // items, so it must read a set this same transaction has already settled.
    &super::abandoned::AbandonedArm,
];

/// Run every arm for one project, in the caller's transaction.
///
/// **Two call sites, both this plan's (R145): `JobRunner::settle` and `SyncRunner::settle`**, and
/// it runs **before** §31's completion evaluator at both, or every Group-A check answers from the
/// previous settle. Two plans owning two call sites of one function is how a third gets added
/// later by whichever plan notices first.
///
/// It emits no event: **R121** gives the `projects.upserted` emit to §30, which also owns the
/// volume question.
///
/// # Errors
/// Fails when SQLite refuses a read or write — the subject, the switches, the N/A set, an arm's
/// own reading, an anchor's root, or the store's diff — or a stored enum is not a value this
/// build's schema declares.
pub fn evaluate_singletons(
    tx: &Transaction<'_>,
    project: ProjectId,
    now: i64,
    store: &dyn DebtStore,
) -> Result<SweepEffect, DebtError> {
    let subject_key = crate::index::subject::subject_for_project(tx, project)?
        .map(|s| s.to_key())
        .unwrap_or_default();
    let not_applicable = crate::completion::inputs::not_applicable_sources(tx, project)?;
    let off = crate::health::switches::off_sources(
        &crate::health::switches::read_switches(tx)?,
        crate::surfaces::settings::content_scan_enabled(tx)?,
    );

    let mut total = SweepEffect::default();
    for arm in SINGLETON_ARMS {
        let source = arm.source();
        // An `off` source (§30.9) and an N/A one (§31.9: *a check that is `na` produces no debt
        // item*) are not swept at all. The arm does not run, so it opens nothing and closes
        // nothing — an item already open is set aside by the reading (§30), never closed and
        // never paid — and no sweep row is written: nothing was observed. Switch-off deleted this
        // source's row, and writing it back would make a check switched on again read as swept.
        if off.contains(&source) || not_applicable.contains(&source) {
            continue;
        }
        let row = registry_for(source);
        let anchor = anchor_for(tx, project, *row)?;
        let reading = arm.observe(tx, project)?;

        let (proposed, seen) = match reading {
            ArmReading::PredicateFalse(mut item) => {
                // The anchor, basis and key belong to the registry and to the subject, never to
                // the arm: an arm that computed its own could hand `may_close` a pair that never
                // compares, and the item would never close.
                item.key = DebtKey::singleton(&subject_key, source);
                item.location = anchor;
                item.basis = row.basis;
                (DebtSweepOutcome::Complete, vec![item])
            }
            ArmReading::PredicateTrue => (DebtSweepOutcome::Complete, Vec::new()),
            ArmReading::Unobservable => (DebtSweepOutcome::Unobservable, Vec::new()),
        };

        let observes = proposed == DebtSweepOutcome::Complete;
        let mut obs = SweepObservation {
            project,
            source,
            outcome: proposed,
            location: anchor,
            generation: None,
            basis: row.basis,
            item_count: observes.then(|| u32::try_from(seen.len()).unwrap_or(u32::MAX)),
            observed_at: now,
        };
        // Rule 3: the freeze is applied before the diff, never after.
        obs.outcome = outcome_at_root(tx, anchor, obs.outcome)?;
        if obs.outcome != DebtSweepOutcome::Complete && obs.outcome != DebtSweepOutcome::Partial {
            obs.item_count = None;
        }

        let effect = store.observe(tx, &obs, &seen)?;
        total.opened.extend(effect.opened);
        total.closed.extend(effect.closed);
        total.refreshed = total.refreshed.saturating_add(effect.refreshed);
        total.unverified = total.unverified.saturating_add(effect.unverified);
    }
    Ok(total)
}

/// [`evaluate_singletons`] plus the day's payout, in one transaction.
///
/// **The one function both settle hooks call**, so the payout cannot be wired at one site and
/// forgotten at the other. The ledger row and the item deletions commit together: a tree with the
/// items gone and no payout, or a payout with the items still open, is the state the ordering
/// exists to prevent.
///
/// # Errors
/// Fails wherever [`evaluate_singletons`] or [`pay_debt_day`] does.
///
/// [`pay_debt_day`]: super::xp::pay_debt_day
pub fn settle_singletons(
    tx: &Transaction<'_>,
    project: ProjectId,
    now: i64,
    tz_offset_min: i32,
    store: &dyn DebtStore,
) -> Result<SweepEffect, DebtError> {
    let effect = evaluate_singletons(tx, project, now, store)?;
    super::xp::pay_debt_day(tx, project, &effect, now, tz_offset_min)?;
    Ok(effect)
}
