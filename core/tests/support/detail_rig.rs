//! Fixtures for the project page's core half. Not API — a rig, kept out of the two test files
//! so both can share one seeding vocabulary instead of drifting into two.
#![allow(dead_code)]

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::detail::DetailCtx;
use codotheca_core::index::path::{PathPlatform, StoredPath};
use codotheca_core::index::Index;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::testing::{FakeGitBackend, FakeMountResolver};

pub const NOW: i64 = 1_781_179_200;

/// Every §6 freshness request the page made, so "asks once" is a count and not a hope.
#[derive(Debug, Default)]
pub struct RecordingJobs {
    pub indexed: std::sync::Mutex<Vec<(i64, i64)>>,
    pub visible: std::sync::Mutex<Vec<(i64, i64)>>,
    /// §29.7: which command asked, and whether it asked for the content scan.
    pub content_asks: std::sync::Mutex<Vec<(i64, bool)>>,
}

impl codotheca_core::jobs::JobSink for RecordingJobs {
    fn on_location_indexed(
        &self,
        project: codotheca_core::protocol::ProjectId,
        location: codotheca_core::protocol::LocationId,
        _store_key: &str,
        _store_kind: StoreClass,
    ) {
        self.indexed
            .lock()
            .expect("lock")
            .push((project.0, location.0));
    }

    fn on_visible(
        &self,
        project: codotheca_core::protocol::ProjectId,
        location: codotheca_core::protocol::LocationId,
        _store_key: &str,
        _store_kind: StoreClass,
        _needs_art: bool,
        wants_content: bool,
    ) {
        if let Ok(mut asks) = self.content_asks.lock() {
            asks.push((project.0, wants_content));
        }
        self.visible
            .lock()
            .expect("lock")
            .push((project.0, location.0));
    }
}

pub struct Rig {
    pub dir: tempfile::TempDir,
    pub index: Index,
    pub git: FakeGitBackend,
    pub mount: FakeMountResolver,
    pub sink: CollectingSink,
    /// §6: `projects.get` asks for a current worktree reading for the copy it shows.
    pub jobs: RecordingJobs,
}

impl Rig {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let index = Index::open(dir.path()).expect("open");
        Self {
            dir,
            index,
            git: FakeGitBackend::new(),
            mount: FakeMountResolver::new(),
            sink: CollectingSink::default(),
            jobs: RecordingJobs::default(),
        }
    }

    pub(crate) fn ctx(&self) -> DetailCtx<'_> {
        DetailCtx {
            index: &self.index,
            git: &self.git,
            mount: &self.mount,
            events: &self.sink,
            jobs: &self.jobs,
            sync: &codotheca_core::sync::runner::NullSyncSink,
            now: NOW,
        }
    }

    pub(crate) fn conn(&self) -> &rusqlite::Connection {
        self.index.conn()
    }

    /// One project whose every derived column is NULL — the phase-1 default, because nothing
    /// has computed them yet.
    pub(crate) fn project(&self, id: i64, name: &str) {
        self.conn()
            .execute(
                "INSERT INTO project (id, name, seed_basename, created_at, updated_at,
                                      last_touched_at)
                 VALUES (?1, ?2, ?2, ?3, ?3, ?3)",
                rusqlite::params![id, name, NOW],
            )
            .expect("seed project");
    }

    /// A location row. `presence` and the observed facts are the caller's, because every one of
    /// them is a fact this test suite is checking the handler does not invent.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn location(
        &self,
        id: i64,
        project_id: i64,
        path: &str,
        presence: &str,
        branch: Option<&str>,
        ahead: Option<i64>,
        last_seen_at: Option<i64>,
    ) {
        let stored = StoredPath::from_bytes(path.as_bytes().to_vec(), PathPlatform::Unix);
        let (bytes, key, display) = stored.as_params();
        self.conn()
            .execute(
                "INSERT INTO location
                   (id, project_id, kind, path_bytes, path_key, path_display,
                    volume_key, store_key, presence, repo_kind, branch, ahead, last_seen_at,
                    refstate_observed_at)
                 VALUES (?1, ?2, 'linux', ?3, ?4, ?5, 'VOL-A', 'store-a', ?6, 'worktree',
                         ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    id,
                    project_id,
                    bytes,
                    key,
                    display,
                    presence,
                    branch,
                    ahead,
                    last_seen_at,
                    ahead.map(|_| NOW - 60),
                ],
            )
            .expect("seed location");
    }

    pub(crate) fn peek_cache(&self, project_id: i64, excerpt: Option<&str>, computed_at: i64) {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO peek_cache (project_id, readme_excerpt, computed_at)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![project_id, excerpt, computed_at],
            )
            .expect("seed peek_cache");
    }

    pub(crate) fn notes(&self, project_id: i64) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT notes FROM project WHERE id = ?1",
                [project_id],
                |r| r.get(0),
            )
            .expect("read notes")
    }

    /// Every column of every row of one table, in rowid order — the before/after comparison
    /// §17's "no destructive operation" claim is actually checked with.
    pub(crate) fn fingerprint(&self, table: &str) -> String {
        let conn = self.conn();
        let cols: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
                .expect("table_info");
            let mapped = stmt
                .query_map([table], |r| r.get::<_, String>(0))
                .expect("columns");
            mapped
                .map(|c| format!("quote(\"{}\")", c.expect("column")))
                .collect()
        };
        let sql = format!(
            "SELECT COALESCE(group_concat({}, '|'), '<empty>') FROM (SELECT * FROM \"{table}\")",
            cols.join(" || ',' || ")
        );
        conn.query_row(&sql, [], |r| r.get(0)).expect("fingerprint")
    }

    /// A real on-disk repository directory, because `RepoHandle::resolve` stats `.git`.
    pub(crate) fn make_repo_dir(&self, name: &str) -> std::path::PathBuf {
        let path = self.dir.path().join(name);
        std::fs::create_dir_all(path.join(".git")).expect("mkdir");
        self.mount.map(
            path.clone(),
            MountFacts {
                store_key: "store-a".to_owned(),
                volume_key: Some("VOL-A".to_owned()),
                class: StoreClass::Local,
            },
        );
        path
    }

    /// The tagged-bytes shape §2.5 gives a path, which is the only shape one enters the core in.
    pub(crate) fn dialog_bytes(path: &std::path::Path) -> serde_json::Value {
        use base64::Engine as _;
        serde_json::json!({
            "b64": base64::engine::general_purpose::STANDARD.encode(path.to_string_lossy().as_bytes())
        })
    }

    /// Every path and size under `root`, so criterion 44's "no file moved" is a measurement
    /// rather than an assertion about the code.
    ///
    /// It walks the **location's** tree, not the temp directory: the index database lives beside
    /// it and its WAL grows on any write, which would make this compare the storage engine
    /// rather than the repository.
    pub(crate) fn hash_tree(root: &std::path::Path) -> Vec<(String, u64)> {
        let mut out = Vec::new();
        walk_tree(root, root, &mut out);
        out.sort();
        out
    }
}

fn walk_tree(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let rel = p
            .strip_prefix(root)
            .unwrap_or(&p)
            .to_string_lossy()
            .into_owned();
        if p.is_dir() {
            out.push((format!("d:{rel}"), 0));
            walk_tree(&p, root, out);
        } else {
            let len = std::fs::metadata(&p).map_or(0, |m| m.len());
            out.push((format!("f:{rel}"), len));
        }
    }
}
