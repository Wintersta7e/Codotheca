//! The **pure** ten-check evaluator: a shaped input struct to ten rows.
//!
//! No database, no remote, no clock beyond the `now` handed in — so every branch is reachable
//! from a test without a fixture repository, and the file that reads tables (`super::inputs`) has
//! no decision in it.
//!
//! **Two groups, and the split is R124's.**
//!
//! - **Group A — `readme`, `license`, `tests`, `pushed`, `ciGreen`, `release`** read §28's stored
//!   answer and **re-derive nothing**. §31 never asks *is there a README*; it asks *was the
//!   evidence available at all*, and maps §28's non-`complete` outcomes onto §31.7a's reasons.
//! - **Group B — `remote`, `description`, `ci`, `deps`** own no debt item, so nothing else
//!   evaluates them and there is no second implementation to collide with.
//!
//! **The two units never meet** (A12b): `evaluable`, `lit`, `unknown` and `na` count **checks**.
//! Debt-item counts are a different quantity and never enter the same expression — the one place
//! an item count is read at all is `deps`' set-emptiness, which is a boolean by the time it
//! arrives here.

use crate::jobs::presence::PresenceState;
use crate::protocol::{
    CheckState, CompletionCheck, DebtSource, DebtSweepOutcome, DependencyVerdict, RemoteFactsState,
    UnknownReason,
};

use super::proposal::{proposes_na, suppressed_source};

/// §28's stored answer for one source, shaped for the map below.
///
/// `outcome: None` is **no sweep row** — this source was never observed — which is `notRunYet`
/// and never a zero. §31.1b follows §30.3's *an item observed is an item*: a scored open item
/// fails the check whatever the sweep outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SingletonReading {
    /// The §28 debt source this reading was taken from.
    pub source: DebtSource,
    /// The source's `debt_sweep` outcome; `None` is no sweep row.
    pub outcome: Option<DebtSweepOutcome>,
    /// Scored open items for the source. **A count of ITEMS**, read only as `>= 1`.
    pub open_items: u32,
}

impl SingletonReading {
    /// A source that has never been swept.
    #[must_use]
    pub const fn never_observed(source: DebtSource) -> Self {
        Self {
            source,
            outcome: None,
            open_items: 0,
        }
    }
}

/// §32's answer for `deps`, which reads a set and owns no item (A8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepsReading {
    /// §32.8's verdict, which already folds the sweep's completeness.
    pub verdict: DependencyVerdict,
    /// Open `dependency_advisory` items whose scoring is `scored`. **A count of ITEMS**, read
    /// here only for emptiness — it never enters an expression with a check count.
    pub scored_open: u32,
    /// **R131/F7's second half.** A lockfile the read could not take is `notRead`; an unreachable
    /// or unsynced source is `notSynced`. Telling a user their dependency check is waiting on a
    /// network read when it is waiting on a 17 MB lockfile is §31.7a's own charge against the
    /// prototype's single `NEEDS GITHUB` note.
    pub lockfile_not_read: bool,
}

/// Everything the ten checks need, already shaped.
///
/// **No refstate fact and no `is_shallow`.** Those moved into §28's arms with the predicate
/// (R124), and `completion_evaluator.rs`'s source walk fails the build if they reappear here.
#[derive(Debug, Clone)]
pub struct CompletionInputs {
    /// One reading per Group-A source. Named fields rather than a map, so a missing source is a
    /// compile error instead of a lookup that answers `None`.
    ///
    /// **The fields carry §28's SOURCE spelling, not §31's check key** — `ci_red` is what stands
    /// behind the check whose key is `CompletionCheck::CiGreen`. §31.9 puts the two vocabularies
    /// in bijection and forbids conflating them, and naming an input after the thing it was read
    /// from is what makes `super::inputs` legible beside `debt_sweep`. It is also what keeps
    /// that file clear of `remote_no_completion_writer`, which bans the snake-case aggregate
    /// spelling from any file naming a remote marker.
    pub readme: SingletonReading,
    /// `missing_license`, behind `CompletionCheck::License`.
    pub license: SingletonReading,
    /// `missing_tests`, behind `CompletionCheck::Tests`.
    pub tests: SingletonReading,
    /// `ci_red`, behind `CompletionCheck::CiGreen`.
    pub ci_red: SingletonReading,
    /// `unpushed_commits`, behind `CompletionCheck::Pushed`.
    pub pushed: SingletonReading,
    /// `no_release`, behind `CompletionCheck::Release`.
    pub release: SingletonReading,

