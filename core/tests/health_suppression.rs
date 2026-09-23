//! §30.5 and A11.2 — **suppression is two gates over one predicate**, and only one of them
//! stops work from happening.
//!
//! p3-29 left `ContentGates.compute_suppressed` as one `false` literal **deliberately**: wiring
//! it to `acknowledged_at` in wave 1 would have suppressed the blob read for every project, for
//! ever, because nothing wrote that column. This file is what proves the expression is no longer
//! a literal — by driving J7 through the **real** `gates_for` rather than by constructing a
//! `ContentGates`, which is the only shape that can tell the two apart.
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{JobClass, JobContext, RepoHandle, TreeEntry};
use codotheca_core::health::enrolment::{compute_suppressed, is_enrolled, surface_suppressed};
use codotheca_core::index::Index;
use codotheca_core::jobs::j7_markers;
use codotheca_core::jobs::JobOutcome;
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::testing::{FakeGitBackend, GitReply};

const NOW: i64 = 1_700_000_000;

/// One project, one copy, the grant on, authorship computed and not Reference — so §29.7's first
/// and third predicates are open and **the only thing left to decide is suppression**.
struct Rig {
    _dir: tempfile::TempDir,
    index: std::sync::Mutex<Index>,
    git: std::sync::Arc<FakeGitBackend>,
    project: ProjectId,
    location: LocationId,
    repo: RepoHandle,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(&dir.path().join("index")).unwrap();
        let (project, location) = {
            let conn = index.conn();
            conn.execute(
                "INSERT INTO project
                   (name, seed_basename, created_at, updated_at, authored_by_user, is_reference)
                 VALUES ('p', 'p', 0, 0, 1, 0)",
                [],
            )
            .unwrap();
            let project = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO app_meta (k, v) VALUES ('content_scan_enabled', '1')
                 ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO location
                   (project_id, kind, path_bytes, path_key, path_display, store_key, presence,
                    repo_kind, head_oid)
                 VALUES (?1, 'linux', x'2f70', x'2f70', '/p', 'store-a', 'present',
                         'worktree', 'head-one')",
                rusqlite::params![project],
            )
            .unwrap();
            (ProjectId(project), LocationId(conn.last_insert_rowid()))
        };
        let rig = Rig {
            _dir: dir,
            index: std::sync::Mutex::new(index),
            git: std::sync::Arc::new(FakeGitBackend::new()),
            project,
            location,
            repo: RepoHandle::bare(
                std::path::Path::new("/does/not/matter"),
                codotheca_core::git::StoreKey::new("test-store"),
                codotheca_core::mount::StoreClass::Local,
            ),
        };
        let bytes = b"// TODO: something to find\n";
        let oid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        rig.git.script_blob(oid, bytes);
        rig.git.always_head_tree(GitReply::Ok(vec![TreeEntry {
            mode: "100644".to_owned(),
            kind: "blob".to_owned(),
            oid: oid.to_owned(),
            path: b"src/a.rs".to_vec(),
        }]));
        rig
    }

    fn enrol(&self) {
        let guard = self.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "UPDATE project SET acknowledged_at = ?2 WHERE id = ?1",
                rusqlite::params![self.project.0, NOW],
            )
            .unwrap();
        drop(guard);
    }

    fn run(&self) -> JobOutcome {
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);
        j7_markers::run_j7(
            &self.index,
            self.git.as_ref(),
            &self.repo,
            &ctx,
            j7_markers::ScanRun {
                project: self.project,
                location: self.location,
                cursor: None,
                now: NOW,
                tz_offset_min: 0,
                detected_in: codotheca_core::protocol::HealthDetectedIn::Background,
                announce: None,
            },
        )
        .unwrap()
    }

    fn count(&self, table: &str) -> i64 {
        let guard = self.index.lock().unwrap();
        let n = guard
            .conn()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        drop(guard);
        n
    }

    fn presence_answers(&self) -> Option<codotheca_core::jobs::presence::PresenceAnswers> {
        let guard = self.index.lock().unwrap();
        let answers = j7_markers::presence_for_project(guard.conn(), self.project).unwrap();
        drop(guard);
        answers
    }

    fn gates(&self) -> j7_markers::ContentGates {
        let guard = self.index.lock().unwrap();
        let gates = j7_markers::gates_for(guard.conn(), self.project).unwrap();
        drop(guard);
        gates
    }
}

/// **A11.2** — the two gates read the same inputs and gate different things, so the predicate is
/// stated once and named twice.
#[test]
fn ac_p3_30_11_both_gates_read_one_predicate() {
    let mut cases = 0usize;
    for enrolled in [false, true] {
        for is_archived in [false, true] {
            assert_eq!(
                compute_suppressed(enrolled, is_archived),
                surface_suppressed(enrolled, is_archived),
                "enrolled={enrolled} archived={is_archived}"
            );
            assert_eq!(
                compute_suppressed(enrolled, is_archived),
                !enrolled || is_archived
            );
            cases += 1;
        }
    }
    eprintln!("enrolment/archive combinations exercised: {cases}");
    assert!(cases > 0);
    assert!(!is_enrolled(None));
}

