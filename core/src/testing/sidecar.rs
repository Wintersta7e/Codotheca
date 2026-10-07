//! Fixtures for the sidecar's registered sections: one per section, so every registered section
//! can be populated by a test — and the indexes the core cannot open, one per startup report.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use rusqlite::Transaction;

use crate::assembly::handoff::{hand_off_discovered, HandoffCtx, Indexed};
use crate::cancel::CancelToken;
use crate::clock::SystemClock;
use crate::completion::evaluate::CheckRow;
use crate::corpus::{write_file, CorpusGit};
use crate::git::{ensure_empty_hooks_dir, GitBackend, GitExec, GitSlots, SystemGit};
use crate::index::migrate::SUPPORTED_SCHEMA_VERSION;
use crate::index::subject::subject_for_project;
use crate::index::{open_connection, Index, IndexError};
use crate::jobs::NullJobSink;
use crate::mount::{MountResolver as _, SystemMountResolver};
use crate::paths::{path_bytes, path_display, path_key};
use crate::protocol::ScanMode;
use crate::protocol::{
    AuthKind, CheckState, CompletionCheck, LocationId, ProjectId, ScopeTier, UnknownReason,
};
use crate::provider::listing::OrgListing;
use crate::scan::discover::{RepoCandidate, RepoKind};
use crate::scan::launcher::{ScanLauncherDeps, ThreadScanLauncher};
use crate::scan::run::{platform_of, Discovered};
use crate::scan::skiplist::SkipList;
use crate::scan::store::SqliteScanStore;
use crate::scan::ScanSupervisor;
use crate::testing::events::ValidatingSink;
use crate::testing::FakeClock;

/// The rows a fixture library already holds, for a section fixture to plant its own against.
#[derive(Debug, Clone, Default)]
pub struct FixtureIds {
    /// The library's projects, in creation order.
    pub projects: Vec<ProjectId>,
    /// The library's locations, in creation order.
    pub locations: Vec<LocationId>,
}

/// A fixture that plants rows for one registered section.
pub type SectionFixture = fn(&Transaction<'_>, &FixtureIds) -> Result<(), IndexError>;

/// One fixture per registered section, by section name. A section registers its fixture in the
/// change that registers the section; `sidecar_registry` holds the two lists equal.
pub const SECTION_FIXTURES: &[(&str, SectionFixture)] = &[
    ("no_scan_projects", uninstalled_project),
    ("check_na", ruled_check),
    ("location_trust", trusted_copy),
    ("readme_consent", consented_readme),
    ("accounts", connected_account),
];

/// A project with one present copy, for a fixture that rules on a project or trusts a copy.
fn present_project(
    tx: &Transaction<'_>,
    name: &str,
) -> Result<(ProjectId, LocationId), IndexError> {
    tx.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, ?2, 1, 1)",
        rusqlite::params![name, format!("{name}-lineage")],
    )?;
    let project = tx.last_insert_rowid();
    let path = format!("/fixture/{name}");
    tx.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'fixture', 'present', 'worktree')",
        rusqlite::params![project, path.as_bytes(), path],
    )?;
    Ok((ProjectId(project), LocationId(tx.last_insert_rowid())))
}

/// A project whose ten check rows the evaluator wrote, with the user's N/A ruling on `tests`: the
/// library's first project, or one of its own in a library that has none.
fn ruled_check(tx: &Transaction<'_>, ids: &FixtureIds) -> Result<(), IndexError> {
    let project = match ids.projects.first() {
        Some(project) => *project,
        None => present_project(tx, "ruled")?.0,
    };
    let rows = CompletionCheck::ALL.map(|key| {
        let ruled = key == CompletionCheck::Tests;
        CheckRow {
            key,
            state: if ruled {
                CheckState::Na
            } else {
                CheckState::Unknown
            },
            user_na: ruled.then_some(true),
            unknown_reason: (!ruled).then_some(UnknownReason::NotRunYet),
            observed_at: 1,
        }
    });
    crate::completion::store::write_all_ten(tx, project, &rows)
}

/// A copy the user trusted: the library's first, or one of its own in a library that has none.
fn trusted_copy(tx: &Transaction<'_>, ids: &FixtureIds) -> Result<(), IndexError> {
    let location = match ids.locations.first() {
        Some(location) => *location,
        None => present_project(tx, "trusted")?.1,
    };
    crate::surfaces::repair::set_trusted(tx, location, 20)?;
    Ok(())
}

