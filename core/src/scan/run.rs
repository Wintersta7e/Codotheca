//! §4.7 and §4.8 — one scan run.
//!
//! Device identity comes from the `MountResolver` seam, never from the OS directly: without it
//! the removable-drive criterion is untestable by construction (§15.2).
//!
//! Resumption means the walk frontier is **not** persisted. The walk restarts; the observation
//! caches make revisiting nearly free. The one visible consequence is the counter, which is
//! floored at the already-indexed count so a resumed scan never appears to start from zero —
//! `found_repos = max(found_this_run, resumed_from)`. It is monotone, it never retreats, and it
//! converges on the true count as the walk overtakes the resume point.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::assembly::handoff::{hand_off_discovered, HandoffCtx, HandoffError};
use crate::cancel::CancelToken;
use crate::clock::Clock;
use crate::git::{GitBackend, GitError, StoreKey};
use crate::index::path::PathPlatform;
use crate::index::Index;
use crate::mount::{MountFacts, MountResolver, StoreClass};
use crate::paths::{path_bytes, path_display, path_from_bytes, path_key};
use crate::scan::discover::{ProbeCtx, RepoCandidate};
use crate::scan::links::LinkPolicy;
use crate::scan::presence::{
    apply_presence, PresenceContext, PresenceSummary, ScanRootRow, ScanRunFinish, ScanRunStart,
    ScanStore, ScanStoreError,
};
use crate::scan::skiplist::SkipList;
use crate::scan::walk::{walk_root, WalkCtx};
use crate::scan::{ScanProblem, ScanProblemKind, WalkEvent, WalkOptions, WalkSink};

/// A root whose first `metadata` call takes longer than this is treated as a possibly-dead
/// network mount: recorded, and excluded from the run's timing figure (§4.8).
const ROOT_PROBE_SLOW: Duration = Duration::from_millis(2_000);

/// A `RepoCandidate` with everything `location` (§1.3) needs except its ids, which plan 08
/// assigns when it resolves identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// What the walk classified: the directory, its repository kind, git dir and common dir.
    pub candidate: RepoCandidate,
    /// The `scan_root` the repository was found under.
    pub root_id: i64,
    /// `location.kind` — `win` | `linux` | `wsl`, taken from the root.
    pub kind: String,
    /// `location.distro` — NOT NULL, `""` when not WSL (§1.3).
    pub distro: String,
    /// `location.path_bytes` — the lossless form the core opens.
    pub path_bytes: Vec<u8>,
    /// `location.path_key` — the comparison form, case-folded by the root's kind (R2).
    pub path_key: Vec<u8>,
    /// `location.path_display` — lossy, for the UI only (§1.3).
    pub path_display: String,
    /// `location.store_key` — the device or share the repository lives on (§4.7).
    pub store_key: String,
    /// **R27: `None` is not `""`.** No stable identifier exists for a bind mount, overlayfs or
    /// tmpfs, the column is nullable for exactly that reason, and a location with no volume key
    /// can never be recognised across a remount — inventing one hides that from the user.
    pub volume_key: Option<String>,
}

/// R31: `ScanMode` is declared in `protocol/schema/protocol.json` and generated into
/// `crate::protocol`. Re-exported so `scan::run::ScanMode` still names it. `as_str` stays as an
/// inherent impl on the generated type (same crate, so it is legal) because the generated enum
/// has no methods and `ScanRunStart` binds this string.
pub use crate::protocol::ScanMode;

impl ScanMode {
    /// The `scan_run.mode` text: `full` or `incremental`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Incremental => "incremental",
        }
    }
}

