//! §46.9's current-state gate, driven through a real `JobRunner` against real repositories: no
//! job runs for a removed copy or for any copy of a removed project, and the check is made at
//! dispatch, so a job queued before the removal does not run after it. J5 art reads no repository
//! and is the one kind the gate lets through.
//!
//! Every test also runs a job for a live copy of another project in the same runner, so the
//! harness demonstrably sees a run and the absence of one means something.
//!
//! The page and the Peek of a removed copy, or of a removed project, answer from what is stored
//! and ask for no reading at all; a live copy's page in the same rig asks once.
//!
//! §46.20's source audit: `project.removed_at` is read in `projects/current.rs` alone, and every
//! other statement naming it is a listed writer.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

#[path = "support/detail_rig.rs"]
mod detail_rig;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::SystemClock;
use codotheca_core::detail::get::handle_project_get;
use codotheca_core::git::{GitSlots, SystemGit};
use codotheca_core::index::Index;
use codotheca_core::jobs::scheduler::JobRunner;
use codotheca_core::jobs::{Job, JobDeps, JobKind, JobOrigin, Priority};
use codotheca_core::mount::StoreClass;
use codotheca_core::projects::{dispatch_projects_command, ProjectsCtx};
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::{LocationId, ProjectId};
use codotheca_core::sync::runner::NullSyncSink;
use serde_json::json;
use support::TestRepo;

/// Records every event the runner publishes, so `scan/job_done` is read rather than assumed.
#[derive(Debug, Default)]
struct RecordingSink {
    events: Mutex<Vec<(String, String, serde_json::Value)>>,
}

impl EventSink for RecordingSink {
    fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
        codotheca_core::testing::events::validated(topic, event, &payload);
        self.events
            .lock()
            .unwrap()
            .push((topic.to_owned(), event.to_owned(), payload));
    }
}

/// One project with one copy, backed by a real repository holding one commit.
struct Copy {
    repo: TestRepo,
    project: ProjectId,
    location: LocationId,
}

struct Rig {
    _dir: tempfile::TempDir,
    index: Arc<Mutex<Index>>,
    runner: Arc<JobRunner>,
    events: Arc<RecordingSink>,
    /// The copy each test removes, or whose project it removes.
    gone: Copy,
    /// A copy of another project that nothing removes.
    live: Copy,
}

fn committed_repo() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("a.txt", b"one\n");
    repo.git(&["add", "a.txt"]);
    repo.commit("first");
    repo
}

fn plant(index: &Mutex<Index>, repo: TestRepo, name: &str) -> Copy {
    let guard = index.lock().unwrap();
    let conn = guard.conn();
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 0, 0)",
        [name],
    )
    .unwrap();
    let project = ProjectId(conn.last_insert_rowid());
    let path = repo.path().to_string_lossy().into_owned();
    conn.execute(
        "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                               store_key, presence, repo_kind)
         VALUES (?1, 'linux', ?2, ?2, ?3, 'store', 'present', 'worktree')",
        rusqlite::params![project.0, path.as_bytes(), path],
    )
    .unwrap();
    let location = LocationId(conn.last_insert_rowid());
    drop(guard);
    Copy {
        repo,
        project,
        location,
    }
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let index = Arc::new(Mutex::new(Index::open_at(dir.path(), 0).unwrap()));
    let gone = plant(&index, committed_repo(), "gone");
    let live = plant(&index, committed_repo(), "live");

    let events = Arc::new(RecordingSink::default());
    let deps = JobDeps {
        git: Arc::new(SystemGit::new(
            Arc::new(gone.repo.exec()),
            Arc::new(GitSlots::new(4)),
            Arc::new(SystemClock::new()),
        )),
        clock: Arc::new(SystemClock::new()),
        cancel: CancelToken::new(),
        tz_offset_min: 0,
    };
    let runner = JobRunner::new(Arc::clone(&index), deps, events.clone());
    Rig {
        _dir: dir,
        index,
        runner,
        events,
        gone,
        live,
    }
}