/// A project the user allowed remote README images for: the library's first, or one of its own
/// in a library that has none.
fn consented_readme(tx: &Transaction<'_>, ids: &FixtureIds) -> Result<(), IndexError> {
    let project = match ids.projects.first() {
        Some(project) => *project,
        None => present_project(tx, "consented")?.0,
    };
    crate::readme::consent::write_readme_remote(tx, project, Some(30), 30)?;
    Ok(())
}

/// A connected account with two organisations, one of them switched on.
fn connected_account(tx: &Transaction<'_>, _ids: &FixtureIds) -> Result<(), IndexError> {
    let store = |e: crate::accounts::store::AccountError| IndexError::Sidecar(e.to_string());
    let account = crate::accounts::store::insert_account(
        tx,
        &crate::accounts::store::NewAccount {
            provider: "github".to_owned(),
            host: "github.com".to_owned(),
            login: "fixture-user".to_owned(),
            display_name: None,
            auth_kind: AuthKind::Pat,
            scope_tier: ScopeTier::Private,
            granted_scopes: crate::provider::scopes::SCOPES_PRIVATE
                .iter()
                .map(|scope| (*scope).to_owned())
                .collect(),
            token_ref: "github:github.com:fixture-user".to_owned(),
        },
        30,
    )
    .map_err(store)?;
    let orgs = [
        OrgListing {
            login: "org-on".to_owned(),
            repo_count_seen: Some(2),
        },
        OrgListing {
            login: "org-off".to_owned(),
            repo_count_seen: None,
        },
    ];
    crate::accounts::store::upsert_orgs(tx, account, &orgs, 31).map_err(store)?;
    crate::accounts::store::set_org_enabled(tx, account, "org-on", true).map_err(store)?;
    Ok(())
}

/// An uninstalled project — both its copies removed — with a note and one session.
fn uninstalled_project(tx: &Transaction<'_>, _ids: &FixtureIds) -> Result<(), IndexError> {
    tx.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, notes, created_at, updated_at)
         VALUES ('uninstalled', 'uninstalled', 'uninstalled-lineage', 'kept after removal', 1, 1)",
        [],
    )?;
    let project = tx.last_insert_rowid();
    for (path, removed_at) in [
        ("/fixture/uninstalled-a", 10_i64),
        ("/fixture/uninstalled-b", 11),
    ] {
        tx.execute(
            "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                   store_key, presence, repo_kind, removed_at)
             VALUES (?1, 'linux', ?2, ?2, ?3, 'fixture', 'missing', 'worktree', ?4)",
            rusqlite::params![project, path.as_bytes(), path, removed_at],
        )?;
    }
    tx.execute(
        "INSERT INTO session (project_id, location_id, started_at, ended_at, credited_seconds,
                              close_reason)
         VALUES (?1, ?2, 100, 160, 60, 'stop')",
        [project, tx.last_insert_rowid()],
    )?;
    Ok(())
}

/// The repositories a fixture library creates, in the order it indexes them. The last is
/// uninstalled once indexed: its copy is marked removed and its directory moved out of the root.
const LIBRARY_REPOS: [&str; 4] = ["alder", "birch", "cedar", "retired"];

/// A library built the way the app builds one, for a test to export, corrupt and rebuild.
#[derive(Debug)]
pub struct FixtureLibrary {
    /// The library's index, still open.
    pub index: Index,
    /// The projects still on disk and their copies, in indexing order, which the section
    /// fixtures planted against.
    pub ids: FixtureIds,
    /// The repositories still on disk: what a scan of the repositories directory finds.
    pub repos: Vec<PathBuf>,
    /// The subject key of every project the library holds, sorted.
    pub subjects: Vec<String>,
}