/// What one run did, as [`ScanRunner::finish`] reports it once the `scan_run` row is closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRunOutcome {
    /// The run's `scan_run` row.
    pub scan_run_id: i64,
    /// The generation the run stamped on the locations it saw.
    pub generation: i64,
    /// Directories visited across every root the run walked.
    pub walked_dirs: u64,
    /// `max(found this run, already indexed)` — see the module note.
    pub found_repos: u64,
    /// Projects already indexed when the run began: the floor `found_repos` never drops below.
    pub resumed_from: u64,
    /// True when the run stopped on its token; `presence` is then all zero.
    pub cancelled: bool,
    /// Store keys of every root the run walked, which presence reads as the stores reachable
    /// this run.
    pub present_stores: BTreeSet<String>,
    /// Roots whose first `metadata` call took two seconds or more — possibly dead network
    /// mounts, kept out of `timed_elapsed_ms`.
    pub isolated_roots: Vec<i64>,
    /// Symlinks and junctions the link policy refused, across every root.
    pub links_refused: u64,
    /// Wall time from `begin` to the end of the presence pass, in milliseconds.
    pub elapsed_ms: u64,
    /// `elapsed_ms` minus every isolated root's probe time — the figure a timing gate may use.
    pub timed_elapsed_ms: u64,
    /// What the presence pass decided; all zero for a cancelled run.
    pub presence: PresenceSummary,
}

/// Everything one scan run reaches, borrowed for the length of the run.
pub struct ScanRunner<'a> {
    /// The scanner's seam onto its own rows: generation, roots, `scan_run`, `scan_problem` and
    /// presence.
    pub store: &'a dyn ScanStore,
    /// Git, for discovery's `rev-parse` probe and the hand-off's reads.
    pub git: &'a dyn GitBackend,
    /// Resolves a path's device identities (§4.7); shared with each root's link policy.
    pub mounts: Arc<dyn MountResolver>,
    /// The unix-seconds clock the `scan_run` row and the hand-off are stamped with.
    pub clock: &'a dyn Clock,
    /// The exclusion list (§4.3) the walk and presence both apply.
    pub skip: &'a SkipList,
    /// The run's §4.8 token, polled by the walk and handed to every git probe it makes.
    pub cancel: &'a CancelToken,
    /// The one `rusqlite::Connection`, shared rather than owned (R39).
    ///
    /// The runner reaches the database two ways on purpose. `store` is plan 07's seam and owns
    /// the run's own rows; this is what `assembly::handoff` needs, because the `location` row
    /// must be written inside the transaction that decided its project and a trait method
    /// cannot carry one (R35a). Both are the same mutex over the same connection — never a
    /// second one.
    pub index: &'a Mutex<Index>,
    /// §13's dispatcher, or `None` on a host with no WSL.
    ///
    /// `None` is not a null object: on Linux there is no bridge to cross and no
    /// `WalkEvent::WslBridge` is ever emitted, so an absent dispatcher is never reached. On
    /// Windows an absent one means the build staged no worker, and the walk says so rather than
    /// reporting an empty distro.
    pub wsl: Option<&'a crate::wsl::dispatch::WslDispatcher>,
    /// Where a newly indexed location's jobs are queued (§4.1a).
    ///
    /// Called from [`ScanRunner::index_one`], with the store key and class from the **one**
    /// `MountFacts` `enrich` resolved: **R4** — there is no second resolver call and the queue
    /// never touches the filesystem.
    pub jobs: Arc<dyn crate::jobs::JobSink>,
}

impl ScanRunner<'_> {
    /// Install the sink that queues jobs for a newly indexed location.
    ///
    /// A runner built without one queues nothing, which is what keeps plan 07's own tests
    /// standing: `NullJobSink` is the absence of a scheduler, not a fake of one.
    pub fn set_job_sink(&mut self, sink: Arc<dyn crate::jobs::JobSink>) {
        self.jobs = sink;
    }
}

impl std::fmt::Debug for ScanRunner<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScanRunner").finish_non_exhaustive()
    }
}

/// One enabled root that is reachable and whose store is known.
struct Walkable<'a> {
    root: &'a ScanRootRow,
    path: std::path::PathBuf,
    facts: MountFacts,
}

