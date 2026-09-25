//! §45's deletion analyser: **the one function** that decides whether a governed act may destroy a
//! byte, and every construction of an `UninstallBlocker`.
//!
//! [`analyse`] takes the location's row and the act and returns the verdict. Its production
//! callers are `locations.uninstallPreflight` and `locations.uninstall` (§24); no other code
//! computes a blocker. **The order is load-bearing** (§45.6):
//!
//! 1. resolve the directory and identify it against the row ([`identity`]);
//! 2. the static gates, all local ([`gates`]), and git at or above the governed floor;
//! 3. snapshot the roots ([`roots`]);
//! 4. the worktree, and 5. nested repositories ([`worktree`]);
//! 6. read every configured remote ([`remote`]) — **only** when roots need an elsewhere and no
//!    blocker so far is undischargeable for the act;
//! 7. one reachability walk ([`roots::uncovered`]);
//! 8. compose ([`compose`]) and fold ([`verdict`]).
//!
//! **Every `Err` is its input's unknown blocker, never a skip**, and the whole analysis runs
//! inside [`ANALYSIS_BUDGET`] as well as each read's own deadline. Every git read runs off the
//! index lock: the row is read first, under the caller's guard, and nothing here touches SQLite.

pub mod compose;
pub mod gates;
pub mod identity;
pub mod junk;
pub mod nested;
pub mod remote;
pub mod roots;
pub mod snapshot;
pub mod verdict;
pub mod worktree;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::cancel::CancelToken;
use crate::git::{GitBackend, JobClass, JobContext, RepoHandle, StoreKey};
use crate::gitw::floor::meets_governed_floor;
use crate::gitw::intent::GIT_INVOCATION_DEADLINE;
use crate::mount::StoreClass;
use crate::proto::dispatch::CommandFailure;
use crate::protocol::{
    LocationId, NestedRepository, PreciousSummary, TrashRefusalKind, UninstallBlocker,
    UninstallDisposition, UninstallVerdict,
};
use crate::removal::{Trash, TrashAvailability};

use self::compose::{compose_remote_blockers, is_undischargeable, RemoteSummary};
use self::gates::OtherLocation;
use self::identity::{identify, IdentityOutcome};
use self::remote::{RemoteReading, RemoteVerifier};
use self::verdict::{fold_disposition, VerdictSeal};

/// The whole analysis's budget, beside each read's `GIT_INVOCATION_DEADLINE`. **Declared, not
/// yet measured Windows-native**: exhausting it gives the unknown blocker of whatever input had
/// not finished.
pub const ANALYSIS_BUDGET: Duration = Duration::from_secs(60);

/// An act §45.1 governs. Lane 0 governs Uninstall; §46 adds its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GovernedAct {
    /// `locations.uninstall` and its pre-flight.
    Uninstall,
}

/// The analyser's seams. **The read backend and the remote verifier are the only two a test may
/// stand in for** (§45.14); the trash is read for its availability, never sent anything here.
#[derive(Clone, Copy)]
pub struct AnalyserSeams<'a> {
    /// Every read of the repository.
    pub git: &'a dyn GitBackend,
    /// Step 6's read of one configured remote.
    pub remotes: &'a dyn RemoteVerifier,
    /// Where an act's bytes would go, for §46.7's availability reading.
    pub trash: &'a dyn Trash,
    /// Test-only: runs between step 8 and step 9 of an act, where a perturbation must be seen.
    #[cfg(feature = "testkit")]
    pub before_act: Option<&'a dyn Fn()>,
}

impl std::fmt::Debug for AnalyserSeams<'_> {
    /// By hand: three seams whose contents are not a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalyserSeams").finish_non_exhaustive()
    }
}

/// What one analysis produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    /// The verdict, as the wire carries it.
    pub verdict: UninstallVerdict,
    /// Its in-core seal: an act's warrant is sealed over this analysis and no other.
    pub seal: VerdictSeal,
    /// §45.8's snapshot, taken when the verdict is `safe` — the content step 9 compares against.
    /// `None` for any other verdict, which no act proceeds on.
    pub snapshot: Option<snapshot::Snapshot>,
}

/// Why step 9 refused an act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActRefusal {
    /// The directory no longer resolves.
    Unresolvable,
    /// It is not the row's repository any more, or its identity cannot be derived.
    IdentityChanged,
    /// The analysis took no snapshot, so there is no content to bind.
    NotSnapshotted,
    /// The snapshot could not be re-read.
    SnapshotUnreadable(UninstallBlocker),
    /// The content changed since the analysis.
    Changed,
}

