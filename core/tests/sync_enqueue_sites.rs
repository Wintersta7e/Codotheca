#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! **AC-P2-21-13** — `project_remote` is enqueued from exactly the two visibility call sites, and
//! **at no third**.
//!
//! The arithmetic that decides on-demand over a sweep (§21.5): a background sweep of a
//! 400-project library over §25's four per-repo forge fields is **400 × 4 = 1,600 calls against
//! 5,000 an hour** — a third of the allowance spent on tiles nobody opened, and stale again before
//! anyone opens one.

use std::sync::Mutex;

use codotheca_core::detail::DetailCtx;
use codotheca_core::index::Index;
use codotheca_core::projects::ProjectsCtx;
use codotheca_core::proto::EventSink;
use codotheca_core::protocol::ProjectId;
use codotheca_core::sync::runner::SyncSink;
use codotheca_core::testing::TempIndex;

const NOW: i64 = 1_800_000_000;

/// Records every project the product asked for remote facts about.
#[derive(Debug, Default)]
struct RecordingSink {
    seen: Mutex<Vec<ProjectId>>,
}

impl SyncSink for RecordingSink {
    fn on_project_visible(&self, project: ProjectId) {
        self.seen.lock().expect("sink").push(project);
    }
}

#[derive(Debug)]
struct Quiet;

impl EventSink for Quiet {
    fn emit(&self, _topic: &str, _event: &str, _payload: serde_json::Value) {}
}

/// Every `.rs` under `core/src/`, with its text. **Panics on an empty walk**: every assertion
/// below is of the form *"this string appears N times"*, and zero files satisfies any N of zero
/// vacuously. A gate whose passing run scans nothing is a failing gate.
fn rust_sources() -> Vec<(std::path::PathBuf, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("core/src is readable") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("readable");
                out.push((path, text));
            }
        }
    }
    assert!(!out.is_empty(), "the walk found no sources under core/src");
    out
}

/// A path relative to `core/src/`, **always with `/` separators**.
///
/// Windows renders the same path as `detail\\get.rs`, and a literal written `detail/get.rs` then
/// fails a comparison that passes in WSL. That defect class — a POSIX path literal compared
/// against a native path — has fired four times in this repository and is reachable by no gate
/// that runs on one platform only.
fn relative(path: &std::path::Path) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    path.strip_prefix(&root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// **AC-P2-21-13.** Exactly two call sites, and they are the two §21.5 names.
///
/// The seam's own declarations are excluded by matching `fn on_project_visible` first: a trait
/// method and its two implementations are not call sites, and counting them would make the
/// assertion pass for the wrong reason.
#[test]
fn project_remote_is_enqueued_from_exactly_the_two_visibility_sites() {
    let sources = rust_sources();
    let mut sites: Vec<String> = Vec::new();
    let mut occurrences = 0_usize;
    for (path, text) in &sources {
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("//") || line.starts_with("///") {
                continue;
            }
            if !line.contains("on_project_visible(") {
                continue;
            }
            // A declaration or an implementation, not a call.
            if line.contains("fn on_project_visible(") {
                continue;
            }
            occurrences += 1;
            sites.push(relative(path));
        }
    }
    sites.sort();
    sites.dedup();
    eprintln!(
        "sync_enqueue_sites: scanned {} file(s), found {occurrences} call site(s) in {sites:?}",
        sources.len()
    );
    assert_eq!(
        occurrences, 2,
        "§21.5 names two sites and no third: {sites:?}"
    );
    assert_eq!(sites, ["detail/get.rs", "projects/peek.rs"]);
}

fn fixture() -> (TempIndex, ProjectId) {
    let fixture = TempIndex::new();
    let project = fixture.insert_project();
    (fixture, project)
}

fn detail_ctx<'a>(
    index: &'a Index,
    sync: &'a dyn SyncSink,
    jobs: &'a dyn codotheca_core::jobs::JobSink,
    git: &'a dyn codotheca_core::git::GitBackend,
    mounts: &'a dyn codotheca_core::mount::MountResolver,
    events: &'a dyn EventSink,
) -> DetailCtx<'a> {
    DetailCtx {
        index,
        git,
        mount: mounts,
        events,
        jobs,
        sync,
        now: NOW,
    }
}