fn job(copy: &Copy, kind: JobKind) -> Job {
    Job {
        kind,
        project_id: copy.project,
        location_id: copy.location,
        store_key: "store".to_owned(),
        store_kind: StoreClass::Local,
        priority: Priority::RefState,
        not_before: 0,
        origin: JobOrigin::Walk,
    }
}

/// `locations.uninstall`'s own row write: `removed_at` and the columns it clears. `presence`
/// stays `present`, which is the shape the removal leaves.
fn uninstall_row(conn: &rusqlite::Connection, location: LocationId) {
    let clears = codotheca_core::uninstall::command::cleared_columns()
        .iter()
        .map(|column| format!("{column} = NULL"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute(
        &format!("UPDATE location SET removed_at = 5, {clears} WHERE id = ?1"),
        [location.0],
    )
    .unwrap();
}

fn uninstall(rig: &Rig, location: LocationId) {
    let guard = rig.index.lock().unwrap();
    uninstall_row(guard.conn(), location);
    drop(guard);
}

/// The user declares the whole project removed; its copy's row is untouched.
fn remove_project(rig: &Rig, project: ProjectId) {
    let guard = rig.index.lock().unwrap();
    guard
        .conn()
        .execute(
            "UPDATE project SET removed_at = 5 WHERE id = ?1",
            [project.0],
        )
        .unwrap();
    drop(guard);
}

/// The `scan/job_done` frames published for `project`, as job slugs.
fn done_for(rig: &Rig, project: ProjectId) -> Vec<String> {
    rig.events
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|(topic, event, payload)| {
            topic == "scan" && event == "job_done" && payload["projectId"] == project.0
        })
        .map(|(_, _, payload)| payload["job"].as_str().unwrap().to_owned())
        .collect()
}

/// Poll until `f` holds or the deadline passes. The pool is asynchronous by construction.
fn wait_for(deadline: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if f() {
            return true;
        }
        if start.elapsed() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Starts the pool and waits until the live copy's J1 has been reported and nothing is due, so
/// the gone copy's job has been taken off the queue one way or the other.
fn run_to_idle(rig: &Rig) {
    rig.runner.start(1);
    let settled = wait_for(Duration::from_secs(60), || {
        done_for(rig, rig.live.project).iter().any(|j| j == "j1") && rig.runner.is_idle()
    });
    rig.runner.request_stop();
    rig.runner.join();
    assert!(
        settled,
        "the live copy's J1 never ran, so the harness saw nothing"
    );
}

/// Nothing ran for the gone copy: no `job_done`, no job state, and J1's own column unwritten.
fn assert_nothing_ran(rig: &Rig) {
    let done = done_for(rig, rig.gone.project);
    let live = done_for(rig, rig.live.project);
    eprintln!("job_done frames: gone copy {done:?}, live copy {live:?}");
    assert!(
        live.iter().any(|j| j == "j1"),
        "the live copy's J1 is not in the events"
    );
    assert_eq!(
        done,
        Vec::<String>::new(),
        "a job ran for a removed copy: job_done {done:?}"
    );
    let guard = rig.index.lock().unwrap();
    let conn = guard.conn();
    let states: i64 = conn
        .query_row(
            "SELECT count(*) FROM project_job_state WHERE project_id = ?1",
            [rig.gone.project.0],
            |r| r.get(0),
        )
        .unwrap();
    let observed: Option<i64> = conn
        .query_row(
            "SELECT refstate_observed_at FROM location WHERE id = ?1",
            [rig.gone.location.0],
            |r| r.get(0),
        )
        .unwrap();
    drop(guard);
    assert_eq!(states, 0, "a job state was written for the removed copy");
    assert_eq!(observed, None, "J1 wrote the removed copy's refstate");
}

#[test]
fn ac_p4_46_20_no_job_runs_for_a_removed_location() {
    let rig = rig();
    uninstall(&rig, rig.gone.location);
    assert!(rig.runner.enqueue(job(&rig.gone, JobKind::J1Refstate)));
    assert!(rig.runner.enqueue(job(&rig.live, JobKind::J1Refstate)));
    run_to_idle(&rig);
    assert_nothing_ran(&rig);
}

#[test]
fn ac_p4_46_20_no_job_runs_for_any_location_of_a_removed_project() {
    let rig = rig();
    remove_project(&rig, rig.gone.project);
    assert!(rig.runner.enqueue(job(&rig.gone, JobKind::J1Refstate)));
    assert!(rig.runner.enqueue(job(&rig.live, JobKind::J1Refstate)));
    run_to_idle(&rig);
    assert_nothing_ran(&rig);
}

/// What separates a dispatch-time gate from an enqueue-time one: the job is queued while the copy
/// is live, the copy is removed, and only then does the pool start.
#[test]
fn ac_p4_46_20_a_job_queued_before_the_removal_does_not_run() {
    let rig = rig();
    assert!(rig.runner.enqueue(job(&rig.gone, JobKind::J1Refstate)));
    assert!(rig.runner.enqueue(job(&rig.live, JobKind::J1Refstate)));
    uninstall(&rig, rig.gone.location);
    run_to_idle(&rig);
    assert_nothing_ran(&rig);
}

/// J5 reads no repository and a removed project keeps its art, so its card can still be redrawn;
/// J1 on the same copy does not run.
#[test]
fn ac_p4_46_20_j5_art_still_runs_for_a_removed_project() {
    let rig = rig();
    uninstall(&rig, rig.gone.location);
    remove_project(&rig, rig.gone.project);
    assert!(rig.runner.enqueue(job(&rig.gone, JobKind::J5Art)));
    assert!(rig.runner.enqueue(job(&rig.gone, JobKind::J1Refstate)));
    assert!(rig.runner.enqueue(job(&rig.live, JobKind::J1Refstate)));
    run_to_idle(&rig);
    let done = done_for(&rig, rig.gone.project);
    eprintln!("job_done frames for the removed project: {done:?}");
    assert_eq!(done, ["j5"], "J5 alone runs for a removed project");
}

/// A page rig: project 1's only copy is the one each test removes, or whose project it removes;
/// project 2's copy is live and is the control.
fn page_rig() -> detail_rig::Rig {
    let rig = detail_rig::Rig::new();
    rig.project(1, "gone");
    rig.location(1, 1, "/srv/work/gone", "present", Some("main"), None, None);
    rig.project(2, "live");
    rig.location(2, 2, "/srv/work/live", "present", Some("main"), None, None);
    rig
}

/// Every §6 reading the rig's commands asked for, as `(project, location)`.
fn asked(rig: &detail_rig::Rig) -> Vec<(i64, i64)> {
    rig.jobs.visible.lock().unwrap().clone()
}

/// A project whose only copy is removed still names that copy as its primary, so its page shows
/// it. The page answers from what is stored and asks for no reading of it.
#[test]
fn ac_p4_46_20_a_gone_copys_page_queues_no_job() {
    let rig = page_rig();
    uninstall_row(rig.conn(), LocationId(1));

    let detail = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("the page answers");
    let shown = serde_json::to_value(&detail).unwrap();
    eprintln!(
        "removed copy's page: row {}, asked {:?}",
        shown["row"]["id"],
        asked(&rig)
    );
    assert_eq!(shown["row"]["id"], 1);
    assert_eq!(
        asked(&rig),
        [],
        "the page asked a reading of a removed copy"
    );

    handle_project_get(&rig.ctx(), json!({ "id": 2 })).expect("the live page answers");
    assert_eq!(asked(&rig), [(2, 2)], "the live copy's page asks once");
}

/// A removed project's copy is not removed, and is still not read: the Peek answers and asks for
/// nothing. The live project's Peek, in the same rig, asks once.
#[test]
fn ac_p4_46_20_a_removed_projects_peek_queues_no_job() {
    let rig = page_rig();
    rig.conn()
        .execute("UPDATE project SET removed_at = 5 WHERE id = 1", [])
        .unwrap();
    let ctx = ProjectsCtx {
        index: &rig.index,
        events: &rig.sink,
        jobs: &rig.jobs,
        mounts: &rig.mount,
        sync: &NullSyncSink,
        now: detail_rig::NOW,
        tz_offset_min: 0,
    };

    let peek = dispatch_projects_command(&ctx, "projects.peek", json!({ "id": 1 }))
        .expect("owned")
        .expect("the Peek answers");
    eprintln!(
        "removed project's Peek: id {}, asked {:?}",
        peek["id"],
        asked(&rig)
    );
    assert_eq!(peek["id"], 1);
    assert_eq!(
        asked(&rig),
        [],
        "the Peek asked a reading for a removed project"
    );

    dispatch_projects_command(&ctx, "projects.peek", json!({ "id": 2 }))
        .expect("owned")
        .expect("the live Peek answers");
    assert_eq!(asked(&rig), [(2, 2)], "the live project's Peek asks once");
}

/// The sites outside `projects/current.rs` whose statements name `project.removed_at`, each
/// `(path under core/src, flagged literals, why)`. Every one is a writer. A later writer of the
/// column appends its own entry, and a count that no longer matches the tree fails: a stale list
/// is a false claim.
const PROJECT_REMOVED_AT_SITES: &[(&str, usize, &str)] = &[
    (
        "identity/store.rs",
        1,
        "`clear_removed_on_find` takes the removal back when a copy of the same lineage is found",
    ),
    (
        "identity/merge.rs",
        1,
        "§1.5's merge rule: the survivor stays removed only while it has no live copy",
    ),
    (
        "testing/sidecar.rs",
        1,
        "a testkit fixture plants a removed project for the sidecar's round trip",
    ),
];

/// The names a reader of the current state goes through.
const CONSUMERS: [&str; 4] = [
    "IS_CURRENT_SQL",
    "is_current_sql!(",
    "is_current(",
    "location_is_current(",
];

/// What one pass of the audit read and found.
struct Audit {
    files: usize,
    literals: usize,
    /// `(path under the root, literal)` for every literal that reads `project.removed_at`.
    flagged: Vec<(String, String)>,
    /// `(path under the root, occurrences)` of the current-state names, per file that has any.
    consumers: Vec<(String, usize)>,
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `pattern` starts at `chars[i]`.
fn starts_at(chars: &[char], i: usize, pattern: &str) -> bool {
    pattern
        .chars()
        .enumerate()
        .all(|(k, c)| chars.get(i + k) == Some(&c))
}

/// Where a raw string starting at `chars[i]` opens: its `#` count and the index of its `"`. An `r`
/// opens one only at the start of a token, or after a `b` that is.
fn raw_string_at(chars: &[char], i: usize) -> Option<(usize, usize)> {
    if chars.get(i) != Some(&'r') {
        return None;
    }
    let before = |back: usize| i.checked_sub(back).and_then(|j| chars.get(j)).copied();
    let token_start = match before(1) {
        Some('b') => !matches!(before(2), Some(c) if is_ident(c)),
        Some(c) => !is_ident(c),
        None => true,
    };
    if !token_start {
        return None;
    }
    let mut open = i + 1;
    while chars.get(open) == Some(&'#') {
        open += 1;
    }
    (chars.get(open) == Some(&'"')).then_some((open - i - 1, open))
}

/// A `"…"` literal's text from just past its opening quote, and the index after its closing one.
fn quoted(chars: &[char], start: usize) -> (String, usize) {
    let mut text = String::new();
    let mut i = start;
    while let Some(&c) = chars.get(i) {
        i += 1;
        match c {
            '"' => break,
            '\\' => match chars.get(i) {
                // A line continuation drops the line break and the next line's indent.
                Some('\n' | '\r') => {
                    while chars.get(i).is_some_and(|next| next.is_whitespace()) {
                        i += 1;
                    }
                }
                Some('n' | 'r' | 't') => {
                    text.push(' ');
                    i += 1;
                }
                Some(&escaped) => {
                    text.push(escaped);
                    i += 1;
                }
                None => {}
            },
            _ => text.push(c),
        }
    }
    (text, i)
}

/// Whether the word at `chars[i]` is `word`, not the start of a longer identifier.
fn word_at(chars: &[char], i: usize, word: &str) -> bool {
    starts_at(chars, i, word) && !chars.get(i + word.len()).is_some_and(|&c| is_ident(c))
}

/// Whether the `#[cfg(test)]` ending at `chars[i]` gates a `mod`: past whitespace, any further
/// outer attributes and a `pub` or `pub(…)`, the next word is `mod`.
fn gates_mod(chars: &[char], mut i: usize) -> bool {
    loop {
        while chars.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        if !starts_at(chars, i, "#[") {
            break;
        }
        // An attribute ends where its brackets balance.
        let mut depth = 0_usize;
        while let Some(&c) = chars.get(i) {
            i += 1;
            if c == '[' {
                depth += 1;
            } else if c == ']' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
        }
    }
    if word_at(chars, i, "pub") {
        i += 3;
        while chars.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        if chars.get(i) == Some(&'(') {
            i = (i..chars.len())
                .find(|&j| chars[j] == ')')
                .map_or(chars.len(), |j| j + 1);
        }
        while chars.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
    }
    word_at(chars, i, "mod")
}

/// One file's code and string literals: comments dropped, each literal's text taken out of the
/// code, and the file cut at its first `#[cfg(test)]` that gates a `mod`, where its tests begin.
/// One gating anything else, a statement or a single item, cuts nothing.
fn split_source(source: &str) -> (String, Vec<String>) {
    let chars: Vec<char> = source.chars().collect();
    let mut code = String::new();
    let mut literals = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if starts_at(&chars, i, "#[cfg(test)]") && gates_mod(&chars, i + "#[cfg(test)]".len()) {
            break;
        }
        if starts_at(&chars, i, "//") {
            while chars.get(i).is_some_and(|&next| next != '\n') {
                i += 1;
            }
        } else if starts_at(&chars, i, "/*") {
            // A block comment may hold another, and ends only where the outer one closes.
            let mut depth = 0_usize;
            while i < chars.len() {
                if starts_at(&chars, i, "/*") {
                    depth += 1;
                    i += 2;
                } else if starts_at(&chars, i, "*/") {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if let Some((hashes, open)) = raw_string_at(&chars, i) {
            let close = (open + 1..chars.len())
                .find(|&j| chars[j] == '"' && (1..=hashes).all(|k| chars.get(j + k) == Some(&'#')))
                .unwrap_or(chars.len());
            literals.push(chars[open + 1..close].iter().collect());
            code.push_str("\"\"");
            i = close + 1 + hashes;
        } else if c == '"' {
            let (literal, next) = quoted(&chars, i + 1);
            literals.push(literal);
            code.push_str("\"\"");
            i = next;
        } else {
            // `'x'` and `'\…'` are characters, so `'"'` opens no string; a `'` with no closing
            // quote two along is a lifetime or a label.
            let char_end = if c != '\'' {
                None
            } else if chars.get(i + 1) == Some(&'\\') {
                (i + 3..chars.len()).find(|&j| chars[j] == '\'')
            } else {
                (chars.get(i + 2) == Some(&'\'')).then_some(i + 2)
            };
            if let Some(end) = char_end {
                i = end + 1;
                continue;
            }
            code.push(c);
            i += 1;
        }
    }
    (code, literals)
}

/// A literal as SQL words and single punctuation marks, lower-cased, whitespace dropped.
fn sql_tokens(literal: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    for c in literal.chars().flat_map(char::to_lowercase) {
        if is_ident(c) {
            word.push(c);
            continue;
        }
        if !word.is_empty() {
            tokens.push(std::mem::take(&mut word));
        }
        if !c.is_whitespace() {
            tokens.push(c.to_string());
        }
    }
    if !word.is_empty() {
        tokens.push(word);
    }
    tokens
}

/// Whether a literal reads `project.removed_at`: it names the `project` table, and a `removed_at`
/// in it is not qualified by a name the same literal binds to `location`. `project_dependency` is
/// one word, so it never names `project`.
fn reads_project_removed_at(literal: &str) -> bool {
    let tokens = sql_tokens(literal);
    let word = |i: usize| tokens.get(i).map_or("", String::as_str);
    let names_project = (0..tokens.len())
        .any(|i| ["from", "join", "update", "into"].contains(&word(i)) && word(i + 1) == "project");
    if !names_project {
        return false;
    }
    // `FROM location l`, `JOIN location AS l`, and the table's own name.
    let mut location = Vec::new();
    for i in 0..tokens.len() {
        if ["from", "join"].contains(&word(i)) && word(i + 1) == "location" {
            let alias = if word(i + 2) == "as" {
                word(i + 3)
            } else {
                word(i + 2)
            };
            location.extend(["location", alias]);
        }
    }
    (0..tokens.len()).any(|i| {
        word(i) == "removed_at"
            && !(i >= 2 && word(i - 1) == "." && location.contains(&word(i - 2)))
    })
}

/// Occurrences of `name` in `code` that do not continue an identifier.
fn count_name(code: &str, name: &str) -> usize {
    code.match_indices(name)
        .filter(|&(at, _)| {
            let before = code.get(..at).and_then(|head| head.chars().next_back());
            !matches!(before, Some(c) if is_ident(c))
        })
        .count()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("the tree reads").flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The audit over every `.rs` under `root` but `projects/current.rs`, where the predicate is
/// written.
fn audit(root: &Path) -> Audit {
    let mut files = Vec::new();
    walk(root, &mut files);
    files.sort();
    let mut found = Audit {
        files: 0,
        literals: 0,
        flagged: Vec::new(),
        consumers: Vec::new(),
    };
    for file in files {
        let path = file
            .strip_prefix(root)
            .expect("under the root")
            .to_string_lossy()
            .replace('\\', "/");
        if path == "projects/current.rs" {
            continue;
        }
        let source = std::fs::read_to_string(&file).expect("a source file reads");
        let (code, literals) = split_source(&source);
        found.files += 1;
        found.literals += literals.len();
        found.flagged.extend(
            literals
                .into_iter()
                .filter(|literal| reads_project_removed_at(literal))
                .map(|literal| (path.clone(), literal)),
        );
        let consumers: usize = CONSUMERS.iter().map(|name| count_name(&code, name)).sum();
        if consumers > 0 {
            found.consumers.push((path, consumers));
        }
    }
    found
}

/// Planted sources, `(file, flagged, text)`: each one shape the audit must read right.
const PLANTED: [(&str, bool, &str); 11] = [
    (
        "alias_read.rs",
        true,
        r#"fn f() { let _ = "SELECT id FROM project p WHERE p.removed_at IS NULL"; }"#,
    ),
    (
        "raw_unqualified.rs",
        true,
        r##"fn f() { let _ = r#"SELECT id FROM project WHERE removed_at IS NULL"#; }"##,
    ),
    (
        // A `"` in a character, and escaped quotes inside the literal, keep it one literal.
        "escaped_quotes.rs",
        true,
        r#"fn f<'a>(_: &'a str) {
    let _q = '"';
    let _ = "SELECT id FROM project WHERE note = \"x\" AND removed_at IS NULL";
}"#,
    ),
    (
        "location_read.rs",
        false,
        r#"fn f() {
    let _ = "SELECT l.id FROM location l JOIN project p ON p.id = l.project_id
              WHERE l.removed_at IS NULL";
}"#,
    ),
    (
        "line_comment.rs",
        false,
        r#"// let _ = "SELECT id FROM project p WHERE p.removed_at IS NULL";
fn f() {}"#,
    ),
    (
        "nested_comment.rs",
        false,
        r#"/* outer /* inner */ "SELECT id FROM project p WHERE p.removed_at IS NULL" */
fn f() {}"#,
    ),
    (
        "after_cfg_test.rs",
        false,
        r#"fn f() {}
#[cfg(test)]
mod tests {
    fn g() { let _ = "SELECT id FROM project p WHERE p.removed_at IS NULL"; }
}"#,
    ),
    (
        "after_attributed_pub_mod.rs",
        false,
        r#"fn f() {}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    fn g() { let _ = "SELECT id FROM project p WHERE p.removed_at IS NULL"; }
}"#,
    ),
    (
        // A `#[cfg(test)]` on one statement leaves the production code after it in scope.
        "statement_cfg_test.rs",
        true,
        r#"fn f() {
    #[cfg(test)]
    let _probe = 1;
    let _ = "SELECT id FROM project p WHERE p.removed_at IS NULL";
}"#,
    ),
    (
        "dependency.rs",
        false,
        r#"fn f() { let _ = "SELECT d.id FROM project_dependency d WHERE d.removed_at IS NULL"; }"#,
    ),
    (
        "consumers.rs",
        false,
        r#"fn f(conn: &Connection) {
    let _ = crate::projects::current::location_is_current(conn, 1);
    let _ = crate::projects::current::is_current(conn, 2);
    let _ = crate::projects::current::IS_CURRENT_SQL;
    // crate::projects::current::is_current(conn, 3) in a comment is no consumer
    let _ = "is_current(";
}"#,
    ),
];

#[test]
fn the_audit_flags_a_planted_hand_write_and_passes_a_location_read() {
    let dir = tempfile::tempdir().unwrap();
    let planted = PLANTED;
    for (name, _, source) in &planted {
        std::fs::write(dir.path().join(name), source).unwrap();
    }

    let found = audit(dir.path());
    let mut flagged: Vec<&str> = found
        .flagged
        .iter()
        .map(|(path, _)| path.as_str())
        .collect();
    for (name, _, _) in &planted {
        let verdict = if flagged.contains(name) {
            "flagged"
        } else {
            "passed"
        };
        eprintln!("{name}: {verdict}");
    }
    eprintln!(
        "files scanned: {}, literals scanned: {}, consumers: {:?}",
        found.files, found.literals, found.consumers
    );
    let mut expected: Vec<&str> = planted
        .iter()
        .filter(|(_, flag, _)| *flag)
        .map(|(name, _, _)| *name)
        .collect();
    flagged.sort_unstable();
    expected.sort_unstable();
    assert_eq!(flagged, expected);
    assert_eq!(found.files, planted.len());
    assert_eq!(found.consumers, [("consumers.rs".to_owned(), 3)]);
}

#[test]
fn ac_p4_46_20_the_predicate_is_written_in_one_place() {
    let found = audit(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
    let consumers: usize = found.consumers.iter().map(|(_, n)| n).sum();
    let mut sites: BTreeMap<&str, usize> = BTreeMap::new();
    for (path, literal) in &found.flagged {
        let one_line = literal.split_whitespace().collect::<Vec<_>>().join(" ");
        eprintln!("flagged: {path}: {one_line}");
        *sites.entry(path.as_str()).or_default() += 1;
    }
    eprintln!(
        "files scanned: {}, literals scanned: {}, flagged literals: {}, permitted sites: {}, \
         consumers: {consumers} {:?}",
        found.files,
        found.literals,
        found.flagged.len(),
        PROJECT_REMOVED_AT_SITES.len(),
        found.consumers
    );
    // A gate whose passing run scans nothing is a failing gate.
    assert!(found.files > 0, "no source file was scanned");
    assert!(found.literals > 0, "no string literal was scanned");
    assert!(
        consumers > 0,
        "nothing reads the current state through its helper"
    );

    let unlisted: Vec<&str> = sites
        .keys()
        .copied()
        .filter(|path| {
            !PROJECT_REMOVED_AT_SITES
                .iter()
                .any(|(site, _, _)| site == path)
        })
        .collect();
    assert_eq!(
        unlisted,
        Vec::<&str>::new(),
        "project.removed_at is written by hand outside projects/current.rs"
    );
    let stale: Vec<(&str, usize, usize)> = PROJECT_REMOVED_AT_SITES
        .iter()
        .map(|&(site, listed, _)| (site, listed, sites.get(site).copied().unwrap_or(0)))
        .filter(|(_, listed, measured)| listed != measured)
        .collect();
    assert_eq!(
        stale,
        [],
        "a listed site's count no longer matches the tree"
    );
    let unreasoned: Vec<&str> = PROJECT_REMOVED_AT_SITES
        .iter()
        .filter(|(_, _, why)| why.trim().is_empty())
        .map(|(site, _, _)| *site)
        .collect();
    assert_eq!(unreasoned, Vec::<&str>::new(), "a site carries no reason");
}
