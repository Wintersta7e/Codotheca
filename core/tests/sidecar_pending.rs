//! §48.8.4: a pending record applies exactly once, in the hand-off transaction that computes its
//! project's subject key, and never onto a guessed project.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use codotheca_core::assembly::handoff::{hand_off_discovered, HandoffCtx, Indexed};
use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::git::{GitBackend, GitExec, GitSlots, RepoHandle, StoreKey, SystemGit};
use codotheca_core::index::migrate::{apply_all, MIGRATIONS};
use codotheca_core::index::pending::{match_pending, resolve_subject_unique};
use codotheca_core::index::rebuild::{rebuild_in_place, RebuildOutcome};
use codotheca_core::index::subject::{subject_for_project, ProjectSubject};
use codotheca_core::index::{open_connection, Index};
use codotheca_core::mount::StoreClass;
use codotheca_core::paths::{path_bytes, path_display, path_key};
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::scan::discover::{RepoCandidate, RepoKind};
use codotheca_core::scan::run::{platform_of, Discovered};
use rusqlite::{params, Connection};

const NOW: i64 = 1_760_000_000;

/// A neutral git, so the machine's own config cannot change what a fixture is.
fn git_at(cwd: &Path, home: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_AUTHOR_DATE", "2024-01-02T03:04:05+00:00")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_DATE", "2024-01-02T03:04:05+00:00")
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "fixture git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A data directory, a git home and repositories on disk, indexed through the real hand-off.
struct Library {
    dir: tempfile::TempDir,
    git: Arc<dyn GitBackend>,
}

impl Library {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        let hooks = codotheca_core::git::ensure_empty_hooks_dir(&dir.path().join("hooks")).unwrap();
        let git: Arc<dyn GitBackend> = Arc::new(SystemGit::new(
            Arc::new(GitExec::system(hooks)),
            Arc::new(GitSlots::for_machine()),
            Arc::new(SystemClock::new()),
        ));
        Self { dir, git }
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    /// One repository with one commit of its own — so its own lineage — and a remote.
    fn repo(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join("repos").join(name);
        let home = self.dir.path().join("home");
        std::fs::create_dir_all(&path).unwrap();
        git_at(&path, &home, &["init", "-b", "main", "."]);
        std::fs::write(path.join("a.txt"), name.as_bytes()).unwrap();
        git_at(&path, &home, &["add", "-A"]);
        git_at(&path, &home, &["commit", "-m", "first"]);
        let url = format!("https://example.invalid/owner/{name}.git");
        git_at(&path, &home, &["remote", "add", "origin", &url]);
        path
    }

    fn open(&self) -> Arc<Mutex<Index>> {
        Arc::new(Mutex::new(Index::open_at(&self.data(), NOW).unwrap()))
    }

    fn hand_off(&self, index: &Mutex<Index>, path: &Path) -> Indexed {
        let cancel = CancelToken::new();
        let ctx = HandoffCtx {
            git: self.git.as_ref(),
            cancel: &cancel,
            store_class: StoreClass::Local,
            generation: 1,
            now: NOW,
        };
        hand_off_discovered(index, &ctx, &discovered_at(path)).unwrap()
    }

    /// Export, close, overwrite the index with bytes SQLite reads as no database, and rebuild.
    fn export_corrupt_and_rebuild(&self, index: Arc<Mutex<Index>>, now: i64) {
        index.lock().unwrap().export_sidecar(now).unwrap();
        drop(Arc::try_unwrap(index).unwrap());
        std::fs::write(Index::db_path(&self.data()), b"this is not a database").unwrap();
        match rebuild_in_place(&self.data(), now + 1) {
            Ok(RebuildOutcome::Rebuilt(report)) => eprintln!("rebuilt, {} pending", report.pending),
            other => panic!("expected a rebuild, got {other:?}"),
        }
    }
}

