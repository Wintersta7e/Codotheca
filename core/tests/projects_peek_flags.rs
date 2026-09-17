#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §8.4.1's Peek panel and §1.2's three organisation primitives.
//!
//! The argument key is `id`, not `projectId`: `protocol/schema/protocol.json` declares
//! `projects.peek {id}` and `projects.setFlags {id, isPinned?, …}`, the generated arg structs
//! carry `deny_unknown_fields`, and the schema is the authority (R14/R31). `projects.setFlags`
//! returns `Empty` for the same reason — the changed row travels as the `projects/flags_changed`
//! event, which is what the renderer's optimistic flip reconciles against.

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::index::Index;
use codotheca_core::projects::{dispatch_projects_command, ProjectsCtx};

const NOW: i64 = 1_781_179_200;

fn seeded() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    index
        .conn()
        .execute(
            "INSERT INTO project (id, name, seed_basename, last_touched_at, created_at, updated_at)
             VALUES (1, 'a', 'a', ?1, ?1, ?1), (2, 'b', 'b', ?1, ?1, ?1)",
            rusqlite::params![NOW],
        )
        .expect("seed");
    // [p2] Each project gets a **location**. §23.2 makes zero locations the *not-cloned* shape,
    // and §23.3 forbids Peek sending `not_indexed` for such a row — so a fixture with no
    // location is no longer a cloned project whose content pass has not run, which is what
    // every assertion in this file is about. The fixture was the wrong shape the moment §23
    // landed; it went on passing because nothing produced the other shape until now.
    index
        .conn()
        .execute(
            "INSERT INTO location (id, project_id, kind, distro, path_bytes, path_key,
                                   path_display, volume_key, store_key, presence, repo_kind)
             VALUES (10, 1, 'linux', '', X'2f61', X'2f61', '<project-path>/a', 'v', 's',
                     'present', 'worktree'),
                    (20, 2, 'linux', '', X'2f62', X'2f62', '<project-path>/b', 'v', 's',
                     'present', 'worktree')",
            [],
        )
        .expect("each project has a working copy");
    (dir, index)
}

fn call(
    index: &Index,
    sink: &CollectingSink,
    cmd: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    let jobs = codotheca_core::jobs::NullJobSink;
    let mounts = codotheca_core::testing::FakeMountResolver::default();
    let ctx = ProjectsCtx {
        index,
        events: sink,
        jobs: &jobs,
        mounts: &mounts,
        sync: &codotheca_core::sync::runner::NullSyncSink,
        now: NOW,
        tz_offset_min: 0,
    };
    dispatch_projects_command(&ctx, cmd, args)
        .expect("owned")
        .expect("ok")
}

#[test]
fn a_readme_that_was_never_indexed_is_not_a_readme_that_is_absent() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    let peek = call(
        &index,
        &sink,
        "projects.peek",
        serde_json::json!({ "id": 1 }),
    );
    assert_eq!(peek["readme"]["state"], "not_indexed");
    assert_eq!(peek["readme"]["readAt"], serde_json::Value::Null);

    index
        .conn()
        .execute(
            "INSERT INTO peek_cache (project_id, readme_excerpt, computed_at) VALUES (1, NULL, ?1)",
            rusqlite::params![NOW],
        )
        .expect("j6 ran and found nothing");
    let peek = call(
        &index,
        &sink,
        "projects.peek",
        serde_json::json!({ "id": 1 }),
    );
    assert_eq!(peek["readme"]["state"], "absent");
    assert_eq!(peek["readme"]["readAt"], NOW);
}

#[test]
fn a_fact_whose_job_has_not_run_is_null_and_playtime_zero_is_true() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    let peek = call(
        &index,
        &sink,
        "projects.peek",
        serde_json::json!({ "id": 1 }),
    );
    for key in [
        "birthYear",
        "primaryLanguage",
        "sizeTrackedBytes",
        "lastCommitAt",
    ] {
        assert_eq!(
            peek[key],
            serde_json::Value::Null,
            "{key} must be null, never 0"
        );
    }
    // §8.4.1's one exception: this ledger starts at install, so 0 is a measurement.
    assert_eq!(peek["playtimeSeconds"], 0);
    // §6: never "clean", only "no changes as of T" — and here there is no T.
    assert_eq!(peek["worktree"]["observedAt"], serde_json::Value::Null);
    assert_eq!(peek["worktree"]["isDirty"], serde_json::Value::Null);
    assert_eq!(peek["worktree"]["untrackedCount"], serde_json::Value::Null);
}