/// §45.6 step 9: **re-read before the act.**
///
/// Identity against the row, and the snapshot against the analysis's; any difference refuses. It
/// runs after the network and before the removal primitive, and nothing runs between it and the
/// removal but the removal.
///
/// # Errors
/// The [`ActRefusal`] that stops the act.
pub fn reread_before_act(
    row: &LocationRow,
    analysis: &Analysis,
    seams: &AnalyserSeams<'_>,
) -> Result<identity::LiveIdentity, ActRefusal> {
    let recorded = analysis
        .snapshot
        .as_ref()
        .ok_or(ActRefusal::NotSnapshotted)?;
    let repo = resolve(row).ok_or(ActRefusal::Unresolvable)?;
    let cancel = CancelToken::new();
    let ctx = JobContext::new(
        JobClass::Interactive,
        &cancel,
        Some(GIT_INVOCATION_DEADLINE),
    );
    match identify(seams.git, &repo, row.lineage_key.as_deref(), &ctx) {
        IdentityOutcome::Match => {}
        IdentityOutcome::Mismatch | IdentityOutcome::Shallow | IdentityOutcome::Unreadable => {
            return Err(ActRefusal::IdentityChanged)
        }
    }
    let live =
        snapshot::snapshot(&repo, seams.git, &ctx).map_err(ActRefusal::SnapshotUnreadable)?;
    if live.digest() != recorded.digest() {
        return Err(ActRefusal::Changed);
    }
    Ok(identity::LiveIdentity::Derived(row.lineage_key.clone()))
}

/// The `location` row and what §45.7 lets the analyser read beside it — nothing else.
///
/// **`project.is_shallow` is not here, and neither is any cached observation of the tree.**
/// Shallowness is read live (step 1); the row supplies what the live reads are compared
/// against. `store` and `trusted` are invocation plumbing — the slot pool and
/// `-c safe.directory` — not verdict inputs (D-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationRow {
    /// The row read.
    pub id: LocationId,
    /// `path_bytes`, decoded: resolved and re-statted, never trusted as a repository.
    pub path: PathBuf,
    /// `project.lineage_key`: what step 1's live derivation is compared with.
    pub lineage_key: Option<String>,
    /// `removed_at`, which refuses.
    pub removed_at: Option<i64>,
    /// When ref state was last observed; `None` means the app has never looked (§24.7G).
    pub refstate_observed_at: Option<i64>,
    /// When the worktree was last observed; the same question.
    pub worktree_observed_at: Option<i64>,
    /// The slot pool the reads take their turn in.
    pub store: StoreKey,
    /// `trusted_at` is set: git reads it with `-c safe.directory`.
    pub trusted: bool,
    /// Every configured scan root, enabled or not: a disabled root is still one the user named.
    pub scan_roots: Vec<PathBuf>,
    /// A launch session on this location has not ended.
    pub live_session: bool,
    /// Every other present location, for §45.5's containment and borrowing refusals.
    pub others: Vec<OtherLocation>,
}

/// **The one row read**, in the caller's transaction: `location` joined to its project's
/// `lineage_key`, the scan roots, the live session and the other present locations.
///
/// # Errors
/// `PROTOCOL` when the id names no location; `INTERNAL` for any other SQLite fault.
pub fn read_location_row(
    tx: &rusqlite::Transaction<'_>,
    id: LocationId,
) -> Result<LocationRow, CommandFailure> {
    let internal = |e: rusqlite::Error| CommandFailure::internal(e.to_string());
    let mut row = tx
        .query_row(
            "SELECT l.path_bytes, p.lineage_key, l.removed_at, l.refstate_observed_at,
                    l.worktree_observed_at, l.store_key, l.trusted_at
               FROM location l JOIN project p ON p.id = l.project_id
              WHERE l.id = ?1",
            [id.0],
            |r| {
                Ok(LocationRow {
                    id,
                    path: crate::paths::path_from_bytes(&r.get::<_, Vec<u8>>(0)?),
                    lineage_key: r.get(1)?,
                    removed_at: r.get(2)?,
                    refstate_observed_at: r.get(3)?,
                    worktree_observed_at: r.get(4)?,
                    store: StoreKey::new(r.get::<_, String>(5)?),
                    trusted: r.get::<_, Option<i64>>(6)?.is_some(),
                    scan_roots: Vec::new(),
                    live_session: false,
                    others: Vec::new(),
                })
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                CommandFailure::protocol(format!("no location {}", id.0))
            }
            other => internal(other),
        })?;

    let mut roots = tx
        .prepare("SELECT path_bytes FROM scan_root")
        .map_err(internal)?;
    row.scan_roots = roots
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .map_err(internal)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(internal)?
        .iter()
        .map(|bytes| crate::paths::path_from_bytes(bytes))
        .collect();

    row.live_session = gates::gate_live_session(tx, id)
        .map_err(internal)?
        .is_some();

    let mut others = tx
        .prepare(
            "SELECT path_bytes, common_dir_bytes FROM location
              WHERE id <> ?1 AND removed_at IS NULL",
        )
        .map_err(internal)?;
    row.others = others
        .query_map([id.0], |r| {
            Ok(OtherLocation {
                path: crate::paths::path_from_bytes(&r.get::<_, Vec<u8>>(0)?),
                common_dir: r
                    .get::<_, Option<Vec<u8>>>(1)?
                    .map(|bytes| crate::paths::path_from_bytes(&bytes)),
            })
        })
        .map_err(internal)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(internal)?;
    Ok(row)
}