/// What the walk hands on, built from what `classify` would have resolved.
fn discovered_at(path: &Path) -> Discovered {
    let handle = RepoHandle::resolve(path, StoreKey::new("store-a"), StoreClass::Local).unwrap();
    Discovered {
        candidate: RepoCandidate {
            path: path.to_path_buf(),
            kind: RepoKind::WorkTree,
            git_dir: handle.git_dir.clone(),
            common_dir: handle.common_dir,
        },
        root_id: 1,
        kind: "linux".to_owned(),
        distro: String::new(),
        path_bytes: path_bytes(path),
        path_key: path_key(path, platform_of("linux")),
        path_display: path_display(path),
        store_key: "store-a".to_owned(),
        volume_key: Some("vol-a".to_owned()),
    }
}

/// Three sessions of one segment each, two session-track XP rows and a launch target the user
/// made, all on `project` and its copy `location`.
fn plant(conn: &Connection, project: ProjectId, location: LocationId) {
    let subject = subject_for_project(conn, project)
        .unwrap()
        .unwrap()
        .to_key();
    for i in 0_i64..3 {
        conn.execute(
            "INSERT INTO session (project_id, location_id, started_at, ended_at,
                                  credited_seconds, close_reason)
             VALUES (?1, ?2, ?3, ?4, 60, 'stop')",
            params![project.0, location.0, 100 + i * 1000, 160 + i * 1000],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_segment (session_id, started_at, ended_at, credited_seconds,
                                          closed_by)
             VALUES (?1, ?2, ?3, 60, 'session_end')",
            params![conn.last_insert_rowid(), 100 + i * 1000, 160 + i * 1000],
        )
        .unwrap();
    }
    for i in 0_i64..2 {
        conn.execute(
            "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind, dedupe_key,
                                    track, meta)
             VALUES (?1, 0, ?2, ?3, 'session', ?4, 'session', NULL)",
            params![
                100 + i,
                project.0,
                subject,
                format!("session:{subject}:{i}")
            ],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO launch_target (project_id, kind, name, exec_bytes, sort_index, detected)
         VALUES (?1, 'editor', 'An editor', x'2f62696e2f6564', 0, 0)",
        [project.0],
    )
    .unwrap();
}

/// Per subject key: sessions, segments, session-track XP rows, user-made launch targets.
fn counts_by_subject(conn: &Connection) -> BTreeMap<String, [i64; 4]> {
    let projects: Vec<i64> = conn
        .prepare("SELECT id FROM project WHERE merged_into IS NULL ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let count = |sql: &str, id: i64| -> i64 { conn.query_row(sql, [id], |r| r.get(0)).unwrap() };
    projects
        .into_iter()
        .map(|id| {
            let key = subject_for_project(conn, ProjectId(id))
                .unwrap()
                .unwrap()
                .to_key();
            let row = [
                count("SELECT count(*) FROM session WHERE project_id = ?1", id),
                count(
                    "SELECT count(*) FROM session_segment s JOIN session x ON x.id = s.session_id
                     WHERE x.project_id = ?1",
                    id,
                ),
                count(
                    "SELECT count(*) FROM xp_events WHERE project_id = ?1 AND track = 'session'",
                    id,
                ),
                count(
                    "SELECT count(*) FROM launch_target WHERE project_id = ?1 AND detected = 0",
                    id,
                ),
            ];
            (key, row)
        })
        .collect()
}

fn pending_rows(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM sidecar_pending", [], |r| r.get(0))
        .unwrap()
}

/// A fresh index at the tip, for the cases planted in SQL.
fn fresh() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    apply_all(&mut conn, MIGRATIONS).unwrap();
    (dir, conn)
}

/// A project on lineage `l` with no remote, and one copy at `/r/<path>`. Answers both ids.
fn remoteless(conn: &Connection, l: &str, path: &str) -> (i64, i64) {
    conn.execute(
        "INSERT INTO project (name, seed_basename, lineage_key, created_at, updated_at)
         VALUES (?1, ?1, ?2, 1, 1)",
        params![path, l],
    )
    .unwrap();
    let project = conn.last_insert_rowid();
    let bytes = format!("/r/{path}");
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', ?2, ?2, ?3, 's', 'present', 'worktree')",
        params![project, bytes.as_bytes(), bytes],
    )
    .unwrap();
    (project, conn.last_insert_rowid())
}