#[test]
fn peek_reports_and_does_not_phrase() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    let peek = call(
        &index,
        &sink,
        "projects.peek",
        serde_json::json!({ "id": 1 }),
    );
    let text = serde_json::to_string(&peek).expect("serialise");
    // §5.6/§8.4.1: the dry one-line note belongs inside an opened project card, and Peek is the
    // triage surface. No roast, no note, no completion furniture.
    for banned in ["roast", "notes", "completion"] {
        assert!(!text.contains(banned), "{banned} must not reach Peek");
    }
}

#[test]
fn set_flags_writes_three_columns_emits_one_event_and_touches_nothing_else() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    let before: i64 = index
        .conn()
        .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
        .expect("count");

    let reply = call(
        &index,
        &sink,
        "projects.setFlags",
        serde_json::json!({ "id": 1, "isPinned": true }),
    );
    // The schema says `Empty`: the flags travel on the event, not in the reply.
    assert_eq!(reply, serde_json::json!({}));

    // §17: phase 1 has no destructive operation. Nothing was removed, nothing else was written.
    let after: i64 = index
        .conn()
        .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
        .expect("count");
    assert_eq!(before, after);
    let jobs: i64 = index
        .conn()
        .query_row("SELECT COUNT(*) FROM project_job_state", [], |r| r.get(0))
        .expect("count");
    assert_eq!(jobs, 0, "setFlags must not reach scheduling (§11.1)");

    let events = sink.named("projects", "flags_changed");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["id"], 1);
    assert_eq!(events[0]["isPinned"], true);
    // An absent field leaves that flag alone; it does not reset it.
    assert_eq!(events[0]["isArchived"], false);
    assert_eq!(events[0]["isHidden"], false);
}

#[test]
fn an_absent_flag_is_left_alone_and_is_not_reset() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    call(
        &index,
        &sink,
        "projects.setFlags",
        serde_json::json!({ "id": 1, "isArchived": true }),
    );
    call(
        &index,
        &sink,
        "projects.setFlags",
        serde_json::json!({ "id": 1, "isPinned": true }),
    );
    let last = sink
        .named("projects", "flags_changed")
        .pop()
        .expect("two events");
    assert_eq!(last["isArchived"], true, "the second call did not reset it");
    assert_eq!(last["isPinned"], true);
}

#[test]
fn pinning_changes_no_sort_order() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    let before = call(&index, &sink, "projects.list", serde_json::json!({}));
    call(
        &index,
        &sink,
        "projects.setFlags",
        serde_json::json!({ "id": 2, "isPinned": true }),
    );
    let after = call(&index, &sink, "projects.list", serde_json::json!({}));

    // §7.8a's table is a table of `none`s: no band, no membership change, no reorder.
    assert_eq!(before["orderKey"], after["orderKey"]);
    assert_eq!(
        before["sections"]
            .as_array()
            .expect("s")
            .iter()
            .map(|s| &s["id"])
            .collect::<Vec<_>>(),
        after["sections"]
            .as_array()
            .expect("s")
            .iter()
            .map(|s| &s["id"])
            .collect::<Vec<_>>()
    );
    assert_eq!(before["sections"][0]["agg"], after["sections"][0]["agg"]);
    let ids = |p: &serde_json::Value| {
        p["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|r| r["id"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&before), ids(&after));
}

#[test]
fn an_unknown_project_is_refused_rather_than_silently_creating_one() {
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    let jobs = codotheca_core::jobs::NullJobSink;
    let mounts = codotheca_core::testing::FakeMountResolver::default();
    let ctx = ProjectsCtx {
        index: &index,
        events: &sink,
        jobs: &jobs,
        mounts: &mounts,
        sync: &codotheca_core::sync::runner::NullSyncSink,
        now: NOW,
        tz_offset_min: 0,
    };
    let err = dispatch_projects_command(
        &ctx,
        "projects.setFlags",
        serde_json::json!({ "id": 99, "isPinned": true }),
    )
    .expect("owned")
    .expect_err("refused");
    assert_eq!(err.code, codotheca_core::protocol::ErrorCode::Protocol);

    let err = dispatch_projects_command(&ctx, "projects.peek", serde_json::json!({ "id": 99 }))
        .expect("owned")
        .expect_err("refused");
    assert_eq!(err.code, codotheca_core::protocol::ErrorCode::Protocol);
    // Refused means nothing was created.
    let rows: i64 = index
        .conn()
        .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows, 2);
}