    /// §29.5's `has_ci` tri-state. `None` is **row-absent** — J7 has never observed this project
    /// — which is *never observed* and is neither `absent` nor `not_read`.
    pub has_ci: Option<PresenceState>,

    /// `true` when a remote is configured on the working copy (§31.1a's `remote` predicate, at
    /// `refs` basis). `None` is *no working copy, or no refstate observed*.
    pub remote_configured: Option<bool>,

    /// Whether any account is connected, which is what separates `needsAccount` from
    /// `notSynced` for `description` and for `ciGreen`'s mapped reason.
    pub account_connected: bool,
    /// §25.1's state for this project's forge row.
    pub facts_state: RemoteFactsState,
    /// `None` is *this project has no remote at all*, which makes `description` `na`.
    pub has_remote: bool,
    /// Read from the forge's own row and **not** from `project.description_source`: a user note
    /// that wins the description chain does not delete the forge's description.
    pub forge_description: Option<String>,
    /// How many topics the forge row carries; `description` passes only with at least one.
    pub topic_count: u32,

    /// §32's reading for the `deps` check.
    pub deps: DepsReading,

    /// J3's archetype. `None` is a NULL column — J3 has not run — and proposes nothing.
    pub archetype: Option<String>,
    /// The user's stored ruling per key, in `CompletionCheck::ALL` order. `None` is *the user has
    /// not ruled*; `Some(false)` is the user **overriding** a proposal.
    pub user_na: [Option<bool>; 10],
}

/// One evaluated check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckRow {
    /// Which of the ten checks this row is.
    pub key: CompletionCheck,
    /// The check's evaluated state.
    pub state: CheckState,
    /// The user's stored N/A ruling for this check, carried through unchanged.
    pub user_na: Option<bool>,
    /// §31.7a's reason, set exactly when `state` is `unknown`.
    pub unknown_reason: Option<UnknownReason>,
    /// When this state was last established, in unix seconds (§31.5).
    pub observed_at: i64,
}

/// §31.1b's four counts, over **checks**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// Checks that pass.
    pub lit: u32,
    /// Checks that pass or fail: the denominator `lit` is read against.
    pub evaluable: u32,
    /// Checks whose state is `unknown`.
    pub unknown: u32,
    /// Checks ruled or proposed not applicable.
    pub na: u32,
}

impl Counts {
    /// # Panics
    /// If the four counts do not sum to ten, which would mean a state outside the four.
    #[must_use]
    pub fn of(rows: &[CheckRow; 10]) -> Self {
        let mut counts = Self {
            lit: 0,
            evaluable: 0,
            unknown: 0,
            na: 0,
        };
        for row in rows {
            match row.state {
                CheckState::Pass => {
                    counts.lit += 1;
                    counts.evaluable += 1;
                }
                CheckState::Fail => counts.evaluable += 1,
                CheckState::Unknown => counts.unknown += 1,
                CheckState::Na => counts.na += 1,
            }
        }
        assert_eq!(
            counts.evaluable + counts.unknown + counts.na,
            10,
            "§31.1b: the three partitions of ten checks"
        );
        counts
    }
}

/// A state with no reason, for the three that are never `unknown` at the point they are built.
const fn plain(state: CheckState) -> (CheckState, Option<UnknownReason>) {
    (state, None)
}

const fn unknown(reason: UnknownReason) -> (CheckState, Option<UnknownReason>) {
    (CheckState::Unknown, Some(reason))
}

/// §31.7a's per-key reason for a Group-A source that could not be observed.
///
/// **This is §31's alone**, because `ArmReading` and `debt_sweep` carry no `UnknownReason` — §28
/// answers *observable or not*, and which sentence a user is owed is a §31 question.
const fn reason_for(key: CompletionCheck, account_connected: bool) -> UnknownReason {
    match key {
        CompletionCheck::Readme | CompletionCheck::License | CompletionCheck::Tests => {
            UnknownReason::NotRead
        }
        CompletionCheck::CiGreen => {
            if account_connected {
                UnknownReason::NotSynced
            } else {
                UnknownReason::NeedsAccount
            }
        }
        // `pushed` and `release`, and any key that reaches here by mistake: a fact about the
        // repository that does not resolve on its own.
        _ => UnknownReason::NotObserved,
    }
}