/// Builds a library in `data_dir` over repositories it creates in `repos_dir`, for the round
/// trip's tests and the fixture binary alike.
///
/// Each repository has one commit at a fixed date and a remote, and is indexed through the
/// hand-off with a `Discovered` built the way an install builds one, so its subject is the one a
/// real scan computes. On top go the rows nothing re-derives — a note, flags, sessions with
/// segments, a session-track XP row, a launch target the user made, a manual collection, the
/// root, an identity with an alias, a setting and a view-state row — then the last repository is
/// uninstalled, and every [`SECTION_FIXTURES`] entry is planted.
///
/// # Errors
/// A filesystem, git or `SQLite` failure while building it.
pub fn build_library(
    data_dir: &Path,
    repos_dir: &Path,
    now: i64,
) -> Result<FixtureLibrary, IndexError> {
    // Left in `data_dir` for its caller to drop with it: a removal in this crate is an audited act.
    let scratch = data_dir.join("fixture-scratch");
    let corpus = CorpusGit::new(scratch.join("git-home")).map_err(other)?;
    let git = SystemGit::new(
        Arc::new(GitExec::system(ensure_empty_hooks_dir(data_dir)?)),
        Arc::new(GitSlots::for_machine()),
        Arc::new(SystemClock::new()),
    );
    let mut index = Mutex::new(Index::open_at(data_dir, now)?);
    let root = locked(&mut index).with_tx(|tx| insert_root(tx, repos_dir, now))?;
    let mut indexed = Vec::new();
    for (day, name) in (0_i64..).zip(LIBRARY_REPOS) {
        let path = repos_dir.join(name);
        commit_repo(&corpus, &path, name, now - (30 - day) * 86_400)?;
        let at = index_repo(&index, &git, root, &path, now)?;
        indexed.push((path, at));
    }

    let (retired_path, retired) = indexed
        .pop()
        .ok_or_else(|| other("the library indexed no repository"))?;
    let ids = FixtureIds {
        projects: indexed.iter().map(|(_, at)| at.project).collect(),
        locations: indexed.iter().map(|(_, at)| at.location).collect(),
    };
    locked(&mut index).with_tx(|tx| {
        plant_base(tx, &ids, now)?;
        tx.execute(
            "UPDATE project SET notes = 'kept after its copy was removed' WHERE id = ?1",
            [retired.project.0],
        )?;
        crate::uninstall::command::commit_removal(tx, retired.location, now - 60)
            .map_err(|failure| other(failure.message))?;
        for (_, fixture) in SECTION_FIXTURES {
            fixture(tx, &ids)?;
        }
        Ok(())
    })?;
    // What an uninstall leaves on disk: no copy under the root.
    std::fs::rename(&retired_path, scratch.join("retired"))?;

    let index = scan_once(Arc::new(index), Arc::new(git), now)?;
    let mut subjects = Vec::new();
    let projects: Vec<i64> = index
        .conn()
        .prepare("SELECT id FROM project WHERE merged_into IS NULL ORDER BY id")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for project in projects {
        if let Some(subject) = subject_for_project(index.conn(), ProjectId(project))? {
            subjects.push(subject.to_key());
        }
    }
    subjects.sort();
    Ok(FixtureLibrary {
        index,
        ids,
        repos: indexed.into_iter().map(|(path, _)| path).collect(),
        subjects,
    })
}

fn other(error: impl std::fmt::Display) -> IndexError {
    IndexError::Io(std::io::Error::other(error.to_string()))
}

fn locked(index: &mut Mutex<Index>) -> &mut Index {
    index.get_mut().unwrap_or_else(PoisonError::into_inner)
}

/// How long the library's one scan may take, on a loaded machine.
const SCAN_DEADLINE: Duration = Duration::from_secs(120);