/// `projects.get` and `projects.peek` each enqueue **once**, and `projects.list` enqueues
/// **nothing** — a shelf of a thousand rows must not queue a thousand network tasks.
#[test]
fn the_two_commands_enqueue_once_each_and_the_shelf_enqueues_nothing() {
    let (fixture, project) = fixture();
    let sink = RecordingSink::default();
    let jobs = codotheca_core::jobs::NullJobSink;
    let git = codotheca_core::testing::FakeGitBackend::new();
    let mounts = codotheca_core::testing::FakeMountResolver::default();
    let events = Quiet;

    let ctx = detail_ctx(fixture.index(), &sink, &jobs, &git, &mounts, &events);
    let _ = codotheca_core::detail::dispatch_detail_command(
        &ctx,
        "projects.get",
        serde_json::json!({ "id": project.0 }),
    );
    assert_eq!(sink.seen.lock().expect("sink").len(), 1, "projects.get");

    let projects = ProjectsCtx {
        index: fixture.index(),
        events: &events,
        jobs: &jobs,
        mounts: &mounts,
        sync: &sink,
        now: NOW,
        tz_offset_min: 0,
    };
    let _ = codotheca_core::projects::dispatch_projects_command(
        &projects,
        "projects.peek",
        serde_json::json!({ "id": project.0 }),
    );
    assert_eq!(sink.seen.lock().expect("sink").len(), 2, "projects.peek");

    let _ = codotheca_core::projects::dispatch_projects_command(
        &projects,
        "projects.list",
        serde_json::json!({}),
    );
    assert_eq!(
        sink.seen.lock().expect("sink").len(),
        2,
        "projects.list queued a network task per row"
    );
}

/// **The trap this task exists for.** Both existing visibility calls sit inside a location guard,
/// and a project with **no location** is exactly the not-cloned project §23 introduces — the case
/// that most needs its remote facts. The enqueue therefore goes **outside** the guard, keyed on
/// the project alone.
#[test]
fn a_project_with_no_location_still_enqueues_from_both_commands() {
    let (fixture, project) = fixture();
    let locations: i64 = fixture
        .index()
        .conn()
        .query_row("SELECT count(*) FROM location", [], |r| r.get(0))
        .expect("counted");
    assert_eq!(locations, 0, "the fixture must have no working copy at all");

    let sink = RecordingSink::default();
    let jobs = codotheca_core::jobs::NullJobSink;
    let git = codotheca_core::testing::FakeGitBackend::new();
    let mounts = codotheca_core::testing::FakeMountResolver::default();
    let events = Quiet;

    let ctx = detail_ctx(fixture.index(), &sink, &jobs, &git, &mounts, &events);
    let _ = codotheca_core::detail::dispatch_detail_command(
        &ctx,
        "projects.get",
        serde_json::json!({ "id": project.0 }),
    );
    let projects = ProjectsCtx {
        index: fixture.index(),
        events: &events,
        jobs: &jobs,
        mounts: &mounts,
        sync: &sink,
        now: NOW,
        tz_offset_min: 0,
    };
    let _ = codotheca_core::projects::dispatch_projects_command(
        &projects,
        "projects.peek",
        serde_json::json!({ "id": project.0 }),
    );

    let seen = sink.seen.lock().expect("sink").clone();
    assert_eq!(
        seen,
        vec![project, project],
        "the one surface made entirely of remote facts never asked for any"
    );
}

/// `NullSyncSink` queues nothing, which is what a build with no runner should do — and it is what
/// makes a core with zero accounts fully functional rather than merely quiet.
#[test]
fn the_null_sink_queues_nothing() {
    let sink = codotheca_core::sync::runner::NullSyncSink;
    sink.on_project_visible(ProjectId(1));
    // Nothing to assert but that it is total and infallible; the value is that the composition
    // root has a real alternative to the runner rather than an `Option` every caller unwraps.
    let _: &dyn SyncSink = &sink;
}

/// The runner is a `SyncSink`, so `SyncPump::sink_ref` hands the two call sites the real thing.
/// Without this the seam would be a trait whose only implementation is a fake — R1's defect, which
/// has cost this project five rulings.
#[test]
fn the_production_sink_is_the_runner_itself() {
    let sources = rust_sources();
    let mut impls = 0_usize;
    for (_, text) in &sources {
        impls += text.matches("impl SyncSink for ").count();
    }
    eprintln!("sync_enqueue_sites: {impls} SyncSink implementation(s) in core/src");
    assert_eq!(
        impls, 2,
        "the runner and the null sink, and nothing else pretending to be either"
    );
    let runner = sources
        .iter()
        .any(|(_, text)| text.contains("impl SyncSink for SyncRunner"));
    assert!(runner, "the seam has no production implementation");
}

/// The two contexts carry the seam by **reference**, so the composition root decides what is
/// behind it and a module cannot construct its own.
#[test]
fn neither_context_can_build_its_own_sink() {
    let sources = rust_sources();
    let mut scanned = 0_usize;
    for (path, text) in &sources {
        let name = relative(path);
        if name != "detail/mod.rs" && name != "projects/mod.rs" {
            continue;
        }
        scanned += 1;
        assert!(
            text.contains("pub sync: &'a dyn crate::sync::runner::SyncSink"),
            "{name} does not take the seam by reference"
        );
    }
    eprintln!("sync_enqueue_sites: {scanned} context module(s) checked");
    assert_eq!(scanned, 2);
}
