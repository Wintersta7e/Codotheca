#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §38.7.4 and §38.8.6 through the real `JobRunner`: J4's settle makes a project's git rows the
//! set its history implies — `commit_day` and `release` alike — deleting what it no longer
//! implies after a ratchet, writing none for a Reference project, and stamping the rows of a
//! read nobody should be told about `silent`.

mod support;

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{
    ensure_empty_hooks_dir, Authorship, CommitterTally, GitBackend, GitError, GitExec, GitSlots,
    RootCommit, SystemGit, TagFact, TagKind,
};
use codotheca_core::identity::confirm;
use codotheca_core::index::Index;
use codotheca_core::jobs::j4_history::local_date;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::ledger::xp::xp_by_term;
use codotheca_core::mount::StoreClass;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{LocationId, ProjectId, XpTerm};
use codotheca_core::testing::zone::FixedZone;
use codotheca_core::testing::{FakeClock, FakeGitBackend, GitReply};
use serde_json::Value;

const ME: &str = "me@example.invalid";
const WORK: &str = "work@example.invalid";
const OTHER: &str = "other@example.invalid";

/// The runner's frozen clock, after every fixture date.
const T0: i64 = 1_800_000_000;
const DAY: i64 = 86_400;

/// A repository whose every commit and tag names its own identity and date.
struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    work: PathBuf,
}