#[test]
fn a_merged_away_project_is_not_reachable_through_either_command() {
    // §1.5: `merged_into` is the redirect. The shelf's two commands answer for the survivor
    // only; following the redirect is plan 14's `projects.get`, which is a different surface.
    let (_dir, index) = seeded();
    let sink = CollectingSink::default();
    index
        .conn()
        .execute("UPDATE project SET merged_into = 2 WHERE id = 1", [])
        .expect("merge");
    let jobs = codotheca_core::jobs::NullJobSink;
    let mounts = codotheca_core::testing::FakeMountResolver::default();
    let ctx = ProjectsCtx {
        index: &index,
        events: &sink,
        jobs: &jobs,
        mounts: &mounts,
        sync: &codotheca_core::sync::runner::NullSyncSink,
        now: NOW,
        tz_offset_min: 0,
    };
    for command in ["projects.peek", "projects.setFlags"] {
        let err = dispatch_projects_command(&ctx, command, serde_json::json!({ "id": 1 }))
            .expect("owned")
            .expect_err("refused");
        assert_eq!(
            err.code,
            codotheca_core::protocol::ErrorCode::Protocol,
            "{command}"
        );
    }
}

// ---------------------------------------------------------------------------
// §6: a Peek asks for a current worktree reading. `on_visible` had no caller,
// which is what made "no changes as of T" a stale answer rather than a current one.
// ---------------------------------------------------------------------------

/// Every §6 request the command made.
#[derive(Debug, Default)]
struct RecordingJobs {
    visible: std::sync::Mutex<Vec<(i64, i64)>>,
}

impl codotheca_core::jobs::JobSink for RecordingJobs {
    fn on_location_indexed(
        &self,
        _: codotheca_core::protocol::ProjectId,
        _: codotheca_core::protocol::LocationId,
        _: &str,
        _: codotheca_core::mount::StoreClass,
    ) {
    }

    fn on_visible(
        &self,
        project: codotheca_core::protocol::ProjectId,
        location: codotheca_core::protocol::LocationId,
        _: &str,
        _: codotheca_core::mount::StoreClass,
        _: bool,
    ) {
        self.visible
            .lock()
            .expect("lock")
            .push((project.0, location.0));
    }
}

/// One project with one copy, whose worktree was observed a long time ago.
fn seeded_with_a_location() -> (tempfile::TempDir, Index) {
    let (dir, index) = seeded();
    index
        .conn()
        .execute(
            "INSERT INTO location
               (id, project_id, kind, path_bytes, path_key, path_display, store_key,
                presence, repo_kind, is_dirty, worktree_observed_at)
             VALUES (1, 1, 'linux', ?1, ?1, '/home/u/a', 'store-a', 'present', 'worktree', 0, ?2)",
            rusqlite::params![b"/home/u/a".to_vec(), NOW - 86_400],
        )
        .expect("seed location");
    (dir, index)
}

#[test]
fn a_peek_asks_for_one_fresh_reading_and_a_list_asks_for_none() {
    let (_dir, index) = seeded_with_a_location();
    let sink = CollectingSink::default();
    let jobs = RecordingJobs::default();
    let mounts = codotheca_core::testing::FakeMountResolver::new();
    let ctx = ProjectsCtx {
        index: &index,
        events: &sink,
        jobs: &jobs,
        mounts: &mounts,
        sync: &codotheca_core::sync::runner::NullSyncSink,
        now: NOW,
        tz_offset_min: 0,
    };

    let peek = dispatch_projects_command(&ctx, "projects.peek", serde_json::json!({ "id": 1 }))
        .expect("owned")
        .expect("ok");
    assert_eq!(
        jobs.visible.lock().expect("lock").clone(),
        vec![(1, 1)],
        "§6: an opened Peek asks for a current reading for the copy it is showing"
    );

    // …and the answer is still the **stored** one, with its own `as_of`. Claiming the time of
    // the call would turn a day-old reading into a fresh one on the wire.
    assert_eq!(
        peek["worktree"]["observedAt"],
        serde_json::json!(NOW - 86_400)
    );
    assert_eq!(peek["worktree"]["isDirty"], serde_json::json!(false));

    dispatch_projects_command(&ctx, "projects.list", serde_json::json!({}))
        .expect("owned")
        .expect("ok");
    assert_eq!(
        jobs.visible.lock().expect("lock").len(),
        1,
        "§8: a shelf of a thousand rows must not queue a thousand status jobs, and the \
         virtualizer means `in the page` is not `on screen`"
    );
}