/// The directory, re-resolved from disk (§24.7E) — never reconstructed from the row.
#[must_use]
pub fn resolve(row: &LocationRow) -> Option<RepoHandle> {
    let mut repo = RepoHandle::resolve(&row.path, row.store.clone(), StoreClass::Local).ok()?;
    repo.trusted = row.trusted;
    Some(repo)
}

/// The analysis's clock: the total budget, and each read's deadline inside what remains of it.
struct Budget {
    started: Instant,
}

impl Budget {
    fn exhausted(&self) -> bool {
        self.started.elapsed() >= ANALYSIS_BUDGET
    }

    fn ctx<'c>(&self, cancel: &'c CancelToken) -> JobContext<'c> {
        let remaining = ANALYSIS_BUDGET.saturating_sub(self.started.elapsed());
        JobContext::new(
            JobClass::Interactive,
            cancel,
            Some(remaining.min(GIT_INVOCATION_DEADLINE)),
        )
    }
}

/// The gates that read no git: they hold whether or not the directory resolves.
fn local_gates(row: &LocationRow, found: &mut Vec<UninstallBlocker>) {
    if row.removed_at.is_some() {
        found.push(UninstallBlocker::RefusedPath);
    }
    found.extend(gates::gate_path(&row.path, &row.scan_roots));
    found.extend(gates::gate_contains(
        &row.path,
        &row.others,
        &row.scan_roots,
    ));
    found.extend(gates::gate_borrowed(&row.path, &row.others));
    found.extend(gates::gate_first_day(
        row.refstate_observed_at,
        row.worktree_observed_at,
    ));
    if row.live_session {
        found.push(UninstallBlocker::LiveSession);
    }
}

/// **The analyser** (§45.6): `row` and `act` in, the verdict out.
#[must_use]
pub fn analyse(
    row: &LocationRow,
    act: GovernedAct,
    seams: &AnalyserSeams<'_>,
    now: i64,
) -> Analysis {
    let budget = Budget {
        started: Instant::now(),
    };
    let cancel = CancelToken::new();
    let git = seams.git;
    let mut found: Vec<UninstallBlocker> = Vec::new();

    // Step 1. A directory that does not resolve is one nothing can reason about: the shipped
    // `never_observed`, beside whatever the local gates say.
    let Some(repo) = resolve(row) else {
        local_gates(row, &mut found);
        found.push(UninstallBlocker::NeverObserved);
        return finish(Findings::of(found), row, seams, now);
    };
    match identify(git, &repo, row.lineage_key.as_deref(), &budget.ctx(&cancel)) {
        IdentityOutcome::Match => {}
        // D-2: a live shallow copy has no lineage to match; it is §24.7B's gate, read live.
        IdentityOutcome::Shallow => found.extend(gates::gate_shallow(true)),
        // A different repository, or no identity at all: **the analysis stops here**, and the
        // one blocker is the whole answer — nothing after step 1 describes the row's repository.
        IdentityOutcome::Mismatch => {
            return finish(
                Findings::of(vec![UninstallBlocker::RefusedPath]),
                row,
                seams,
                now,
            )
        }
        IdentityOutcome::Unreadable => {
            return finish(
                Findings::of(vec![UninstallBlocker::RefsUnreadable]),
                row,
                seams,
                now,
            )
        }
    }

    // Step 2's location gates, and git at or above the governed floor — below it the act is
    // unknown and runs no weaker read (PA1).
    local_gates(row, &mut found);
    found.extend(gates::gate_gitdir_outside(&repo, &repo.work_dir));
    found.extend(gates::gate_linked_worktree(&repo));
    match git.version(&budget.ctx(&cancel)) {
        Ok(version) if meets_governed_floor(&version) => {}
        _ => {
            found.push(UninstallBlocker::RefsUnreadable);
            return finish(Findings::of(found), row, seams, now);
        }
    }
    let mut findings = analyse_repo(&repo, act, seams, &budget, 0, found);
    // §45.8's use 1: a `safe` verdict binds the content it was decided over, for step 9. A
    // snapshot that cannot be read is its input's unknown blocker, and the verdict is not safe.
    let snapshot = if findings.blockers.is_empty() {
        match snapshot::snapshot(&repo, git, &budget.ctx(&cancel)) {
            Ok(taken) => Some(taken),
            Err(blocker) => {
                findings.blockers.push(blocker);
                None
            }
        }
    } else {
        None
    };
    let mut analysis = finish(findings, row, seams, now);
    analysis.snapshot = snapshot;
    analysis
}

