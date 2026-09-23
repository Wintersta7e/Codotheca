//! §30.5 and §27.7 — `acknowledged_at`'s **production writer**, and the enrolment predicate it
//! feeds.
//!
//! The column has been declared since `0001_meta_and_projects.sql:102`, read by the new-arrival
//! predicate, projected onto every row and restored by the sidecar — and written by nothing.
//! The settled backlog-suppression gate reads it, so without this writer health ships
//! **suppressed for every project, for ever, with every test around it green.**
#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/detail_rig.rs"]
mod detail_rig;

use codotheca_core::detail::get::handle_project_get;
use codotheca_core::health::acknowledge::{stamp_acknowledged, stamp_and_read_enrolment};
use codotheca_core::health::enrolment::is_enrolled;
use codotheca_core::health::read_for_project;
use codotheca_core::protocol::{HealthState, ProjectId};
use detail_rig::{Rig, NOW};
use serde_json::json;

fn acknowledged_at(rig: &Rig, id: i64) -> Option<i64> {
    rig.conn()
        .query_row(
            "SELECT acknowledged_at FROM project WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .expect("read acknowledged_at")
}

/// §30.5's write-once shape. `projects.get` and `projects.launch` stamp; a replay is a no-op, so
/// `projects.get` keeps the `read` classification `app/src/main/core/idempotence.ts:41` gives it,
/// and a second open can never move the time the user first acknowledged the project.
///
/// **The launch half is `ac_p3_30_8_launching_a_project_stamps_acknowledged_at_once` in
/// `core/tests/commands_launch.rs`**, where the launch fixture already lives: a second copy of
/// that rig here would be a hundred and fifty lines of setup stated twice.
#[test]
fn ac_p3_30_8_projects_get_stamps_once_and_never_moves_a_non_null() {
    let rig = Rig::new();
    rig.project(1, "alpha");
    assert_eq!(acknowledged_at(&rig, 1), None, "seeded unenrolled");

    handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("first get");
    assert_eq!(
        acknowledged_at(&rig, 1),
        Some(NOW),
        "the first projects.get stamps"
    );

    // A direct replay of the writer at a later clock, which is what a second open is.
    {
        let tx = rig.conn().unchecked_transaction().unwrap();
        let stamped = stamp_acknowledged(&tx, ProjectId(1), NOW + 9_000).unwrap();
        tx.commit().unwrap();
        assert!(!stamped, "a replay stamps nothing");
    }
    assert_eq!(
        acknowledged_at(&rig, 1),
        Some(NOW),
        "write-once: the stored time never moves"
    );
}

/// §10.5a — **reading a card is not acknowledging it.** `acknowledges()`
/// (`app/src/renderer/firstrun/newArrivals.ts:45-47`) returns true for `openedProjectPage` and
/// `launched` and false for `peeked`, and this writer mirrors that vocabulary: clearing on Peek
/// would let one arrow-key run down a column erase the whole batch.
#[test]
fn ac_p3_30_8_peek_and_list_never_stamp() {
    let rig = Rig::new();
    rig.project(1, "alpha");
    rig.project(2, "beta");

    let ctx = rig.ctx();
    let projects = codotheca_core::projects::ProjectsCtx {
        index: &rig.index,
        events: &rig.sink,
        jobs: &rig.jobs,
        mounts: &rig.mount,
        sync: &codotheca_core::sync::runner::NullSyncSink,
        now: NOW,
        tz_offset_min: 0,
    };
    codotheca_core::projects::list::handle(&projects, json!({})).expect("list");
    codotheca_core::projects::peek::handle(&projects, json!({ "id": 2 })).expect("peek");

    assert_eq!(acknowledged_at(&rig, 1), None, "projects.list never stamps");
    assert_eq!(acknowledged_at(&rig, 2), None, "projects.peek never stamps");

    // And the command that does, on the same rig, so the two are compared rather than asserted
    // separately: a writer that never fires would pass the two nulls above on its own.
    handle_project_get(&ctx, json!({ "id": 2 })).expect("get");
    assert_eq!(acknowledged_at(&rig, 2), Some(NOW));
}

/// **The ordering assertion, and the only one that catches the open-the-page-twice bug.**
///
/// The stamp commits **before** the reading is computed, in the same transaction. Reversed, the
/// first open serves a reading computed while the project was still unenrolled, the user sees
/// `suppressed`, and has to open the page twice to see a reading that was available the first
/// time.
#[test]
fn ac_p3_30_9_the_first_get_on_an_unenrolled_project_is_not_suppressed() {
    let rig = Rig::new();
    rig.project(1, "alpha");

    let enrolled = {
        let tx = rig.conn().unchecked_transaction().unwrap();
        let enrolled = stamp_and_read_enrolment(&tx, ProjectId(1), NOW).unwrap();
        tx.commit().unwrap();
        enrolled
    };
    assert!(
        enrolled,
        "the enrolment the reading is computed from is read AFTER the stamp"
    );

    // The predicate itself, over both inputs it can have.
    assert!(!is_enrolled(None));
    assert!(is_enrolled(Some(NOW)));

    // And through the real handler, on a project that passes every gate above suppression —
    // authored, not Reference, one present copy — so enrolment is the only thing between it and
    // a reading.
    let rig2 = Rig::new();
    rig2.project(1, "alpha");
    rig2.location(1, 1, "/a", "present", Some("main"), Some(0), Some(NOW));
    rig2.conn()
        .execute("UPDATE project SET authored_by_user = 1 WHERE id = 1", [])
        .unwrap();

    // The control: read before any stamp, this project is `suppressed`. Without it, a fixture
    // stopped at an earlier gate would read `absent` in either order and prove nothing.
    let (before, _) = read_for_project(rig2.conn(), ProjectId(1)).unwrap();
    assert_eq!(acknowledged_at(&rig2, 1), None, "the control stamped");
    assert_eq!(
        before.state,
        HealthState::Suppressed,
        "the fixture stops at a gate before suppression, so it cannot tell the two orders apart"
    );

    let detail = handle_project_get(&rig2.ctx(), json!({ "id": 1 })).expect("first get");
    eprintln!(
        "AC-P3-30-9 state before any stamp: {:?}; returned by the first get: {:?}",
        before.state, detail.health.state
    );
    assert_ne!(
        detail.health.state,
        HealthState::Suppressed,
        "the first projects.get served the reading computed before its own stamp"
    );
    assert_eq!(detail.health.state, HealthState::Live);
    assert!(
        is_enrolled(acknowledged_at(&rig2, 1)),
        "one projects.get leaves the project enrolled"
    );
}

/// §30.5, and `.dev/spec/01-data-model.md`'s standing rule: `acknowledged_at` is **never derived
/// from `last_interaction_at`**, which counts reflog activity performed *outside* the app — so
/// deriving it would let activity outside the app forge enrolment.
///
/// A source audit, because the rule is about what the code may not do rather than about a value.
#[test]
fn ac_p3_30_10_no_acknowledged_at_writer_reads_last_interaction_at() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    assert!(
        !files.is_empty(),
        "scanned no files: a source audit over nothing is a failing audit"
    );

    let mut write_sites = 0usize;
    for path in &files {
        let text = std::fs::read_to_string(path).expect("read source");
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            // A write site is an assignment in SQL text, never a projection or a SELECT list.
            if !(line.contains("acknowledged_at =") || line.contains("acknowledged_at=")) {
                continue;
            }
            write_sites += 1;
            let lo = i.saturating_sub(6);
            let hi = (i + 7).min(lines.len());
            let window = lines[lo..hi].join("\n");
            assert!(
                !window.contains("last_interaction_at"),
                "{}:{} writes acknowledged_at within reach of last_interaction_at",
                path.display(),
                i + 1
            );
        }
    }

    // stderr, never stdout: stdout carries protocol frames and nothing else, and
    // `print_stdout = "deny"` in `core/Cargo.toml` holds that for tests too.
    eprintln!(
        "acknowledged_at write sites scanned: {write_sites} over {} files",
        files.len()
    );
    assert!(
        write_sites > 0,
        "no acknowledged_at write site exists: §27.7's defect is still open"
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