/// A run whose `scan_run` row exists and whose walk has not started.
///
/// The split exists for `ScanLauncher`: it must return the run's id to `scan.start` immediately,
/// and the walk takes tens of seconds. Nothing else needs the two halves apart, so `run` still
/// does both.
#[derive(Debug)]
pub struct StartedScan {
    /// The `scan_run` row `begin` wrote.
    pub scan_run_id: i64,
    /// The generation `begin` took.
    pub generation: i64,
    /// Projects already indexed at `begin`: the floor for the run's `found_repos`.
    pub resumed_from: u64,
    /// Every `scan_root` row as `begin` read it, enabled or not.
    pub roots: Vec<ScanRootRow>,
    started: Instant,
}

impl StartedScan {
    /// The enabled roots, which is what `ScanRunStarted.roots` reports.
    #[must_use]
    pub fn enabled_root_ids(&self) -> Vec<i64> {
        self.roots
            .iter()
            .filter(|root| root.enabled)
            .map(|root| root.root_id)
            .collect()
    }
}

impl ScanRunner<'_> {
    /// Take a generation and write the `scan_run` row. Spawns nothing and walks nothing.
    ///
    /// # Errors
    ///
    /// When the store cannot issue a generation, count indexed projects, read the roots or
    /// insert the `scan_run` row.
    pub fn begin(&self, mode: ScanMode) -> Result<StartedScan, ScanStoreError> {
        let started = Instant::now();
        let generation = self.store.next_generation()?;
        let resumed_from = self.store.indexed_project_count()?;
        let roots = self.store.scan_roots()?;
        let scan_run_id = self.store.begin_scan_run(&ScanRunStart {
            generation,
            started_at: self.clock.now_unix(),
            mode: mode.as_str(),
            roots_json: roots_json(&roots),
        })?;
        Ok(StartedScan {
            scan_run_id,
            generation,
            resumed_from,
            roots,
            started,
        })
    }

    /// [`ScanRunner::begin`] then [`ScanRunner::finish`], both on the calling thread.
    ///
    /// # Errors
    ///
    /// Whatever `begin` or `finish` returns.
    pub fn run(
        &self,
        mode: ScanMode,
        sink: &WalkSink<'_>,
    ) -> Result<ScanRunOutcome, ScanStoreError> {
        let begun = self.begin(mode)?;
        self.finish(begun, sink)
    }

    /// Walk, classify, apply presence and close the run out.
    ///
    /// # Errors
    ///
    /// When the presence pass fails or the `scan_run` row cannot be closed. A problem row that
    /// cannot be written is logged and is never an error.
    pub fn finish(
        &self,
        begun: StartedScan,
        sink: &WalkSink<'_>,
    ) -> Result<ScanRunOutcome, ScanStoreError> {
        let StartedScan {
            scan_run_id,
            generation,
            resumed_from,
            roots,
            started,
        } = begun;

        let survey = self.survey_roots(&roots, scan_run_id, sink);
        let (walked_dirs, found_this_run, links_refused, cancelled) =
            self.walk_all(&survey, resumed_from, scan_run_id, generation, sink);

        let found_repos = found_this_run.max(resumed_from);

        // §4.6 applies "after a run completes". A cancelled run did not visit everything, so
        // running it would mark every unvisited location missing.
        let presence = if cancelled {
            PresenceSummary::default()
        } else {
            apply_presence(
                self.store,
                &PresenceContext {
                    generation,
                    roots: &roots,
                    skip: self.skip,
                    present_stores: &survey.present_stores,
                },
            )?
        };

        let elapsed_ms = millis(started.elapsed());
        self.store.finish_scan_run(
            scan_run_id,
            &ScanRunFinish {
                ended_at: self.clock.now_unix(),
                walked_dirs,
                found_repos,
                cancelled,
            },
        )?;

        Ok(ScanRunOutcome {
            scan_run_id,
            generation,
            walked_dirs,
            found_repos,
            resumed_from,
            cancelled,
            present_stores: survey.present_stores,
            isolated_roots: survey.isolated_roots,
            links_refused,
            elapsed_ms,
            timed_elapsed_ms: elapsed_ms.saturating_sub(survey.isolated_ms),
            presence,
        })
    }

    /// Pass one: which enabled roots are reachable, and on which stores.
    ///
    /// A root that is reachable but whose mount cannot be resolved is **not** walked. Without a
    /// `store_key` nothing found under it could be written — `location.store_key` is `NOT NULL`
    /// and R27 forbids substituting a default — so walking it would produce discoveries plan 08
    /// has to drop, which is worse than reporting the root once.
    fn survey_roots<'r>(
        &self,
        roots: &'r [ScanRootRow],
        scan_run_id: i64,
        sink: &WalkSink<'_>,
    ) -> Survey<'r> {
        /// An enabled root lying under another enabled root of the same kind and distro is
        /// redundant: the covering root's walk reaches the same directories, so walking both
        /// costs the subtree twice and discovers every repository under it twice.
        ///
        /// It is reachable because `firstrun::roots` guards one direction only — it asks whether
        /// a *new* path sits under an existing root, never whether it is an *ancestor* of one —
        /// and §10.1a's list sorts by hit count, which can render an ancestor below its own
        /// descendants and invite exactly that order. The redundant row is left in `scan_root`:
        /// the user chose it, `presence` still reads it, and removing it here would be a write
        /// this pass has no mandate for.
        fn is_covered(root: &ScanRootRow, roots: &[ScanRootRow]) -> bool {
            roots.iter().any(|other| {
                if !other.enabled
                    || other.root_id == root.root_id
                    || other.kind != root.kind
                    || other.distro != root.distro
                {
                    return false;
                }
                if other.path_key == root.path_key {
                    // Exact duplicates cover each other; keep the lower id so one of the pair walks.
                    return other.root_id < root.root_id;
                }
                crate::paths::is_under(&root.path_key, &other.path_key)
            })
        }

        let mut survey = Survey::default();
        for root in roots
            .iter()
            .filter(|root| root.enabled && !is_covered(root, roots))
        {
            let path = path_from_bytes(&root.path_bytes);
            let probe = Instant::now();
            let reachable = std::fs::metadata(&path).is_ok();
            let took = probe.elapsed();
            if took >= ROOT_PROBE_SLOW {
                survey.isolated_roots.push(root.root_id);
                survey.isolated_ms = survey.isolated_ms.saturating_add(millis(took));
            }
            if !reachable {
                self.report(
                    scan_run_id,
                    ScanProblem {
                        kind: ScanProblemKind::OfflineStore,
                        path_display: path_display(&path),
                        detail: "scan root not reachable this generation".to_owned(),
                    },
                    sink,
                );
                continue;
            }
            match self.mounts.resolve(&path) {
                Ok(facts) => {
                    survey.present_stores.insert(facts.store_key.clone());
                    survey.walkable.push(Walkable { root, path, facts });
                }
                Err(err) => self.report(
                    scan_run_id,
                    ScanProblem {
                        kind: ScanProblemKind::OfflineStore,
                        path_display: path_display(&path),
                        detail: format!("store identity unavailable: {err}"),
                    },
                    sink,
                ),
            }
        }
        survey
    }

    /// Pass two: the walk. One set of root keys for the whole run, so a link arriving at a
    /// directory a *different* root already reached is still a duplicate.
    fn walk_all(
        &self,
        survey: &Survey<'_>,
        resumed_from: u64,
        scan_run_id: i64,
        generation: i64,
        sink: &WalkSink<'_>,
    ) -> (u64, u64, u64, bool) {
        let link_roots: Vec<Vec<u8>> = survey
            .walkable
            .iter()
            .map(|w| w.root.path_key.clone())
            .collect();
        let mut walked_dirs = 0_u64;
        let found_this_run = AtomicU64::new(0);
        let mut links_refused = 0_u64;
        let mut cancelled = self.cancel.is_cancelled();

        for entry in &survey.walkable {
            if self.cancel.is_cancelled() {
                cancelled = true;
                break;
            }
            let opts = WalkOptions {
                descend_into_repos: entry.root.descend_into_repos,
                // §4.2: bare candidates are only tested under explicitly enabled roots, and
                // every root reaching here is one.
                bare_candidates: true,
                ..WalkOptions::default()
            };
            let probe = ProbeCtx::new(
                self.git,
                StoreKey::new(entry.facts.store_key.clone()),
                entry.facts.class,
                self.cancel,
            );
            let ctx = WalkCtx {
                opts: &opts,
                skip: self.skip,
                probe: &probe,
                links: Arc::new(LinkPolicy::new(
                    opts.follow_links,
                    link_roots.clone(),
                    survey.present_stores.clone(),
                    Arc::clone(&self.mounts),
                )),
            };

            let stats = walk_root(&entry.path, &ctx, &|event| match event {
                WalkEvent::Repo(candidate) => {
                    let discovered = self.enrich(candidate, entry);
                    if discovered.candidate.path.to_str().is_none() {
                        // §11.1's non-UTF-8 group. The repository indexes normally — criterion 5
                        // requires it — and this row is what tells the user the path it is shown
                        // is the lossy one.
                        self.report(
                            scan_run_id,
                            ScanProblem {
                                kind: ScanProblemKind::NonUtf8Path,
                                path_display: discovered.path_display.clone(),
                                detail: "path is not valid UTF-8; the displayed form is lossy"
                                    .to_owned(),
                            },
                            sink,
                        );
                    }
                    let seen = found_this_run.fetch_add(1, Ordering::Relaxed) + 1;
                    self.index_one(
                        &discovered,
                        entry.facts.class,
                        generation,
                        scan_run_id,
                        sink,
                    );
                    sink(WalkEvent::Discovered(Box::new(discovered)));
                    sink(WalkEvent::Progress {
                        walked_dirs: 0,
                        found_repos: seen.max(resumed_from),
                    });
                }
                WalkEvent::Walked { dirs } => {
                    sink(WalkEvent::Progress {
                        walked_dirs: dirs,
                        found_repos: found_this_run.load(Ordering::Relaxed).max(resumed_from),
                    });
                }
                WalkEvent::Problem(problem) => self.report(scan_run_id, problem, sink),
                // §4.5: the walk registers the bridge and refuses to traverse it; §13 crosses it
                // over the protocol instead. Everything the distro reports comes back through
                // this same sink, so an in-distro repository is indexed by the arm above.
                WalkEvent::WslBridge {
                    distro,
                    path_display,
                } => self.cross_the_bridge(
                    &distro,
                    &path_display,
                    entry.root.root_id,
                    &opts,
                    scan_run_id,
                    generation,
                    sink,
                ),
                other => sink(other),
            });

            walked_dirs = walked_dirs.saturating_add(stats.walked_dirs);
            links_refused = links_refused.saturating_add(stats.links_refused);
            cancelled |= stats.cancelled;
        }

        (
            walked_dirs,
            found_this_run.load(Ordering::Relaxed),
            links_refused,
            cancelled,
        )
    }

    /// §13: hand one bridge path to the in-distro worker.
    ///
    /// **A distro with no consent is not scanned and is not an error.** The dispatcher answers
    /// `Unreachable { NeedsConsent }` and emits its own problem row saying which distro is
    /// waiting on the user — the difference between *the user has not opted in* and *the scan
    /// failed* has to survive to the surface, and it is that row that carries it.
    ///
    /// A build with no dispatcher records the bridge instead of ignoring it. Silence here would
    /// report a distro's repositories as absent rather than unscanned.
    /// Public so a host with **no** bridge to walk can still drive this arm. `\\wsl$\` paths
    /// exist only on Windows, so a Linux test can never reach it through `walk_root` — and an
    /// arm no test can reach is how the whole in-distro subsystem stayed unwired.
    #[allow(clippy::too_many_arguments)]
    pub fn cross_the_bridge(
        &self,
        distro: &str,
        path_display: &str,
        root_id: i64,
        opts: &WalkOptions,
        scan_run_id: i64,
        generation: i64,
        sink: &WalkSink<'_>,
    ) {
        let Some(wsl) = self.wsl else {
            self.report(
                scan_run_id,
                ScanProblem {
                    kind: ScanProblemKind::OfflineStore,
                    path_display: path_display.to_owned(),
                    detail: format!(
                        "{distro}: this build has no in-distro worker, so the bridge was not                          crossed"
                    ),
                },
                sink,
            );
            return;
        };
        // The bridge path carries the Linux path the distro knows this directory by. Parsing it
        // here rather than re-deriving one keeps `wsl::path` the single owner of that mapping.
        let Some(parsed) = crate::wsl::path::parse_bridge_path(path_display) else {
            self.report(
                scan_run_id,
                ScanProblem {
                    kind: ScanProblemKind::UnreadableRepo,
                    path_display: path_display.to_owned(),
                    detail: format!("{distro}: not a bridge path this build understands"),
                },
                sink,
            );
            return;
        };
        let generation_for_walk = generation;
        let outcome = wsl.scan(
            &crate::scan::wsl::WslBridgeRef {
                distro: distro.to_owned(),
            },
            &parsed.linux_path,
            root_id,
            opts,
            &|event| match event {
                WalkEvent::Repo(_) | WalkEvent::WslBridge { .. } => {}
                WalkEvent::Discovered(discovered) => {
                    // The distro's own store class: `store_key` came back from the worker's
                    // mount table, and nothing on this host can resolve a path inside a distro.
                    self.index_one(
                        &discovered,
                        StoreClass::Unknown,
                        generation_for_walk,
                        scan_run_id,
                        sink,
                    );
                    sink(WalkEvent::Discovered(discovered));
                }
                WalkEvent::Problem(problem) => self.report(scan_run_id, problem, sink),
                other => sink(other),
            },
        );
        if let Some(kind) = crate::wsl::dispatch::error_kind_for(&outcome) {
            // §11.5 for a distro that has no git of its own. Its repositories were found; they
            // just cannot be read, and that is a different sentence from "not found".
            self.report(
                scan_run_id,
                ScanProblem {
                    kind: ScanProblemKind::UnreadableRepo,
                    path_display: path_display.to_owned(),
                    detail: format!("{distro}: {kind}"),
                },
                sink,
            );
        }
    }

    /// §4.1a: the row, then the queue.
    ///
    /// This is the step the module's own doc comment used to describe as the gap. Every git read
    /// inside `hand_off_discovered` happens with the index lock free (**R39**), which is what
    /// lets this run from the walk's worker threads at all.
    ///
    /// A repository that could not be indexed becomes a `scan_problem`, never a silent skip: a
    /// scan that quietly drops what it could not read claims a completeness the walk did not
    /// have. A **cancelled** run is the one exception — it reports nothing, because a token that
    /// fired once would otherwise write one row per remaining repository, none of which is a
    /// problem with the repository.
    fn index_one(
        &self,
        discovered: &Discovered,
        store_class: StoreClass,
        generation: i64,
        scan_run_id: i64,
        sink: &WalkSink<'_>,
    ) {
        let ctx = HandoffCtx {
            git: self.git,
            cancel: self.cancel,
            store_class,
            generation,
            now: self.clock.now_unix(),
        };
        match hand_off_discovered(self.index, &ctx, discovered) {
            Ok(indexed) => self.jobs.on_location_indexed(
                indexed.project,
                indexed.location,
                &discovered.store_key,
                store_class,
            ),
            Err(err) => {
                if self.cancel.is_cancelled() {
                    return;
                }
                self.report(
                    scan_run_id,
                    ScanProblem {
                        kind: problem_kind_for(&err),
                        path_display: discovered.path_display.clone(),
                        detail: format!("could not be indexed: {err}"),
                    },
                    sink,
                );
            }
        }
    }

    /// Every problem reaches `scan_problem` as well as the subscriber — §11.1's summary reads the
    /// table, not the event stream. A failed write is logged and never fatal: a problem row is a
    /// diagnostic, and losing one must not abort a scan that is otherwise succeeding.
    fn report(&self, scan_run_id: i64, problem: ScanProblem, sink: &WalkSink<'_>) {
        if let Err(err) = self.store.record_problem(scan_run_id, &problem) {
            eprintln!("scan: could not record problem: {err}");
        }
        sink(WalkEvent::Problem(problem));
    }

    /// Give a candidate its two device identities (§4.7).
    ///
    /// The candidate's own mount is asked for first, because a repository may sit on a mount
    /// nested inside the root's; the root's facts are the fallback, and they are a *measured*
    /// value rather than a default — R27's objection is to inventing an identifier, not to
    /// reusing the one the enclosing root resolved a moment ago.
    fn enrich(&self, candidate: RepoCandidate, entry: &Walkable<'_>) -> Discovered {
        let path: &Path = &candidate.path;
        let facts = self
            .mounts
            .resolve(path)
            .unwrap_or_else(|_| entry.facts.clone());
        Discovered {
            root_id: entry.root.root_id,
            kind: entry.root.kind.clone(),
            distro: entry.root.distro.clone(),
            path_bytes: path_bytes(path),
            path_key: path_key(path, platform_of(&entry.root.kind)),
            path_display: path_display(path),
            store_key: facts.store_key,
            volume_key: facts.volume_key,
            candidate,
        }
    }
}