/// A pending project record on `subject` naming the copy at `/r/<path>`, carrying one session.
fn pend(conn: &Connection, subject: &str, path: &str) {
    let key = serde_json::json!([{
        "kind": "linux",
        "distro": "",
        "path_key": hex(format!("/r/{path}").as_bytes()),
    }]);
    let record = serde_json::json!({
        "kind": "project",
        "record": {
            "subject": subject,
            "notes": format!("the note of {path}"),
            "is_pinned": false,
            "is_archived": false,
            "is_hidden": false,
            "acknowledged_at": null,
            "seed_basename": path,
            "reroll_offset": 0,
            "sessions": [{
                "started_at": 10, "ended_at": 20, "credited_seconds": 10,
                "close_reason": "stop", "segments": [],
            }],
            "xp_events": [],
            "launch_targets": [],
            "location_keys": key,
        },
        "member_of": [],
    });
    conn.execute(
        "INSERT INTO sidecar_pending (source_generation, subject_key, location_keys, record,
                                      queued_at)
         VALUES (1, ?1, ?2, ?3, 5)",
        params![subject, key.to_string(), record.to_string()],
    )
    .unwrap();
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn matched(conn: &mut Connection, project: i64) {
    let tx = conn.transaction().unwrap();
    let report = match_pending(&tx, ProjectId(project), NOW).unwrap();
    tx.commit().unwrap();
    eprintln!("matched for project {project}: {report:?}");
}

fn note_of(conn: &Connection, project: i64) -> Option<String> {
    conn.query_row("SELECT notes FROM project WHERE id = ?1", [project], |r| {
        r.get(0)
    })
    .unwrap()
}

/// Run `f` on the index's connection, holding the lock for that and nothing else.
fn read<T>(index: &Mutex<Index>, f: impl FnOnce(&Connection) -> T) -> T {
    let guard = index.lock().unwrap();
    let out = f(guard.conn());
    drop(guard);
    out
}

/// AC-P4-48-18: rebuild; the hand-off rediscovers half the subjects; export; corrupt again;
/// rebuild; rediscover all. Every record of the first sidecar appears exactly once and the
/// pending table ends empty. Remoteless siblings resolve by location keys or stay pending, and a
/// project whose subject is first computed at a later hand-off receives its record there.
#[test]
fn ac_p4_48_18_pending_survives_and_is_consumed_once() {
    consumed_once_across_two_rebuilds();
    siblings_resolve_by_location_keys_or_wait();
    a_subject_computed_later_receives_its_record();
}

fn consumed_once_across_two_rebuilds() {
    let lib = Library::new();
    let repos: Vec<PathBuf> = ["anvil", "bellows", "chisel", "drill"]
        .iter()
        .map(|name| lib.repo(name))
        .collect();
    let original = lib.open();
    for path in &repos {
        let indexed = lib.hand_off(&original, path);
        read(&original, |conn| {
            plant(conn, indexed.project, indexed.location);
        });
    }
    let first = read(&original, counts_by_subject);
    assert_eq!(first.len(), 4, "four subjects: {first:?}");

    lib.export_corrupt_and_rebuild(original, NOW + 10);
    let half = lib.open();
    for path in &repos[..2] {
        lib.hand_off(&half, path);
    }
    lib.export_corrupt_and_rebuild(half, NOW + 20);
    let whole = lib.open();
    for path in &repos {
        lib.hand_off(&whole, path);
    }

    let after = read(&whole, counts_by_subject);
    let mut compared = 0_u32;
    for (subject, expected) in &first {
        let found = after
            .get(subject)
            .unwrap_or_else(|| panic!("{subject} did not come back"));
        for (i, what) in ["sessions", "segments", "xp rows", "launch targets"]
            .iter()
            .enumerate()
        {
            eprintln!(
                "{what} for {subject}: expected {}, found {}",
                expected[i], found[i]
            );
            assert_eq!(
                found[i], expected[i],
                "{what} for {subject}: expected {}, found {}",
                expected[i], found[i]
            );
            compared += 1;
        }
    }
    eprintln!("per-subject counts compared: {compared}");
    assert_eq!(compared, 16);
    assert_eq!(read(&whole, pending_rows), 0, "every record was consumed");
}