/// What steps 2–8 found over one repository: the location's, or a nested one's.
#[derive(Debug, Default)]
struct Findings {
    blockers: Vec<UninstallBlocker>,
    nested: Vec<NestedRepository>,
    precious: Option<PreciousSummary>,
    verified: bool,
}

impl Findings {
    /// An analysis that stopped with `blockers`: nothing enumerated, nothing read remotely.
    fn of(blockers: Vec<UninstallBlocker>) -> Self {
        Self {
            blockers,
            ..Self::default()
        }
    }
}

/// Steps 2–8 over `repo` at `depth` (the location is 0), after `found`: the repository's own
/// gates, its roots, its worktree, its nested repositories — each in full, folded — and its own
/// remotes.
fn analyse_repo(
    repo: &RepoHandle,
    act: GovernedAct,
    seams: &AnalyserSeams<'_>,
    budget: &Budget,
    depth: u32,
    mut found: Vec<UninstallBlocker>,
) -> Findings {
    let cancel = CancelToken::new();
    let git = seams.git;
    found.extend(gates::gate_lfs(repo));
    match git.interrupted_ops(repo, &budget.ctx(&cancel)) {
        Ok(ops) if ops.is_empty() => {}
        Ok(_) => found.push(UninstallBlocker::InterruptedOperation),
        Err(_) => found.push(UninstallBlocker::RefsUnreadable),
    }

    // Step 3: the roots. Every stash entry is `stash_present` here, before the network step:
    // no read that step could make would change an answer the stash has already decided.
    if budget.exhausted() {
        found.push(UninstallBlocker::RefsUnreadable);
        return Findings::of(found);
    }
    let roots = match roots::read_roots(git, repo, &budget.ctx(&cancel)) {
        Ok(roots) => {
            if !roots.stash.is_empty() {
                found.push(UninstallBlocker::StashPresent);
            }
            Some(roots)
        }
        Err(blocker) => {
            found.push(blocker);
            None
        }
    };

    // Step 4, over a repository that has a worktree; a module git dir has none.
    if budget.exhausted() {
        found.push(UninstallBlocker::NeverObserved);
        return Findings::of(found);
    }
    let tree = if repo.work_dir == repo.git_dir {
        worktree::WorktreeFindings::default()
    } else {
        worktree::analyse_worktree(repo, git, &budget.ctx(&cancel))
    };
    found.extend(tree.blockers.iter().copied());

    // Step 5: every nested repository, in full, and the fold. The parent gains
    // `submodule_unsafe` iff one is `blocked`; one that is only `unknown` passes its
    // unknown-class blockers up, so an offline submodule remote leaves the parent `unknown`.
    let mut listed: Vec<NestedRepository> = Vec::new();
    for candidate in nested::candidates(repo, &tree.gitlinks, &tree.in_tree) {
        let entries = analyse_nested(&candidate, act, seams, budget, depth + 1);
        if let Some(child) = entries.first() {
            match child.disposition {
                UninstallDisposition::Blocked => found.push(UninstallBlocker::SubmoduleUnsafe),
                UninstallDisposition::Unknown => found.extend(
                    child
                        .blockers
                        .iter()
                        .copied()
                        .filter(|b| verdict::is_unknown_blocker(*b)),
                ),
                UninstallDisposition::Safe => {}
            }
        }
        listed.extend(entries);
    }

    // Step 6, only when both hold: roots need an elsewhere, and nothing found so far is
    // undischargeable for this act. A verification write that cannot change the answer is not
    // made.
    let verified = match roots {
        Some(roots)
            if roots.need_elsewhere() && !found.iter().any(|b| is_undischargeable(act, *b)) =>
        {
            elsewhere(repo, &roots, seams, budget, &mut found)
        }
        _ => false,
    };
    Findings {
        blockers: found,
        nested: listed,
        precious: tree.precious,
        verified,
    }
}