/// What pass one learned. `walkable` borrows the roots it describes.
#[derive(Default)]
struct Survey<'a> {
    walkable: Vec<Walkable<'a>>,
    present_stores: BTreeSet<String>,
    isolated_roots: Vec<i64>,
    isolated_ms: u64,
}

/// R2: the root's `kind` decides case-folding, not the host.
///
/// `win` is the only case-insensitive one; a `linux` or `wsl` root holds paths where `Repo` and
/// `repo` are two directories, and folding them together on a Windows host would merge two
/// locations into one row.
#[must_use]
pub fn platform_of(kind: &str) -> PathPlatform {
    if kind == "win" {
        PathPlatform::Windows
    } else {
        PathPlatform::Unix
    }
}

/// §11.1's group for a repository the hand-off could not index.
///
/// The three that have their own group keep it: an untrusted repository is fixed by trusting it
/// and a permission error by changing a mode, and filing either under *unreadable* would offer
/// the user no action. Everything else is `unreadable_repo`.
const fn problem_kind_for(err: &HandoffError) -> ScanProblemKind {
    match err {
        HandoffError::Git(GitError::Untrusted { .. }) => ScanProblemKind::UntrustedRepo,
        HandoffError::Git(GitError::PermissionDenied { .. }) => ScanProblemKind::PermissionDenied,
        HandoffError::Git(GitError::StoreOffline { .. }) => ScanProblemKind::OfflineStore,
        _ => ScanProblemKind::UnreadableRepo,
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// `scan_run.roots_json` (§1.9): the root ids and their enabled state, so a run can be read back
/// against the roots it actually had. Paths are not included — `scan_root` holds them.
fn roots_json(roots: &[ScanRootRow]) -> String {
    let body = roots
        .iter()
        .map(|r| format!("{{\"id\":{},\"enabled\":{}}}", r.root_id, r.enabled))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{body}]")
}