/// Two live remoteless siblings on one lineage: a record lands on the one its location keys
/// name, never on the lower id by default, and one naming neither waits.
fn siblings_resolve_by_location_keys_or_wait() {
    let (_d, mut conn) = fresh();
    let (a, _) = remoteless(&conn, "sibling-lineage", "a");
    let (b, _) = remoteless(&conn, "sibling-lineage", "b");
    let subject = ProjectSubject::Lineage {
        lineage_key: "sibling-lineage".to_owned(),
        remote_key: None,
    }
    .to_key();
    pend(&conn, &subject, "b");
    pend(&conn, &subject, "elsewhere");
    matched(&mut conn, a);
    matched(&mut conn, b);
    assert_eq!(
        note_of(&conn, a),
        None,
        "a record not naming a's copy landed on a"
    );
    assert_eq!(note_of(&conn, b).as_deref(), Some("the note of b"));
    assert_eq!(
        pending_rows(&conn),
        1,
        "a record naming neither sibling stays pending"
    );
}

/// A project the scan has not found yet has no copy and no lineage, so no subject key; the
/// hand-off that hydrates it computes its lineage key, and its record applies there.
fn a_subject_computed_later_receives_its_record() {
    let lib = Library::new();
    let path = lib.repo("lathe");
    let original = lib.open();
    let indexed = lib.hand_off(&original, &path);
    let (expected, remote) = read(&original, |conn| {
        plant(conn, indexed.project, indexed.location);
        let remote: String = conn
            .query_row(
                "SELECT remote_key FROM project WHERE id = ?1",
                [indexed.project.0],
                |r| r.get(0),
            )
            .unwrap();
        (counts_by_subject(conn), remote)
    });
    lib.export_corrupt_and_rebuild(original, NOW + 30);

    let rebuilt = lib.open();
    let listed = read(&rebuilt, |conn| {
        conn.execute(
            "INSERT INTO project (name, seed_basename, remote_key, created_at, updated_at)
             VALUES ('lathe', 'lathe', ?1, 1, 1)",
            [&remote],
        )
        .unwrap();
        let id = ProjectId(conn.last_insert_rowid());
        assert_eq!(subject_for_project(conn, id).unwrap(), None);
        id
    });
    let hydrated = lib.hand_off(&rebuilt, &path);
    assert_eq!(
        hydrated.project, listed,
        "the hand-off hydrated the listed row"
    );
    assert_eq!(read(&rebuilt, counts_by_subject), expected);
    assert_eq!(read(&rebuilt, pending_rows), 0);
}