/// §28's stored answer, mapped onto a state. **Total**, and it re-derives no predicate.
/// §31.1b follows §30.3's *an item observed is an item*: a scored open item fails before the
/// sweep outcome is considered; without one, only a complete outcome can pass.
const fn from_singleton(
    key: CompletionCheck,
    reading: SingletonReading,
    account_connected: bool,
) -> (CheckState, Option<UnknownReason>) {
    if reading.open_items >= 1 {
        return plain(CheckState::Fail);
    }
    match reading.outcome {
        // **Four causes, one answer, and the answer is `notRunYet` for all four because all
        // four resolve at the next sweep** — which is exactly what separates `notRunYet` from
        // `notObserved`, a fact about the repository that never resolves on its own.
        //
        // * `None` — no sweep row: this source was never observed.
        // * `skipped_suppressed` — the sweep was declined, so nothing was looked at.
        // * `partial` — with no standing item, a partial sweep may still have missed one, so it
        //   cannot prove a pass.
        // * `skipped_reference` — unreachable, because `gather` excludes a Reference project
        //   before this read. Named anyway: a total `match` cannot acquire a silent default.
        None
        | Some(
            DebtSweepOutcome::SkippedSuppressed
            | DebtSweepOutcome::Partial
            | DebtSweepOutcome::SkippedReference,
        ) => unknown(UnknownReason::NotRunYet),
        Some(DebtSweepOutcome::Unobservable | DebtSweepOutcome::Failed) => {
            unknown(reason_for(key, account_connected))
        }
        Some(DebtSweepOutcome::Complete) => plain(CheckState::Pass),
    }
}

/// §31.3's tri-state, for the one content check that owns no debt item.
///
/// **A budget exceedance is `not_read` and therefore `unknown`, never `absent` and never
/// `fail`** — the single most likely place this phase renders unknown as zero, because a timeout
/// looks exactly like a missing file. Row-absent is neither: it is *never observed*.
const fn from_presence(state: Option<PresenceState>) -> (CheckState, Option<UnknownReason>) {
    match state {
        Some(PresenceState::Present) => plain(CheckState::Pass),
        Some(PresenceState::Absent) => plain(CheckState::Fail),
        Some(PresenceState::NotRead) => unknown(UnknownReason::NotRead),
        None => unknown(UnknownReason::NotRunYet),
    }
}

/// §31.1a's `remote`: a remote configured on the working copy, at `refs` basis.
const fn from_remote(configured: Option<bool>) -> (CheckState, Option<UnknownReason>) {
    match configured {
        Some(true) => plain(CheckState::Pass),
        Some(false) => plain(CheckState::Fail),
        // No working copy, or a copy whose refstate was never persisted. A fact about the
        // repository, so it does not resolve on its own.
        None => unknown(UnknownReason::NotObserved),
    }
}

/// §31.1a's `description`: a forge description **and** at least one topic.
fn from_description(inputs: &CompletionInputs) -> (CheckState, Option<UnknownReason>) {
    match inputs.facts_state {
        RemoteFactsState::NoAccount => unknown(UnknownReason::NeedsAccount),
        // A row this account may not read is not a row nobody read; both are *the value has not
        // come back from its source*.
        RemoteFactsState::NotPermitted | RemoteFactsState::NotObserved => {
            unknown(UnknownReason::NotSynced)
        }
        RemoteFactsState::Observed => {
            let described = inputs
                .forge_description
                .as_deref()
                .is_some_and(|d| !d.trim().is_empty());
            if described && inputs.topic_count >= 1 {
                plain(CheckState::Pass)
            } else {
                plain(CheckState::Fail)
            }
        }
    }
}

/// §31.9's `deps`: `fail` iff at least one `scored` open advisory item, `pass` iff zero **and**
/// the sweep was complete, `unknown` otherwise.
///
/// The completeness conjunct is what stops zero advisories on an unread lockfile reading as a
/// pass, and §32's own verdict already carries it.
const fn from_deps(deps: DepsReading) -> (CheckState, Option<UnknownReason>) {
    if deps.scored_open >= 1 {
        return plain(CheckState::Fail);
    }
    match deps.verdict {
        // Both are a COMPLETE sweep with nothing this check counts: `vulnerable` with no scored
        // open item is a shown-only advisory, which is observed and is not a failure. The
        // completeness conjunct is what stops zero advisories on an unread lockfile passing.
        DependencyVerdict::Clean | DependencyVerdict::Vulnerable => plain(CheckState::Pass),
        DependencyVerdict::Unknown => {
            if deps.lockfile_not_read {
                unknown(UnknownReason::NotRead)
            } else {
                unknown(UnknownReason::NotSynced)
            }
        }
    }
}

