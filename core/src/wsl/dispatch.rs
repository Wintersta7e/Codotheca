//! §13 and §4.5 — what happens after the Windows walk refuses the bridge.
//!
//! Consent gates *starting* a distro, not reading one that is already up: attaching to a running
//! distro costs nothing and surprises nobody, while starting a stopped one spins up a virtual
//! machine. And a distro that was not read contributes no store keys, which is how §4.6 reaches
//! `offline` — frozen, never `missing`, never deleted. Absent is not abandoned.

use crate::index::path::PathPlatform;
use crate::paths::{path_bytes, path_key};
use crate::scan::discover::{RepoCandidate, RepoKind};
use crate::scan::run::Discovered;
use crate::scan::wsl::WslBridgeRef;
use crate::scan::{ScanProblem, ScanProblemKind, WalkEvent, WalkOptions, WalkSink};
use crate::wsl::conn::WslWorkerPool;
use crate::wsl::distros::{DistroInfo, DistroState};
use crate::wsl::path::bridge_path;
use crate::wsl::proto::{WalkRequest, WorkerEvent, WorkerGit, WorkerRepoFound, WorkerRequest};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// The `app_meta` key holding the distros the user has agreed may be started.
///
/// **Deviation, ruled by the user:** the consent set lives as a one-key JSON list in the
/// existing key-value table. The plan's open question named a `settings` table; the schema has
/// none — `core/migrations/0001_meta_and_projects.sql:4` declares `app_meta (k, v)` and `:15`
/// declares `view_state (k, v)`, and those are the only two. `app_meta` is the right half:
/// `view_state` is §1.9's query, sort, scroll and geometry, and a standing permission to start
/// a virtual machine is not view state.
pub const CONSENTED_DISTROS_KEY: &str = "wsl_consented_distros";

/// The consent set as stored. An unreadable or malformed value yields an **empty** set, which
/// consents to nothing: the failure mode of this read has to be "ask again", never "start it".
#[must_use]
pub fn read_consented(conn: &rusqlite::Connection) -> BTreeSet<String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT v FROM app_meta WHERE k = ?1",
            rusqlite::params![CONSENTED_DISTROS_KEY],
            |row| row.get(0),
        )
        .ok();
    raw.and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .map(|names| names.into_iter().collect())
        .unwrap_or_default()
}