/// The hand-off commits the match before it returns, and the hand-off runs no job: a restored
/// record is in place before any job for the copy can be enqueued.
#[test]
fn the_match_commits_in_the_hand_off_before_any_job_runs() {
    let lib = Library::new();
    let path = lib.repo("anvil");
    let original = lib.open();
    let before = lib.hand_off(&original, &path);
    read(&original, |conn| {
        plant(conn, before.project, before.location);
    });
    lib.export_corrupt_and_rebuild(original, NOW + 10);

    let rebuilt = lib.open();
    assert_eq!(read(&rebuilt, pending_rows), 1);
    let after = lib.hand_off(&rebuilt, &path);
    read(&rebuilt, |conn| {
        assert_eq!(
            pending_rows(conn),
            0,
            "the record is consumed in the hand-off"
        );
        let sessions: i64 = conn
            .query_row(
                "SELECT count(*) FROM session WHERE project_id = ?1",
                [after.project.0],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sessions, 3);
        let jobs: i64 = conn
            .query_row("SELECT count(*) FROM project_job_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(jobs, 0, "no job ran before the match");
    });
}

/// A copy this app removed comes back, every column, before the sessions that name it — so each
/// session finds its copy rather than none.
#[test]
fn a_removed_location_is_recreated_before_its_sessions() {
    let lib = Library::new();
    let path = lib.repo("anvil");
    let original = lib.open();
    let before = lib.hand_off(&original, &path);
    let removed = read(&original, |conn| {
        conn.execute(
            "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                   store_key, presence, repo_kind, removed_at)
             VALUES (?1, 'linux', x'2f6f6c64', x'2f6f6c64', '/old', 's', 'missing', 'worktree',
                     4242)",
            [before.project.0],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO session (project_id, location_id, started_at, ended_at,
                                  credited_seconds, close_reason)
             VALUES (?1, ?2, 100, 160, 60, 'stop')",
            [before.project.0, id],
        )
        .unwrap();
        dump(conn, id)
    });
    lib.export_corrupt_and_rebuild(original, NOW + 10);

    let rebuilt = lib.open();
    let after = lib.hand_off(&rebuilt, &path);
    let (id, session_location, back) = read(&rebuilt, |conn| {
        let (id, session_location): (i64, Option<i64>) = conn
            .query_row(
                "SELECT l.id, s.location_id FROM location l, session s
                 WHERE l.path_key = x'2f6f6c64' AND s.project_id = l.project_id",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        (id, session_location, dump(conn, id))
    });
    assert_eq!(
        session_location,
        Some(id),
        "the session names the copy it ran in"
    );
    let mut compared = 0_u32;
    for (column, value) in &removed {
        match column.as_str() {
            "id" => {}
            "project_id" => assert_eq!(back[column], format!("Integer({})", after.project.0)),
            _ => assert_eq!(&back[column], value, "column {column}"),
        }
        compared += 1;
    }
    eprintln!("removed-location columns compared: {compared}");
    assert!(compared > 2);
}

/// Every column of a `location` row, as text.
fn dump(conn: &Connection, id: i64) -> BTreeMap<String, String> {
    let mut stmt = conn
        .prepare("SELECT * FROM location WHERE id = ?1")
        .unwrap();
    let names: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
    stmt.query_row([id], |r| {
        names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let value: rusqlite::types::Value = r.get(i)?;
                Ok((name.clone(), format!("{value:?}")))
            })
            .collect::<rusqlite::Result<_>>()
    })
    .unwrap()
}

/// The one never-guess rule: `Some` only when exactly one live project holds the subject.
#[test]
fn resolve_subject_unique_answers_only_a_single_live_holder() {
    let (_d, conn) = fresh();
    let (a, _) = remoteless(&conn, "shared", "a");
    let (b, _) = remoteless(&conn, "shared", "b");
    let siblings = ProjectSubject::Lineage {
        lineage_key: "shared".to_owned(),
        remote_key: None,
    };
    let mut cases = 0_u32;

    let two_live = resolve_subject_unique(&conn, &siblings).unwrap();
    eprintln!("two live siblings: {two_live:?}");
    assert_eq!(two_live, None, "two live holders resolve to none");
    cases += 1;

    conn.execute("UPDATE project SET merged_into = ?1 WHERE id = ?2", [b, a])
        .unwrap();
    let one_merged = resolve_subject_unique(&conn, &siblings).unwrap();
    eprintln!("one merged away: {one_merged:?}");
    assert_eq!(
        one_merged,
        Some(ProjectId(b)),
        "a merged project holds nothing"
    );
    cases += 1;

    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES ('bare', 'bare', 1, 1)",
        [],
    )
    .unwrap();
    let bare = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display, store_key,
                               presence, repo_kind)
         VALUES (?1, 'linux', x'2f72', x'2f72', '/r', 's', 'present', 'worktree')",
        [bare],
    )
    .unwrap();
    let path = subject_for_project(&conn, ProjectId(bare))
        .unwrap()
        .unwrap();
    let by_path = resolve_subject_unique(&conn, &path).unwrap();
    eprintln!("a path one project holds: {by_path:?}");
    assert_eq!(by_path, Some(ProjectId(bare)));
    cases += 1;

    let nobody = ProjectSubject::Lineage {
        lineage_key: "nobody".to_owned(),
        remote_key: Some("example.invalid/owner/nobody".to_owned()),
    };
    let unheld = resolve_subject_unique(&conn, &nobody).unwrap();
    eprintln!("a subject nothing holds: {unheld:?}");
    assert_eq!(unheld, None);
    cases += 1;

    eprintln!("resolver cases: {cases}");
    assert_eq!(cases, 4);
}