/// §31.4's gate, which runs **before** every read.
///
/// ```text
/// state = 'na'  iff  user_na = 1  or  (user_na IS NULL and the archetype proposes na)
/// ```
///
/// `user_na = 0` is the user overriding a proposal: the check is evaluated.
fn is_na(key: CompletionCheck, user_na: Option<bool>, archetype: Option<&str>) -> bool {
    match user_na {
        Some(true) => true,
        Some(false) => false,
        None => proposes_na(archetype, key),
    }
}

/// §30.3's `notApplicable` input, in §28's vocabulary: whether the check standing behind `source`
/// (§31.9's bijection) is N/A for this project, by the gate above and no second one.
///
/// `user_na` is in `CompletionCheck::ALL` order, as [`CompletionInputs::user_na`] is. A source no
/// check stands behind is never N/A. **§31.1a's derived `ciGreen` rule is not read**: `ci_red`
/// is N/A only when `ciGreen` itself is, which is what `suppressed_source` promises §30.
#[must_use]
pub fn source_not_applicable(
    source: DebtSource,
    archetype: Option<&str>,
    user_na: &[Option<bool>; 10],
) -> bool {
    CompletionCheck::ALL
        .iter()
        .zip(user_na)
        .any(|(key, ruling)| {
            suppressed_source(*key) == Some(source) && is_na(*key, *ruling, archetype)
        })
}

/// The ten checks, in `CompletionCheck` declaration order.
///
/// Total and pure. `now` stamps every row it builds; the writer decides which stamps survive —
/// `observed_at` is when a state was last **established**, not when it was last attempted, so a
/// no-change recompute leaves it where it was (§31.5, R123).
///
/// **No `unreachable` reason is produced here.** §31.8 freezes a project whose primary copy is
/// `offline` or `missing` rather than recomputing it, and a reading that was never computed may
/// not be frozen — so `gather` declines both cases before the evaluator is reached. The variant
/// stays in §31.7a's note table because the renderer must be able to name it; §31 has no
/// producer for it in this phase.
#[must_use]
pub fn evaluate(inputs: &CompletionInputs, now: i64) -> [CheckRow; 10] {
    let archetype = inputs.archetype.as_deref();
    let account = inputs.account_connected;

    let mut rows: [CheckRow; 10] = CompletionCheck::ALL.map(|key| CheckRow {
        key,
        state: CheckState::Unknown,
        user_na: None,
        unknown_reason: Some(UnknownReason::NotRunYet),
        observed_at: now,
    });

    // `ci`'s own answer is needed before `ciGreen`'s N/A gate, so it is computed first and read
    // back below rather than evaluated twice.
    let ci = from_presence(inputs.has_ci);

    for (index, row) in rows.iter_mut().enumerate() {
        let key = row.key;
        let user_na = inputs.user_na.get(index).copied().flatten();
        row.user_na = user_na;

        if is_na(key, user_na, archetype) {
            row.state = CheckState::Na;
            row.unknown_reason = None;
            continue;
        }

        let (state, reason) = match key {
            // Group A — §28's stored answer, re-deriving nothing.
            CompletionCheck::Readme => from_singleton(key, inputs.readme, account),
            CompletionCheck::License => from_singleton(key, inputs.license, account),
            CompletionCheck::Tests => from_singleton(key, inputs.tests, account),
            CompletionCheck::Pushed => from_singleton(key, inputs.pushed, account),
            CompletionCheck::Release => from_singleton(key, inputs.release, account),
            CompletionCheck::CiGreen => {
                // §31.1a: `na` when `ci` is `na` or `fail`. This is §31's rule and it runs
                // before the read — a green run on a project with no CI config is not a claim
                // this check may make.
                let ci_index = CompletionCheck::ALL
                    .iter()
                    .position(|k| *k == CompletionCheck::Ci)
                    .unwrap_or(0);
                let ci_na = is_na(
                    CompletionCheck::Ci,
                    inputs.user_na.get(ci_index).copied().flatten(),
                    archetype,
                );
                if ci_na || ci.0 == CheckState::Fail {
                    plain(CheckState::Na)
                } else {
                    from_singleton(key, inputs.ci_red, account)
                }
            }

            // Group B — §31's own evaluation, because nothing else evaluates them.
            CompletionCheck::Remote => from_remote(inputs.remote_configured),
            CompletionCheck::Description => {
                if inputs.has_remote {
                    from_description(inputs)
                } else {
                    // §31.1a: `na` when there is no remote. A description a project cannot have
                    // is not a check it failed.
                    plain(CheckState::Na)
                }
            }
            CompletionCheck::Ci => ci,
            CompletionCheck::Deps => from_deps(inputs.deps),
        };
        row.state = state;
        row.unknown_reason = reason;
    }

    rows
}