/// A **surface-suppressed** project still runs its bounded work: §29's enumeration lands its four
/// presence answers while the blob read writes nothing.
#[test]
fn ac_p3_30_11_a_surface_suppressed_project_still_runs_its_bounded_work() {
    let rig = Rig::new();
    // Unenrolled and not archived, which is what a project in the untriaged backlog is.
    assert!(rig.gates().compute_suppressed, "the gate is not consulted");
    assert!(
        rig.gates().runs(),
        "suppression stopped the enumeration too"
    );

    assert_eq!(rig.run(), JobOutcome::Done);

    let scans = rig.count("blob_scan");
    let findings = rig.count("blob_finding");
    let presence = rig
        .presence_answers()
        .expect("the enumeration wrote its four answers");
    eprintln!(
        "surface-suppressed run: blob_scan={scans} blob_finding={findings} presence={presence:?}"
    );
    assert_eq!(scans, 0, "the blob read ran behind a closed gate");
    assert_eq!(findings, 0);
    // The running side, named and counted: an assertion that only checked the zeros would pass
    // against a J7 that did nothing at all.
    let content_rows = rig.count("project_content_scan");
    assert!(
        content_rows > 0,
        "the bounded work wrote nothing, so the zeros above prove nothing"
    );
    assert!(
        rig.git.blob_requests().is_empty(),
        "a blob was requested from git"
    );
}

/// **The test that fails against p3-29's literal and passes only once the expression is real.**
///
/// It drives J7 through the real `gates_for` and never constructs a `ContentGates`: constructing
/// one exercises `reads_blobs` and proves nothing about what fills the field.
#[test]
fn ac_p3_30_11_the_blob_read_gate_reads_acknowledged_at() {
    let unenrolled = Rig::new();
    assert_eq!(unenrolled.run(), JobOutcome::Done);
    let closed = unenrolled.count("blob_scan");
    let closed_presence = unenrolled.count("project_content_scan");

    let enrolled = Rig::new();
    enrolled.enrol();
    assert!(
        !enrolled.gates().compute_suppressed,
        "an enrolled project is not compute-suppressed"
    );
    assert_eq!(enrolled.run(), JobOutcome::Done);
    let open = enrolled.count("blob_scan");
    let open_presence = enrolled.count("project_content_scan");

    eprintln!(
        "blob_scan rows — unenrolled: {closed}, enrolled: {open}; \
         project_content_scan rows — unenrolled: {closed_presence}, enrolled: {open_presence}"
    );
    assert_eq!(closed, 0, "an unenrolled project read blobs");
    assert!(open > 0, "an enrolled project read no blob");
    assert!(
        closed_presence > 0 && open_presence > 0,
        "the enumeration must run on both sides"
    );

    // Archived is the gate's second cause, and it closes an enrolled project again.
    let archived = Rig::new();
    archived.enrol();
    {
        let guard = archived.index.lock().unwrap();
        guard
            .conn()
            .execute(
                "UPDATE project SET is_archived = 1 WHERE id = ?1",
                [archived.project.0],
            )
            .unwrap();
        drop(guard);
    }
    assert!(archived.gates().compute_suppressed);
    assert_eq!(archived.run(), JobOutcome::Done);
    assert_eq!(archived.count("blob_scan"), 0);
}

/// **`compute_suppressed` is consulted at exactly one call site**, and that is the criterion's
/// real claim: that *only* the blob read is gated by it.
///
/// A source audit, because the claim is about where a value may be read and not about what it
/// evaluates to. It is the half that is decidable in this worktree — §32's lockfile clause lands
/// with p3-32's sweep.
#[test]
fn ac_p3_30_11_compute_suppressed_is_consulted_at_exactly_one_call_site() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    assert!(
        !files.is_empty(),
        "scanned no files: a source audit over nothing is a failing audit"
    );

    let mut reads: Vec<String> = Vec::new();
    let mut scanned_lines = 0usize;
    for path in &files {
        let text = std::fs::read_to_string(path).unwrap();
        for (i, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            scanned_lines += 1;
            // A **read** of the field, not its declaration and not the expression that fills it.
            if code.contains(".compute_suppressed") {
                reads.push(format!(
                    "{}:{}",
                    path.strip_prefix(&root).unwrap_or(path).display(),
                    i + 1
                ));
            }
        }
    }
    let reads: Vec<String> = reads.into_iter().map(|s| s.replace('\\', "/")).collect();
    eprintln!(
        "compute_suppressed reads over {} files / {scanned_lines} code lines: {reads:?}",
        files.len()
    );
    // **Two readers, and the plan's *"exactly one call site"* is wrong about the merged tree.**
    // p3-28 landed the second deliberately: `sweep_from_content` reads the gate to record §28.5's
    // `skipped_suppressed` outcome, which is a *record of* the skip and not a second gate on it.
    // The criterion's real claim survives intact — **only the blob read is gated** — and it is
    // what the file list asserts.
    let mut files_reading: Vec<&str> = reads
        .iter()
        .map(|r| r.split(':').next().unwrap_or(r))
        .collect();
    files_reading.sort_unstable();
    files_reading.dedup();
    assert_eq!(
        files_reading,
        ["debt/markers.rs", "jobs/j7_markers.rs"],
        "the compute gate is consulted somewhere other than §29's blob read and §28's sweep record"
    );
    // The one in J7 is the gate itself; the one in §28 must not gate anything, so it may not sit
    // in a `reads_blobs`-shaped predicate.
    let markers = std::fs::read_to_string(root.join("debt/markers.rs")).unwrap();
    assert!(
        markers.contains("DebtSweepOutcome::SkippedSuppressed"),
        "§28's read of the gate is not the outcome record it is supposed to be"
    );

    // And the stale comment naming a future owner is gone: a *this will be replaced* comment that
    // is no longer true is worse than none.
    let j7 = std::fs::read_to_string(root.join("jobs/j7_markers.rs")).unwrap();
    assert!(
        !j7.contains("until p3-30 lands"),
        "the doc comment still names p3-30 as a future owner"
    );
}

fn collect_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}