/// One nested repository, analysed — and every repository nested in it, flattened after it with
/// its path under this one's.
fn analyse_nested(
    candidate: &nested::Candidate,
    act: GovernedAct,
    seams: &AnalyserSeams<'_>,
    budget: &Budget,
    depth: u32,
) -> Vec<NestedRepository> {
    let path_display = candidate.rel.to_string_lossy().replace('\\', "/");
    let unknown = |blocker| {
        vec![NestedRepository {
            path_display: path_display.clone(),
            kind: candidate.kind,
            disposition: UninstallDisposition::Unknown,
            blockers: vec![blocker],
        }]
    };
    if depth > nested::MAX_NESTING {
        return unknown(UninstallBlocker::NestingTooDeep);
    }
    let Some(repo) = &candidate.repo else {
        return unknown(UninstallBlocker::NeverObserved);
    };
    let findings = analyse_repo(repo, act, seams, budget, depth, Vec::new());
    let mut blockers = findings.blockers;
    blockers.sort_unstable_by_key(|b| format!("{b:?}"));
    blockers.dedup();
    let mut out = vec![NestedRepository {
        path_display: path_display.clone(),
        kind: candidate.kind,
        disposition: fold_disposition(&blockers),
        blockers,
    }];
    out.extend(findings.nested.into_iter().map(|mut deeper| {
        deeper.path_display = format!("{path_display}/{}", deeper.path_display);
        deeper
    }));
    out
}

/// Steps 6 to 8's remote half: read every configured remote, walk once against what answered,
/// and compose. True when a network remote answered — `remoteVerifiedAt`'s condition.
fn elsewhere(
    repo: &RepoHandle,
    roots: &roots::Roots,
    seams: &AnalyserSeams<'_>,
    budget: &Budget,
    found: &mut Vec<UninstallBlocker>,
) -> bool {
    let cancel = CancelToken::new();
    if budget.exhausted() {
        found.push(UninstallBlocker::RemoteUnreachable);
        return false;
    }
    let Ok(urls) = seams.git.remote_urls(repo, &budget.ctx(&cancel)) else {
        // A config that could not be read is a remote that was not established.
        found.push(UninstallBlocker::RemoteUnreachable);
        return false;
    };
    let mut names: Vec<String> = Vec::new();
    for (name, _) in urls {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let readings: Vec<RemoteReading> = names
        .iter()
        .map(|name| seams.remotes.read(repo, name, &budget.ctx(&cancel)))
        .collect();
    let summary = RemoteSummary::of(&readings);

    // Steps 7 and 8.
    if budget.exhausted() {
        found.push(UninstallBlocker::RefsUnreadable);
    } else {
        match roots::uncovered(
            seams.git,
            repo,
            roots,
            &summary.covered,
            &budget.ctx(&cancel),
        ) {
            Ok(uncovered) => found.extend(compose_remote_blockers(&summary, uncovered)),
            Err(blocker) => found.push(blocker),
        }
    }
    summary.answered > 0
}

/// Step 8's fold, and the verdict it seals. **Every blocker found is reported** — the fold
/// decides a disposition, it never edits the list.
fn finish(findings: Findings, row: &LocationRow, seams: &AnalyserSeams<'_>, now: i64) -> Analysis {
    let mut blockers = findings.blockers;
    // One blocker of a kind is enough; the list is a vocabulary, not a tally.
    blockers.sort_unstable_by_key(|b| format!("{b:?}"));
    blockers.dedup();
    let disposition = fold_disposition(&blockers);
    let seal = VerdictSeal::of(&blockers, disposition);
    // §24.7F: the copy says what will happen **before the click**. §46.7: the reason and the
    // boolean come from one reading, so `trashAvailable` cannot disagree with `trashRefusal`.
    let trash_refusal = trash_refusal_of(&seams.trash.availability(&row.path));
    Analysis {
        verdict: UninstallVerdict {
            disposition,
            blockers,
            remote_verified_at: findings.verified.then_some(now),
            trash_available: trash_refusal.is_none(),
            computed_at: now,
            nested: findings.nested,
            precious: findings.precious,
            trash_refusal,
        },
        seal,
        snapshot: None,
    }
}

/// §46.7's wire reason for one availability reading: the classifier's, carried as it named it.
const fn trash_refusal_of(availability: &TrashAvailability) -> Option<TrashRefusalKind> {
    match availability {
        TrashAvailability::Available => None,
        TrashAvailability::Unavailable(kind) => Some(*kind),
    }
}