/// One full scan of the library's root, its jobs queued into nothing, and the index handed back
/// once the scan's thread has let go of it.
///
/// The app scans after every change, and §4.6's presence pass then re-derives every copy's
/// `presence`: a removed copy reads `missing`, not the `present` its removal left. Without the
/// scan the library would hold values no scanned library does, and the next scan would rewrite
/// them.
fn scan_once(
    index: Arc<Mutex<Index>>,
    git: Arc<dyn GitBackend>,
    now: i64,
) -> Result<Index, IndexError> {
    let scans = ScanSupervisor::new(Arc::new(ThreadScanLauncher::new(ScanLauncherDeps {
        store: Arc::new(SqliteScanStore::new(Arc::clone(&index))),
        index: Arc::clone(&index),
        git,
        mounts: Arc::new(SystemMountResolver::new()),
        clock: Arc::new(FakeClock::new(now)),
        skip: Arc::new(SkipList::default()),
        wsl: None,
        jobs: Arc::new(NullJobSink),
        events: Arc::new(ValidatingSink::default()),
    })));
    scans.start(ScanMode::Full, now).map_err(other)?;
    let deadline = Instant::now() + SCAN_DEADLINE;
    while scans.live().is_some() {
        if Instant::now() > deadline {
            return Err(other("the library's scan never finished"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(scans);
    // The run releases its slot as its last act and drops its handle on the index just after.
    let mut shared = index;
    loop {
        match Arc::try_unwrap(shared) {
            Ok(alone) => return Ok(alone.into_inner().unwrap_or_else(PoisonError::into_inner)),
            Err(held) if Instant::now() < deadline => {
                shared = held;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return Err(other("the library's scan never let go of the index")),
        }
    }
}

/// The side this host's own paths are on.
const fn host_kind() -> &'static str {
    if cfg!(windows) {
        "win"
    } else {
        "linux"
    }
}

/// The repositories directory as a root the user added, so a scan of the library walks it.
fn insert_root(tx: &Transaction<'_>, repos_dir: &Path, now: i64) -> Result<i64, IndexError> {
    tx.execute(
        "INSERT INTO scan_root (kind, distro, path_bytes, path_key, path_display, enabled,
                                added_by, descend_into_repos, added_at)
         VALUES (?1, '', ?2, ?3, ?4, 1, 'user', 0, ?5)",
        rusqlite::params![
            host_kind(),
            path_bytes(repos_dir),
            path_key(repos_dir, platform_of(host_kind())),
            path_display(repos_dir),
            now
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

/// One repository at `path`: one commit dated `at` and a remote, so its subject is its lineage.
fn commit_repo(corpus: &CorpusGit, path: &Path, name: &str, at: i64) -> Result<(), IndexError> {
    std::fs::create_dir_all(path)?;
    let template = corpus.template_arg();
    let url = format!("https://example.invalid/owner/{name}.git");
    write_file(&path.join("notes.txt"), name.as_bytes()).map_err(other)?;
    for args in [
        &["init", "-q", template.as_str(), "."][..],
        &["add", "notes.txt"],
        &["commit", "-q", "-m", "first"],
        &["remote", "add", "origin", url.as_str()],
    ] {
        corpus.run(path, at, args).map_err(other)?;
    }
    Ok(())
}

/// Index the repository at `path` through the hand-off, as an install does after its clone.
fn index_repo(
    index: &Mutex<Index>,
    git: &dyn GitBackend,
    root: i64,
    path: &Path,
    now: i64,
) -> Result<Indexed, IndexError> {
    let facts = SystemMountResolver::new().resolve(path).map_err(other)?;
    let git_dir = path.join(".git");
    let discovered = Discovered {
        candidate: RepoCandidate {
            path: path.to_path_buf(),
            kind: RepoKind::WorkTree,
            git_dir: git_dir.clone(),
            common_dir: git_dir,
        },
        root_id: root,
        kind: host_kind().to_owned(),
        distro: String::new(),
        path_bytes: path_bytes(path),
        path_key: path_key(path, platform_of(host_kind())),
        path_display: path_display(path),
        store_key: facts.store_key,
        volume_key: facts.volume_key,
    };
    let cancel = CancelToken::new();
    let ctx = HandoffCtx {
        git,
        cancel: &cancel,
        store_class: facts.class,
        generation: 1,
        now,
    };
    hand_off_discovered(index, &ctx, &discovered).map_err(other)
}

/// The rows no scan re-derives, on the library's three present projects.
fn plant_base(tx: &Transaction<'_>, ids: &FixtureIds, now: i64) -> Result<(), IndexError> {
    let [first, second, third] = ids.projects[..] else {
        return Err(other("the base rows need three projects"));
    };
    let [first_copy, second_copy, _] = ids.locations[..] else {
        return Err(other("the base rows need three copies"));
    };
    // The seed is the directory name at first index, which the art keeps after a rename; one that
    // differs from the directory now is a value only the sidecar carries.
    tx.execute(
        "UPDATE project SET notes = 'a note the user wrote', is_pinned = 1, acknowledged_at = ?2,
                            reroll_offset = 2, seed_basename = 'alder-before-rename'
         WHERE id = ?1",
        [first.0, now - 500],
    )?;
    tx.execute("UPDATE project SET is_hidden = 1 WHERE id = ?1", [second.0])?;
    for (project, location, started, segments) in [
        (first, first_copy, now - 7_200, 2_i64),
        (second, second_copy, now - 3_600, 1),
    ] {
        tx.execute(
            "INSERT INTO session (project_id, location_id, started_at, ended_at,
                                  credited_seconds, close_reason)
             VALUES (?1, ?2, ?3, ?4, ?5, 'stop')",
            [
                project.0,
                location.0,
                started,
                started + segments * 60,
                segments * 60,
            ],
        )?;
        let session = tx.last_insert_rowid();
        for segment in 0..segments {
            let from = started + segment * 60;
            tx.execute(
                "INSERT INTO session_segment (session_id, started_at, ended_at, credited_seconds,
                                              closed_by)
                 VALUES (?1, ?2, ?3, 60, 'session_end')",
                [session, from, from + 60],
            )?;
        }
    }
    let subject = subject_for_project(tx, first)?
        .ok_or_else(|| other("an indexed project has no subject"))?
        .to_key();
    tx.execute(
        "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                track, meta)
         VALUES (?1, 0, ?2, ?3, 'session', ?4, 'session', NULL)",
        rusqlite::params![
            now - 7_080,
            first.0,
            subject,
            format!("session:{subject}:0")
        ],
    )?;
    tx.execute(
        "INSERT INTO launch_target (project_id, kind, name, exec_bytes, sort_index, detected)
         VALUES (?1, 'editor', 'An editor', x'2f62696e2f6564', 0, 0)",
        [third.0],
    )?;
    tx.execute(
        "INSERT INTO collection (name, kind) VALUES ('Fixture shelf', 'manual')",
        [],
    )?;
    let collection = tx.last_insert_rowid();
    for member in [first, second] {
        tx.execute(
            "INSERT INTO collection_member (collection_id, project_id) VALUES (?1, ?2)",
            [collection, member.0],
        )?;
    }
    tx.execute(
        "INSERT INTO identity (email, name, source, confirmed_at)
         VALUES ('fixture@example.invalid', 'Fixture', 'gitconfig', ?1)",
        [now - 400],
    )?;
    tx.execute(
        "INSERT INTO identity_alias (identity_id, email, reason)
         VALUES (?1, 'fixture@users.example.invalid', 'manual')",
        [tx.last_insert_rowid()],
    )?;
    tx.execute(
        "INSERT INTO app_meta (k, v) VALUES ('effects_tier', 'reduced')
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        [],
    )?;
    tx.execute(
        "INSERT INTO view_state (k, v) VALUES ('shelf.sort', 'touched')
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        [],
    )?;
    Ok(())
}

/// An index the core cannot open, one per startup report window the shell paints (§11.2a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupPlant {
    /// Bytes that are not a database beside a junk `-wal` and `-shm`: the `corrupt_index` report.
    Corrupt,
    /// A real database stamped one schema past this build's: the `schema_from_future` report.
    Future,
    /// An empty database stamped schema 5. Step 6 alters `location`, which an empty database does
    /// not have, so every later build stops there: the `migration_failed` report.
    MigrationFailed,
}

impl StartupPlant {
    /// The plant a `--kind` word names, or `None` for any other word.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "corrupt" => Some(Self::Corrupt),
            "future" => Some(Self::Future),
            "migration-failed" => Some(Self::MigrationFailed),
            _ => None,
        }
    }

    /// The `--kind` word for this plant, and the `kind` its result records.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Corrupt => "corrupt",
            Self::Future => "future",
            Self::MigrationFailed => "migration-failed",
        }
    }
}

/// Plants `plant` in `data_dir`, so the next core started there exits with that plant's report,
/// and answers the names of the files the directory then holds, sorted.
///
/// # Errors
///
/// A filesystem or `SQLite` error while writing the files.
pub fn plant_startup_failure(
    plant: StartupPlant,
    data_dir: &Path,
    now: i64,
) -> Result<Vec<String>, IndexError> {
    let db = Index::db_path(data_dir);
    match plant {
        StartupPlant::Corrupt => {
            std::fs::create_dir_all(data_dir)?;
            std::fs::write(&db, b"this is not a database")?;
            std::fs::write(data_dir.join("index.db-wal"), b"a stale journal")?;
            std::fs::write(data_dir.join("index.db-shm"), b"a stale wal-index")?;
        }
        StartupPlant::Future => {
            drop(Index::open_at(data_dir, now)?);
            open_connection(&db)?.pragma_update(
                None,
                "user_version",
                SUPPORTED_SCHEMA_VERSION + 1,
            )?;
        }
        StartupPlant::MigrationFailed => {
            open_connection(&db)?.pragma_update(None, "user_version", 5)?;
        }
    }
    let mut names = std::fs::read_dir(data_dir)?
        .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}