/// Replaces the consent set. Takes the transaction from its caller, as every writer here does.
pub fn write_consented(
    tx: &rusqlite::Transaction<'_>,
    consented: &BTreeSet<String>,
) -> Result<(), rusqlite::Error> {
    let names: Vec<&String> = consented.iter().collect();
    let text = serde_json::to_string(&names).map_err(|e| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(e.to_string())))
    })?;
    tx.execute(
        "INSERT INTO app_meta (k, v) VALUES (?1, ?2)
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        rusqlite::params![CONSENTED_DISTROS_KEY, text],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistroReadiness {
    Ready,
    /// Installed and stopped, and the user has not agreed to start it.
    NeedsConsent,
    /// Not installed. Distinct from stopped: there is nothing to consent to.
    Absent,
}

#[must_use]
pub fn readiness(installed: &[DistroInfo], distro: &str, consented: bool) -> DistroReadiness {
    match installed.iter().find(|d| d.name == distro) {
        None => DistroReadiness::Absent,
        Some(info) if info.state == DistroState::Running => DistroReadiness::Ready,
        Some(_) if consented => DistroReadiness::Ready,
        Some(_) => DistroReadiness::NeedsConsent,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    NeedsConsent,
    Absent,
    Launch { detail: String },
}

#[derive(Debug)]
pub enum DistroOutcome {
    Scanned {
        distro: String,
        store_keys: BTreeSet<String>,
        repos: u64,
        /// §13: the distro has no git, so its repositories carry an explained error.
        git_missing: bool,
    },
    Unreachable {
        distro: String,
        reason: Unavailable,
    },
}

/// The scan summary's record of a distro that was not read. `offline_store` is the honest kind:
/// the store was not present this generation, which is not the same as gone.
#[must_use]
pub fn problem_for(distro: &str, reason: &Unavailable) -> ScanProblem {
    let detail = match reason {
        Unavailable::NeedsConsent => "the distro is stopped and has not been allowed to start",
        Unavailable::Absent => "the distro is not installed",
        Unavailable::Launch { detail } => detail.as_str(),
    };
    ScanProblem {
        kind: ScanProblemKind::OfflineStore,
        path_display: bridge_path(distro, "/"),
        detail: detail.to_owned(),
    }
}

/// Only a distro that was actually read contributes store keys. Everything withheld here becomes
/// `Presence::Offline` in plan 07's presence pass, which is exactly what §4.6 asks for.
#[must_use]
pub fn present_store_keys(outcomes: &[DistroOutcome]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for outcome in outcomes {
        if let DistroOutcome::Scanned { store_keys, .. } = outcome {
            out.extend(store_keys.iter().cloned());
        }
    }
    out
}

/// §11.5's `project.error_kind`. Only the git case sets one: an unreachable distro's projects
/// were never observed this generation, and a never-succeeded error would claim otherwise.
#[must_use]
pub fn error_kind_for(outcome: &DistroOutcome) -> Option<&'static str> {
    match outcome {
        DistroOutcome::Scanned {
            git_missing: true, ..
        } => Some("GIT_MISSING"),
        _ => None,
    }
}

/// The same row shape the native walk produces, so nothing downstream branches on `wsl`. This
/// function only builds the event; the `location` row is written from it by plan 08's
/// `upsert_location`, reached through plan 07's `ScanStore::upsert_location` inside the same
/// transaction `resolve_identity` ran in (R1).
///
/// The path is keyed as **Unix**: two Linux paths differing only in case are two repositories,
/// and `UNIQUE(kind, distro, path_key)` must not fold them together on a Windows host.
#[must_use]
pub fn discovered_of(found: &WorkerRepoFound, distro: &str, root_id: i64) -> Option<Discovered> {
    // R7 put the inverse of `as_str` on `RepoKind` itself; an unrecognised slug is dropped
    // rather than guessed, because every kind decides a different set of later reads.
    let kind = RepoKind::from_str(&found.kind)?;
    let path = std::path::PathBuf::from(&found.work_dir);
    Some(Discovered {
        candidate: RepoCandidate {
            path: path.clone(),
            kind,
            git_dir: std::path::PathBuf::from(&found.git_dir),
            common_dir: std::path::PathBuf::from(&found.common_dir),
        },
        root_id,
        kind: "wsl".to_owned(),
        distro: distro.to_owned(),
        path_bytes: path_bytes(&path),
        path_key: path_key(&path, PathPlatform::Unix),
        // §8.5: the Locations row carries `WSL · <distro>` as a chip, so the path itself is the
        // Linux path. The bridge form is display-only and is never stored.
        path_display: found.work_dir.clone(),
        store_key: found.store_key.clone(),
        // R27: `None` is not `""`. A mount with no stable identifier can never be recognised
        // across a remount, and inventing a key hides that.
        volume_key: found.volume_key.clone(),
    })
}

#[derive(Debug)]
pub struct WslDispatcher {
    pool: Arc<WslWorkerPool>,
    installed: Vec<DistroInfo>,
    consented: BTreeSet<String>,
}

impl WslDispatcher {
    #[must_use]
    pub fn new(
        pool: Arc<WslWorkerPool>,
        installed: Vec<DistroInfo>,
        consented: BTreeSet<String>,
    ) -> WslDispatcher {
        WslDispatcher {
            pool,
            installed,
            consented,
        }
    }

    /// Dispatches the in-distro walk §4.5 refused to do from Windows, and streams what it finds
    /// into the same sink the native walk uses.
    pub fn scan(
        &self,
        bridge: &WslBridgeRef,
        linux_root: &str,
        root_id: i64,
        opts: &WalkOptions,
        sink: &WalkSink<'_>,
    ) -> DistroOutcome {
        let distro = bridge.distro.clone();
        let consented = self.consented.contains(&distro);
        let reason = match readiness(&self.installed, &distro, consented) {
            DistroReadiness::Ready => None,
            DistroReadiness::NeedsConsent => Some(Unavailable::NeedsConsent),
            DistroReadiness::Absent => Some(Unavailable::Absent),
        };
        if let Some(reason) = reason {
            sink(WalkEvent::Problem(problem_for(&distro, &reason)));
            return DistroOutcome::Unreachable { distro, reason };
        }

        let worker = match self.pool.get(&distro) {
            Ok(worker) => worker,
            Err(err) => {
                let reason = Unavailable::Launch {
                    detail: err.to_string(),
                };
                sink(WalkEvent::Problem(problem_for(&distro, &reason)));
                return DistroOutcome::Unreachable { distro, reason };
            }
        };
        // §13: no git in the distro is an explained error on its repositories, not a reason to
        // stop finding them.
        let git_missing = matches!(worker.git(), WorkerGit::Missing { .. });

        let store_keys = std::sync::Mutex::new(BTreeSet::new());
        let repos = AtomicU64::new(0);
        let dropped = AtomicBool::new(false);
        let request = WorkerRequest::Walk(WalkRequest {
            root: linux_root.to_owned(),
            follow_links: opts.follow_links,
            descend_into_repos: opts.descend_into_repos,
            bare_candidates: opts.bare_candidates,
            skip_extra: Vec::new(),
        });
        let outcome = worker.stream(&request, &mut |event| match event {
            WorkerEvent::Repo(found) => {
                if let Ok(mut keys) = store_keys.lock() {
                    keys.insert(found.store_key.clone());
                }
                if let Some(row) = discovered_of(&found, &distro, root_id) {
                    repos.fetch_add(1, Ordering::Relaxed);
                    sink(WalkEvent::Discovered(Box::new(row)));
                } else {
                    // A kind this build does not know is reported, not counted and not guessed:
                    // silently dropping it would make the summary claim a completeness the walk
                    // did not have.
                    dropped.store(true, Ordering::Relaxed);
                    sink(WalkEvent::Problem(ScanProblem {
                        kind: ScanProblemKind::UnreadableRepo,
                        path_display: found.work_dir.clone(),
                        detail: format!("unrecognised repository kind {:?}", found.kind),
                    }));
                }
            }
            WorkerEvent::Problem {
                kind,
                path_display,
                detail,
            } => {
                sink(WalkEvent::Problem(ScanProblem {
                    kind: problem_kind_of(&kind),
                    path_display,
                    detail,
                }));
            }
            WorkerEvent::Progress {
                walked_dirs,
                found_repos,
            } => {
                sink(WalkEvent::Progress {
                    walked_dirs,
                    found_repos,
                });
            }
            WorkerEvent::SkippedMount {
                mount_point,
                fstype,
            } => {
                sink(WalkEvent::Problem(ScanProblem {
                    kind: ScanProblemKind::OfflineStore,
                    path_display: mount_point,
                    detail: format!("{fstype} is a Windows volume; the native scan covers it"),
                }));
            }
        });

        if let Err(err) = outcome {
            let reason = Unavailable::Launch {
                detail: err.to_string(),
            };
            sink(WalkEvent::Problem(problem_for(&distro, &reason)));
            // A partial walk's rows stay: they are timestamped observations, which is what §4.8
            // says every partial result is. But the store is not claimed present.
            return DistroOutcome::Unreachable { distro, reason };
        }

        DistroOutcome::Scanned {
            distro,
            store_keys: store_keys.into_inner().unwrap_or_default(),
            repos: repos.load(Ordering::Relaxed),
            git_missing,
        }
    }
}

fn problem_kind_of(slug: &str) -> ScanProblemKind {
    match slug {
        "permission_denied" => ScanProblemKind::PermissionDenied,
        "untrusted_repo" => ScanProblemKind::UntrustedRepo,
        "clock_skew" => ScanProblemKind::ClockSkew,
        "non_utf8_path" => ScanProblemKind::NonUtf8Path,
        "offline_store" => ScanProblemKind::OfflineStore,
        _ => ScanProblemKind::UnreadableRepo,
    }
}

#[cfg(all(test, feature = "testkit"))]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{
        discovered_of, error_kind_for, present_store_keys, problem_for, read_consented, readiness,
        write_consented, DistroOutcome, DistroReadiness, Unavailable, WslDispatcher,
    };
    use crate::scan::wsl::WslBridgeRef;
    use crate::scan::{ScanProblemKind, WalkEvent, WalkOptions};
    use crate::testing::wsl::LoopbackLauncher;
    use crate::testing::{FakeGitBackend, TempIndex};
    use crate::wsl::conn::WslWorkerPool;
    use crate::wsl::distros::{DistroInfo, DistroState};
    use crate::wsl::mounts::MountTable;
    use crate::wsl::proto::{WorkerGit, WorkerRepoFound};
    use crate::wsl::serve::WorkerContext;
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};

    const MOUNTINFO: &str = "28 1 8:32 / / rw - ext4 /dev/sdc rw\n";

    fn installed() -> Vec<DistroInfo> {
        vec![
            DistroInfo {
                name: "up".to_owned(),
                state: DistroState::Running,
            },
            DistroInfo {
                name: "down".to_owned(),
                state: DistroState::Stopped,
            },
        ]
    }

    fn launcher(presence: WorkerGit) -> Arc<LoopbackLauncher> {
        Arc::new(LoopbackLauncher::new(move |distro: &str| WorkerContext {
            distro: distro.to_owned(),
            git: Box::new(FakeGitBackend::new()),
            mounts: MountTable::from_mountinfo(MOUNTINFO),
            presence: presence.clone(),
        }))
    }

    fn found(work_dir: &str, kind: &str) -> WorkerRepoFound {
        WorkerRepoFound {
            work_dir: work_dir.to_owned(),
            git_dir: format!("{work_dir}/.git"),
            common_dir: format!("{work_dir}/.git"),
            kind: kind.to_owned(),
            store_key: "wsl:up:/".to_owned(),
            volume_key: Some("wsl-distro:up:/".to_owned()),
            store_class: "local".to_owned(),
        }
    }

    #[test]
    fn a_running_distro_needs_no_consent_because_attaching_starts_nothing() {
        assert_eq!(readiness(&installed(), "up", false), DistroReadiness::Ready);
        assert_eq!(readiness(&installed(), "up", true), DistroReadiness::Ready);
    }

    #[test]
    fn a_stopped_distro_needs_consent_because_starting_one_is_an_act() {
        assert_eq!(
            readiness(&installed(), "down", false),
            DistroReadiness::NeedsConsent
        );
        assert_eq!(
            readiness(&installed(), "down", true),
            DistroReadiness::Ready
        );
        assert_eq!(
            readiness(&installed(), "ghost", true),
            DistroReadiness::Absent
        );
    }

    #[test]
    fn a_stopped_unconsented_distro_is_never_launched() {
        // §13: never automatic during first run.
        let l = launcher(WorkerGit::Present {
            version: "2.43.0".to_owned(),
        });
        let dispatcher = WslDispatcher::new(
            Arc::new(WslWorkerPool::new(l.clone())),
            installed(),
            BTreeSet::new(),
        );
        let events: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let sink = |event: WalkEvent| {
            if let WalkEvent::Problem(p) = event {
                if let Ok(mut seen) = events.lock() {
                    seen.push(p.kind.as_str().to_owned());
                }
            }
        };
        let outcome = dispatcher.scan(
            &WslBridgeRef {
                distro: "down".to_owned(),
            },
            "/home/me",
            1,
            &WalkOptions::default(),
            &sink,
        );
        assert!(
            l.launched().is_empty(),
            "a stopped distro must not be started"
        );
        assert!(matches!(
            outcome,
            DistroOutcome::Unreachable {
                reason: Unavailable::NeedsConsent,
                ..
            }
        ));
        assert_eq!(events.lock().expect("lock").as_slice(), ["offline_store"]);
    }

    #[test]
    fn an_unreachable_distro_contributes_no_store_and_no_error_kind() {
        // Absent is not abandoned: withholding the store key is how §4.6 reaches `offline`, and
        // nothing here may say `missing` or `PATH_GONE`.
        let outcomes = [
            DistroOutcome::Unreachable {
                distro: "down".to_owned(),
                reason: Unavailable::NeedsConsent,
            },
            DistroOutcome::Unreachable {
                distro: "ghost".to_owned(),
                reason: Unavailable::Absent,
            },
        ];
        assert!(present_store_keys(&outcomes).is_empty());
        for outcome in &outcomes {
            assert_eq!(error_kind_for(outcome), None);
        }
        assert_eq!(
            problem_for("down", &Unavailable::NeedsConsent).kind,
            ScanProblemKind::OfflineStore
        );
    }

    #[test]
    fn a_distro_without_git_still_discovers_and_is_marked_git_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("p/.git")).expect("mkdir");
        let l = launcher(WorkerGit::Missing {
            detail: "not on PATH".to_owned(),
        });
        let dispatcher = WslDispatcher::new(
            Arc::new(WslWorkerPool::new(l)),
            vec![DistroInfo {
                name: "up".to_owned(),
                state: DistroState::Running,
            }],
            BTreeSet::new(),
        );
        let seen: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let sink = |event: WalkEvent| {
            if let WalkEvent::Discovered(d) = event {
                if let Ok(mut rows) = seen.lock() {
                    rows.push(d.path_display.clone());
                }
            }
        };
        let outcome = dispatcher.scan(
            &WslBridgeRef {
                distro: "up".to_owned(),
            },
            &dir.path().to_string_lossy(),
            7,
            &WalkOptions::default(),
            &sink,
        );
        match outcome {
            DistroOutcome::Scanned {
                repos,
                git_missing,
                ref store_keys,
                ..
            } => {
                assert_eq!(repos, 1);
                assert!(git_missing);
                // See `wsl::serve`'s walk test: the table maps Linux paths, so on a Windows
                // host the temporary root resolves to the honest `wsl:up:?` and the store key
                // can only be asserted where the root is a path the table can cover.
                #[cfg(unix)]
                assert!(store_keys.contains("wsl:up:/"));
                assert_eq!(store_keys.len(), 1, "one root, one store");
            }
            ref other @ DistroOutcome::Unreachable { .. } => panic!("outcome was {other:?}"),
        }
        assert_eq!(seen.lock().expect("lock").len(), 1);
        assert_eq!(error_kind_for(&outcome), Some("GIT_MISSING"));
    }

    #[test]
    fn a_discovered_row_stores_the_linux_path_and_keys_it_case_sensitively() {
        // §8.5: the Locations row carries `WSL · <distro>` as a chip, so `path_display` is the
        // Linux path. §4bis.4 makes the bridge form display-only, never stored.
        let lower = discovered_of(&found("/home/me/project", "worktree"), "up", 7).expect("builds");
        let upper = discovered_of(&found("/home/me/Project", "worktree"), "up", 7).expect("builds");

        assert_eq!(lower.kind, "wsl");
        assert_eq!(lower.distro, "up");
        assert_eq!(lower.path_display, "/home/me/project");
        assert!(!lower.path_display.contains("wsl.localhost"));
        assert_eq!(lower.root_id, 7);
        assert_eq!(lower.volume_key.as_deref(), Some("wsl-distro:up:/"));
        // Two Linux paths differing only in case are two repositories, and
        // UNIQUE(kind, distro, path_key) must not merge them.
        assert_ne!(lower.path_key, upper.path_key);
    }

    #[test]
    fn a_mount_with_no_stable_identity_stores_no_volume_key_rather_than_an_empty_one() {
        // R27: `None` is not `""`. A location that can never be recognised across a remount has
        // to say so, and a fabricated key would claim it can.
        let mut raw = found("/home/me/project", "worktree");
        raw.volume_key = None;
        let row = discovered_of(&raw, "up", 7).expect("builds");
        assert_eq!(row.volume_key, None);
    }

    #[test]
    fn an_unrecognised_repository_kind_is_dropped_rather_than_guessed() {
        assert!(discovered_of(&found("/home/me/p", "nonsense"), "up", 7).is_none());
    }

    #[test]
    fn the_consent_set_round_trips_and_an_absent_key_consents_to_nothing() {
        // The failure mode of this read has to be "ask again", never "start it".
        let mut index = TempIndex::new();
        assert!(read_consented(index.index().conn()).is_empty());

        let wanted: BTreeSet<String> = ["alpha".to_owned(), "beta".to_owned()]
            .into_iter()
            .collect();
        let tx = index.index_mut().conn_mut().transaction().expect("begins");
        write_consented(&tx, &wanted).expect("writes");
        tx.commit().expect("commits");
        assert_eq!(read_consented(index.index().conn()), wanted);

        // Replacing the set withdraws consent for anything not in it.
        let only_beta: BTreeSet<String> = ["beta".to_owned()].into_iter().collect();
        let tx = index.index_mut().conn_mut().transaction().expect("begins");
        write_consented(&tx, &only_beta).expect("writes");
        tx.commit().expect("commits");
        assert_eq!(read_consented(index.index().conn()), only_beta);

        index
            .index()
            .conn()
            .execute(
                "UPDATE app_meta SET v = 'not json' WHERE k = ?1",
                rusqlite::params![super::CONSENTED_DISTROS_KEY],
            )
            .expect("corrupts");
        assert!(
            read_consented(index.index().conn()).is_empty(),
            "a bad value consents to nothing"
        );
    }
}