impl Repo {
    fn init() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = support::canonical(dir.path());
        let home = root.join("home");
        let work = root.join("repo");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        let repo = Self {
            _dir: dir,
            root,
            home,
            work,
        };
        repo.git_as(
            ME,
            "2024-01-01T00:00:00+00:00",
            &["init", "-q", "-b", "main"],
        );
        repo
    }

    /// `git args` as `email`, both author and committer (and so tagger), at `date`.
    fn git_as(&self, email: &str, date: &str, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(&self.work)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .env("GIT_AUTHOR_NAME", "Someone")
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", "Someone")
            .env("GIT_COMMITTER_EMAIL", email)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(["-c", "gc.auto=0"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "fixture git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// A commit by `email` on `date` (noon UTC).
    fn commit(&self, email: &str, date: &str) {
        let at = format!("{date}T12:00:00+00:00");
        self.git_as(email, &at, &["commit", "-q", "--allow-empty", "-m", date]);
    }

    /// An annotated tag on `HEAD`, by `email` as its tagger, on `date`.
    fn annotated(&self, email: &str, date: &str, name: &str) {
        let at = format!("{date}T12:00:00+00:00");
        self.git_as(email, &at, &["tag", "-a", name, "-m", "a release"]);
    }

    /// A lightweight tag on `HEAD`, which carries no identity of its own.
    fn lightweight(&self, name: &str) {
        self.git_as(ME, "2024-01-01T00:00:00+00:00", &["tag", name]);
    }
}

/// What a J4 settle announces is not under test here, beyond matching the schema.
struct Quiet;

impl EventSink for Quiet {
    fn emit(&self, topic: &str, event: &str, payload: Value) {
        codotheca_core::testing::events::validated(topic, event, &payload);
    }
}

/// One project on one present copy of `repo`, the user's addresses confirmed as `identities`,
/// and `committers` as its authorship census.
struct Rig {
    repo: Repo,
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    project: ProjectId,
    location: LocationId,
}

impl Rig {
    fn new(repo: Repo, identities: &[&str], committers: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::open_at(dir.path(), T0).unwrap();
        let path = repo.work.to_string_lossy().into_owned();
        let (project, location) = index
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                          is_reference, created_at, updated_at)
                     VALUES ('p', 'p', 'lin', 1, 0, 0, 0)",
                    [],
                )?;
                let project = ProjectId(tx.last_insert_rowid());
                tx.execute(
                    "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                           store_key, presence, repo_kind)
                     VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
                    rusqlite::params![project.0, path.as_bytes(), path],
                )?;
                let location = LocationId(tx.last_insert_rowid());
                for email in identities {
                    tx.execute(
                        "INSERT INTO identity (email, is_user, source, confirmed_at)
                         VALUES (?1, 1, 'manual', ?2)",
                        rusqlite::params![email, T0 - 100],
                    )?;
                }
                for email in committers {
                    tx.execute(
                        "INSERT INTO project_committer (project_id, email, commits)
                         VALUES (?1, ?2, 1)",
                        rusqlite::params![project.0, email],
                    )?;
                }
                Ok((project, location))
            })
            .unwrap();
        Self {
            repo,
            _dir: dir,
            index: Arc::new(Mutex::new(index)),
            project,
            location,
        }
    }

    fn real_git(&self, clock: Arc<FakeClock>) -> Arc<dyn GitBackend> {
        // The empty hooks directory sits beside the repository, as the shared fixture keeps it.
        let hooks = ensure_empty_hooks_dir(&self.repo.root).unwrap();
        Arc::new(SystemGit::new(
            Arc::new(GitExec::new(support::test_git(), hooks)),
            Arc::new(GitSlots::new(4)),
            clock,
        ))
    }

    /// One J4 over the real repository, run to idle.
    fn settle(&self) {
        let clock = Arc::new(FakeClock::new(T0));
        self.settle_with(self.real_git(Arc::clone(&clock)), clock);
    }

    /// One J4 on a fresh runner with `git`, spun until the runner is idle — never a bare sleep.
    fn settle_with(&self, git: Arc<dyn GitBackend>, clock: Arc<FakeClock>) {
        let deps = JobDeps {
            git,
            clock,
            cancel: CancelToken::new(),
            zone: Arc::new(FixedZone(0)),
        };
        let runner = JobRunner::new(Arc::clone(&self.index), deps, Arc::new(Quiet));
        assert!(runner.enqueue(Job {
            kind: JobKind::J4History,
            project_id: self.project,
            location_id: self.location,
            store_key: "store".to_owned(),
            store_kind: StoreClass::Local,
            priority: Priority::Interactive,
            not_before: 0,
            origin: JobOrigin::Walk,
        }));
        runner.start(1);
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline && !runner.is_idle() {
            std::thread::sleep(Duration::from_millis(25));
        }
        let idle = runner.is_idle();
        runner.request_stop();
        runner.join();
        assert!(idle, "J4 did not settle; job state: {:?}", self.job_state());
    }

    fn job_state(&self) -> Option<(String, Option<String>)> {
        let guard = self.index.lock().unwrap();
        let state = guard
            .conn()
            .query_row(
                "SELECT state, reason FROM project_job_state WHERE project_id = ?1 AND job = 'j4'",
                [self.project.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        drop(guard);
        state
    }

    /// `kind`'s rows for the project, oldest first: each one's date and its `meta`.
    fn rows(&self, kind: &str) -> Vec<(String, Value)> {
        let guard = self.index.lock().unwrap();
        let mut statement = guard
            .conn()
            .prepare(
                "SELECT ts, meta FROM xp_events WHERE project_id = ?1 AND kind = ?2
                  ORDER BY ts, dedupe_key",
            )
            .unwrap();
        let rows: Vec<(String, Value)> = statement
            .query_map(rusqlite::params![self.project.0, kind], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .unwrap()
            .map(|row| {
                let (ts, meta) = row.unwrap();
                (
                    local_date(ts.div_euclid(DAY)),
                    serde_json::from_str(&meta).unwrap(),
                )
            })
            .collect();
        drop(statement);
        drop(guard);
        rows
    }

    fn dates(&self, kind: &str) -> Vec<String> {
        self.rows(kind).into_iter().map(|(date, _)| date).collect()
    }

    /// The `release` rows as `date what`.
    fn releases(&self) -> Vec<String> {
        self.rows("release")
            .into_iter()
            .map(|(date, meta)| format!("{date} {}", meta["what"].as_str().unwrap_or("?")))
            .collect()
    }

    fn sql(&self, sql: &str) {
        let guard = self.index.lock().unwrap();
        guard.conn().execute(sql, [self.project.0]).unwrap();
        drop(guard);
    }

    fn confirm(&self, emails: &[&str]) {
        let mut guard = self.index.lock().unwrap();
        let emails: Vec<String> = emails.iter().map(|e| (*e).to_owned()).collect();
        confirm::apply(guard.conn_mut(), &emails, T0).unwrap();
        drop(guard);
    }
}

// ---------------------------------------------------------------------------------------------
// AC-P4-38-11 — the `release` writer, through J4.
// ---------------------------------------------------------------------------------------------

#[test]
fn ac_p4_38_11_release_through_j4() {
    let repo = Repo::init();
    repo.commit(ME, "2024-05-01");
    repo.annotated(ME, "2024-05-03", "v1");
    repo.commit(OTHER, "2024-05-02");
    repo.lightweight("their-commit");
    repo.annotated(OTHER, "2024-05-04", "their-tag");
    repo.commit(ME, "2024-05-09");
    repo.lightweight("light");
    repo.annotated(ME, "2024-05-20", "pair-b");
    repo.annotated(ME, "2024-05-20", "pair-a");
    repo.annotated(ME, "2024-06-02", "archived");
    let rig = Rig::new(repo, &[ME], &[ME, OTHER]);

    rig.settle();

    let releases = rig.releases();
    eprintln!("release rows: {releases:?}");
    // An identity tagger dates by the tag; a lightweight tag on an identity commit by the commit;
    // someone else's tag of either kind writes none; one date names its byte-first tag; and a tag
    // the user named `archived` is a release like any other.
    assert_eq!(
        releases,
        [
            "2024-05-03 v1",
            "2024-05-09 light",
            "2024-05-20 pair-a",
            "2024-06-02 archived"
        ]
    );
    let (_, meta) = &rig.rows("release")[0];
    assert_eq!(meta["when"], "2024-05-03");
    assert_eq!(meta["project_name"], "p");

    // Three dates in May are capped at two releases; June's one is one.
    let guard = rig.index.lock().unwrap();
    let release_xp = xp_by_term(guard.conn()).unwrap().of(XpTerm::Release);
    drop(guard);
    eprintln!("release term: {release_xp}");
    assert_eq!(release_xp, 80 + 40);
}

// ---------------------------------------------------------------------------------------------
// AC-P4-38-12 — the reconcile.
// ---------------------------------------------------------------------------------------------

#[test]
fn ac_p4_38_12_the_reconcile() {
    a_narrowing_that_keeps_the_project_authored();
    a_confirm_that_moves_the_project_to_reference();
    a_reference_project_still_holding_the_users_commits();
}

/// The confirm's affected set does not hold a project that stays authored, so it is the next J4
/// that removes the dropped address's day.
fn a_narrowing_that_keeps_the_project_authored() {
    let repo = Repo::init();
    repo.commit(ME, "2024-05-01");
    repo.commit(WORK, "2024-05-02");
    let rig = Rig::new(repo, &[ME, WORK], &[ME, WORK]);
    rig.settle();
    assert_eq!(rig.dates("commit_day"), ["2024-05-01", "2024-05-02"]);

    rig.confirm(&[ME]);
    assert_eq!(
        rig.dates("commit_day"),
        ["2024-05-01", "2024-05-02"],
        "the confirm itself recomputes nothing for a project that stays authored"
    );
    rig.settle();
    eprintln!("after the narrowing's J4: {:?}", rig.dates("commit_day"));
    assert_eq!(rig.dates("commit_day"), ["2024-05-01"]);
}

/// A project the confirm moves to Reference: the next J4 writes no git row, and still walks.
fn a_confirm_that_moves_the_project_to_reference() {
    let repo = Repo::init();
    repo.commit(ME, "2024-05-01");
    repo.annotated(ME, "2024-05-03", "v1");
    let rig = Rig::new(repo, &[ME, WORK], &[ME]);
    rig.settle();
    assert_eq!(rig.dates("commit_day"), ["2024-05-01"]);
    assert_eq!(rig.releases(), ["2024-05-03 v1"]);

    rig.confirm(&[WORK]);
    rig.sql("UPDATE project SET last_commit_subject = NULL WHERE id = ?1");
    rig.settle();
    let guard = rig.index.lock().unwrap();
    let (reference, subject): (i64, Option<String>) = guard
        .conn()
        .query_row(
            "SELECT is_reference, last_commit_subject FROM project WHERE id = ?1",
            [rig.project.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    drop(guard);
    eprintln!(
        "reference={reference} subject={subject:?} commit_day={:?} release={:?}",
        rig.dates("commit_day"),
        rig.releases()
    );
    assert_eq!(reference, 1);
    assert_eq!(rig.dates("commit_day"), Vec::<String>::new());
    assert_eq!(rig.releases(), Vec::<String>::new());
    assert_eq!(subject.as_deref(), Some("2024-05-01"), "J4 still walked");
}

/// There, the user's commits left the identity set with the verdict. Here the history still holds
/// them — the state a Reference verdict leaves — and no recompute emptied the rows: the J4
/// reconcile alone does.
fn a_reference_project_still_holding_the_users_commits() {
    let repo = Repo::init();
    repo.commit(ME, "2024-05-01");
    repo.annotated(ME, "2024-05-03", "v1");
    let rig = Rig::new(repo, &[ME], &[ME]);
    rig.settle();
    assert_eq!(rig.releases(), ["2024-05-03 v1"]);
    rig.sql("UPDATE project SET is_reference = 1 WHERE id = ?1");
    rig.settle();
    eprintln!(
        "a Reference project holding the user's commits: commit_day={:?} release={:?}",
        rig.dates("commit_day"),
        rig.releases()
    );
    assert_eq!(rig.dates("commit_day"), Vec::<String>::new());
    assert_eq!(rig.releases(), Vec::<String>::new());
}

static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn record(sql: &str) {
    TRACE
        .lock()
        .unwrap()
        .push(sql.split_whitespace().collect::<Vec<_>>().join(" "));
}

/// The rewrite clause of AC-P4-38-8: an amended commit moves to another date, and the next J4
/// deletes the stale day only after the floor ratcheted from the reading it is about to change.
#[test]
fn a_history_rewrite_is_reconciled_after_a_ratchet() {
    let repo = Repo::init();
    repo.commit(ME, "2024-05-01");
    repo.commit(ME, "2024-05-02");
    let rig = Rig::new(repo, &[ME], &[ME]);
    rig.settle();
    assert_eq!(rig.dates("commit_day"), ["2024-05-01", "2024-05-02"]);

    rig.repo.git_as(
        ME,
        "2024-05-09T12:00:00+00:00",
        &[
            "commit",
            "-q",
            "--amend",
            "--allow-empty",
            "--reset-author",
            "-m",
            "rewritten",
        ],
    );
    TRACE.lock().unwrap().clear();
    rig.index.lock().unwrap().conn_mut().trace(Some(record));
    rig.settle();
    rig.index.lock().unwrap().conn_mut().trace(None);
    let statements = std::mem::take(&mut *TRACE.lock().unwrap());

    assert_eq!(rig.dates("commit_day"), ["2024-05-01", "2024-05-09"]);
    let delete = statements
        .iter()
        .position(|s| s.starts_with("DELETE FROM xp_events"));
    let ratchet = statements
        .iter()
        .position(|s| s.starts_with("UPDATE level_floor"));
    eprintln!("first ratchet at {ratchet:?}, first delete at {delete:?}");
    assert!(delete.is_some(), "the stale day was not deleted");
    assert!(
        ratchet.is_some() && ratchet < delete,
        "the floor UPDATE must come before the DELETE"
    );
}

/// `silent` on the rows of a read nobody should hear about: a first read, and an upgrade's
/// re-read; never on a new commit of a project already read.
#[test]
fn silent_receipts() {
    let repo = Repo::init();
    repo.commit(ME, "2024-05-01");
    repo.annotated(ME, "2024-05-01", "v1");
    let rig = Rig::new(repo, &[ME], &[ME]);
    let silent = |date: &str, kind: &str| -> Option<Value> {
        rig.rows(kind)
            .into_iter()
            .find(|(d, _)| d == date)
            .map(|(_, meta)| meta["silent"].clone())
    };

    rig.settle();
    eprintln!("first read: {:?}", rig.rows("commit_day"));
    assert_eq!(silent("2024-05-01", "commit_day"), Some(Value::Bool(true)));
    assert_eq!(silent("2024-05-01", "release"), Some(Value::Bool(true)));

    rig.repo.commit(ME, "2024-05-05");
    rig.sql(
        "UPDATE project_job_state SET state = 'queued', reason = 'app_upgraded'
          WHERE project_id = ?1 AND job = 'j4'",
    );
    rig.settle();
    assert_eq!(silent("2024-05-05", "commit_day"), Some(Value::Bool(true)));

    rig.repo.commit(ME, "2024-05-07");
    rig.settle();
    eprintln!("after a new commit: {:?}", rig.rows("commit_day"));
    assert_eq!(silent("2024-05-07", "commit_day"), Some(Value::Null));
}

/// A tag read that fails fails the whole settle: the reconcile never runs with the release half
/// missing, which would delete every standing `release` row.
#[test]
fn a_failed_tag_read_keeps_the_standing_release_rows() {
    let rig = Rig::new(Repo::init(), &[ME], &[ME]);
    // 2024-05-01 and 2024-05-03, noon UTC.
    let (commit_at, tagged_at) = (1_714_564_800, 1_714_737_600);
    let fake = FakeGitBackend::new();
    fake.always_root_commits(GitReply::Ok(vec![RootCommit {
        oid: "a".repeat(40),
        committed_at: commit_at,
        tz_offset_min: 0,
    }]))
    .always_authorship(GitReply::Ok(Authorship {
        committers: vec![CommitterTally {
            email: ME.to_owned(),
            commits: 1,
            days: std::iter::once(commit_at / DAY).collect(),
            last_commit_at: commit_at,
        }],
    }))
    .always_commit_subjects(GitReply::Ok(Vec::new()))
    .on_release_tags(
        None,
        GitReply::Ok(vec![TagFact {
            name: "v1".to_owned(),
            object_oid: "b".repeat(40),
            commit_oid: "a".repeat(40),
            kind: TagKind::Annotated {
                tagger_email: ME.to_owned(),
                tagged_at,
                tz_offset_min: 0,
            },
        }]),
    )
    .on_release_tags(
        None,
        GitReply::Err(GitError::Unreadable {
            detail: "a tag read that failed".to_owned(),
        }),
    );
    let fake = Arc::new(fake);
    let clock = Arc::new(FakeClock::new(T0));

    rig.settle_with(Arc::<FakeGitBackend>::clone(&fake), Arc::clone(&clock));
    assert_eq!(rig.releases(), ["2024-05-03 v1"]);
    assert_eq!(rig.dates("commit_day"), ["2024-05-01"]);

    rig.settle_with(fake, clock);
    eprintln!(
        "after the failed read: job={:?} release={:?}",
        rig.job_state(),
        rig.releases()
    );
    assert_ne!(
        rig.job_state().map(|(state, _)| state).as_deref(),
        Some("ok")
    );
    assert_eq!(rig.releases(), ["2024-05-03 v1"]);
    assert_eq!(rig.dates("commit_day"), ["2024-05-01"]);
}
