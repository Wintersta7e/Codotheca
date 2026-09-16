//! `projects.get` (§8.5). Compiled only under `testkit`: the rig links
//! `codotheca_core::testing`, which the feature gates.
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
use codotheca_core::protocol::{ErrorCode, LaneState, ReadmeStateKind};
use detail_rig::{Rig, NOW};
use serde_json::{json, Value};

/// Every key in the payload whose value is a JSON `0`, anywhere in the tree.
fn zero_keys(v: &Value, path: &str, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                zero_keys(child, &format!("{path}.{k}"), out);
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                zero_keys(child, &format!("{path}[{i}]"), out);
            }
        }
        Value::Number(n) if n.as_i64() == Some(0) => out.push(path.to_owned()),
        _ => {}
    }
}

fn rig_with_nothing_computed() -> Rig {
    let rig = Rig::new();
    rig.project(1, "alpha");
    rig
}

#[test]
fn an_uncomputed_field_is_null_on_the_wire_and_never_zero() {
    let rig = rig_with_nothing_computed();
    let detail = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    let v = serde_json::to_value(&detail).expect("encode");

    // §7.7a, and the phase-1 case that is the *only* case: completion is uncomputed.
    assert_eq!(v["row"]["completionLit"], Value::Null);
    assert_eq!(v["row"]["completionApplicable"], Value::Null);
    // §6: nothing has been observed, so nothing carries an observation time or a figure.
    for key in [
        "lastCommitAt",
        "firstCommitAt",
        "sizeTrackedBytes",
        "trackedFiles",
        "isDirty",
        "untrackedCount",
        "ahead",
        "behind",
        "stashCount",
        "fetchHeadAt",
        "refstateObservedAt",
        "worktreeObservedAt",
    ] {
        assert_eq!(
            v["row"][key],
            Value::Null,
            "row.{key} claims a value nothing computed"
        );
    }

    let mut zeros = Vec::new();
    zero_keys(&v, "", &mut zeros);
    // Genuine measured zeros: no session has been launched, the art walk is at its origin, and
    // this install's session ledger is complete, so a week with no launch really is 0.
    // **Extend this list only for a field the fixture genuinely measures as zero** — extending
    // it to quiet a failure is how unknown-as-zero would enter the wire unnoticed.
    zeros.retain(|k| {
        // `rerollOffset` appears twice — `ProjectDetail` carries it and so does its `row`. Both
        // are the same stored column read once, and `0` there is the art walk at its origin,
        // which is a measurement (§7.3a).
        !k.ends_with("rerollOffset")
            && k != ".playtimeSeconds"
            && !k.ends_with("sessionCount")
            && !k.ends_with("sessionSeconds")
    });
    assert!(zeros.is_empty(), "unknown rendered as zero at: {zeros:?}");
}

#[test]
fn a_null_fetch_head_yields_no_age_and_no_zero() {
    // Criterion 63: `location.fetch_head_at` is the sole source of the BEHIND chip's age. A
    // fixture with `ahead > 0` and no FETCH_HEAD must carry no substitute.
    let rig = rig_with_nothing_computed();
    rig.location(
        1,
        1,
        "/srv/work/thing",
        "present",
        Some("main"),
        Some(3),
        None,
    );
    let detail = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    let v = serde_json::to_value(&detail).expect("encode");
    assert_eq!(v["locations"][0]["ahead"], json!(3));
    assert_eq!(v["locations"][0]["fetchHeadAt"], Value::Null);
    // Not the ref-state observation, which is a different clock and would date the wrong thing.
    assert_ne!(
        v["locations"][0]["fetchHeadAt"],
        v["locations"][0]["refstateObservedAt"]
    );
}

#[test]
fn the_readme_carries_the_time_it_was_read_or_no_time_at_all() {
    // R33 gap 2: readAt comes from peek_cache.computed_at and dates the observation.
    let rig = rig_with_nothing_computed();
    let none = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    assert_eq!(none.readme.state, ReadmeStateKind::NotIndexed);
    assert_eq!(
        none.readme.read_at, None,
        "nothing looked, so nothing was observed"
    );

    rig.peek_cache(1, None, 1_700_000_500);
    let looked = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    assert_eq!(looked.readme.state, ReadmeStateKind::Absent);
    assert_eq!(
        looked.readme.read_at,
        Some(1_700_000_500),
        "we looked and found none"
    );
}

#[test]
fn an_offline_location_carries_last_seen_and_names_no_drive() {
    // R33 gap 3 / criterion 63: the facts are exactly `last seen <age>` and the branch as last
    // observed. `volume_key` is stable, not readable, and unreadable while unmounted anyway.
    let rig = rig_with_nothing_computed();
    rig.location(
        1,
        1,
        "/srv/work/thing",
        "offline",
        Some("main"),
        None,
        Some(1_699_000_000),
    );
    let detail = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    let v = serde_json::to_value(&detail).expect("encode");
    let row = &v["locations"][0];
    assert_eq!(row["presence"], json!("offline"));
    assert_eq!(row["lastSeenAt"], json!(1_699_000_000_i64));
    assert_eq!(row["branch"], json!("main"));
    let text = serde_json::to_string(&v).expect("string");
    for banned in ["volumeKey", "volume_key", "storeKey", "store_key", "VOL-A"] {
        assert!(
            !text.contains(banned),
            "the payload names a drive: {banned}"
        );
    }
}

#[test]
fn one_copy_compares_no_heads_and_two_that_agree_do() {
    // §8.5.2: a copy nobody has read is not evidence of agreement. `not_compared` is the only
    // honest answer with fewer than two observed HEADs.
    let rig = rig_with_nothing_computed();
    rig.location(1, 1, "/srv/work/thing", "present", Some("main"), None, None);
    let one = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    let v = serde_json::to_value(&one).expect("encode");
    assert_eq!(v["locations"][0]["headComparison"], json!("not_compared"));

    rig.location(
        2,
        1,
        "/srv/other/thing",
        "present",
        Some("main"),
        None,
        None,
    );
    rig.conn()
        .execute("UPDATE location SET head_oid = 'abc'", [])
        .expect("observe both heads");
    let two = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    let v = serde_json::to_value(&two).expect("encode");
    assert_eq!(v["locations"][0]["headComparison"], json!("same_commit"));

    rig.conn()
        .execute("UPDATE location SET head_oid = 'def' WHERE id = 2", [])
        .expect("diverge");
    let apart = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    let v = serde_json::to_value(&apart).expect("encode");
    assert_eq!(
        v["locations"][0]["headComparison"],
        json!("different_commit")
    );
}

#[test]
fn commit_days_are_absent_not_zero_until_j4_has_run() {
    // §8.5.5's easiest surface to lie on: 26 zero-height bars is the canonical picture of
    // not-computed, and it looks like a complete, honest record of doing nothing.
    let rig = rig_with_nothing_computed();
    let detail = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    assert_eq!(detail.activity.commit_days, LaneState::NotComputed);
    assert_eq!(detail.activity.weeks.len(), 26);
    assert!(detail
        .activity
        .weeks
        .iter()
        .all(|w| w.commit_days.is_none()));
    // The two ledgers arrive apart, with their own lane states, and nothing sums them.
    assert_eq!(detail.activity.sessions, LaneState::Measured);
    let v = serde_json::to_value(&detail.activity).expect("encode");
    let keys: Vec<&str> = v["weeks"][0]
        .as_object()
        .expect("week")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["commitDays", "sessionCount", "sessionSeconds", "weekStart"]
    );
}

#[test]
fn a_measured_lane_reports_commit_days_and_never_a_commit_count() {
    let rig = rig_with_nothing_computed();
    rig.conn()
        .execute(
            "INSERT INTO project_job_state (project_id, job, state, at) VALUES (1, 'j4', 'ok', ?1)",
            [NOW],
        )
        .expect("j4 ran");
    // Two commit-*days* in the newest slot. `xp_events` stores one row per day and nothing
    // anywhere stores a commit count (§8.5.5).
    for (i, day) in [NOW - 86_400, NOW - 2 * 86_400].iter().enumerate() {
        rig.conn()
            .execute(
                "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track)
                 VALUES (?1, 1, 'p1', 'commit_day', ?2, 'git')",
                rusqlite::params![day, format!("commit_day:p1:{i}")],
            )
            .expect("seed day");
    }
    let detail = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    assert_eq!(detail.activity.commit_days, LaneState::Measured);
    let newest = detail.activity.weeks.last().expect("26 weeks");
    assert_eq!(newest.commit_days, Some(2));
    let total: u32 = detail
        .activity
        .weeks
        .iter()
        .filter_map(|w| w.commit_days)
        .sum();
    assert_eq!(total, 2, "days, and at most seven of them a week");
}

#[test]
fn a_shallow_clone_is_excluded_rather_than_reported_as_empty() {
    let rig = rig_with_nothing_computed();
    rig.conn()
        .execute("UPDATE project SET is_shallow = 1 WHERE id = 1", [])
        .expect("shallow");
    let detail = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    assert_eq!(detail.activity.commit_days, LaneState::ShallowExcluded);
    assert!(detail
        .activity
        .weeks
        .iter()
        .all(|w| w.commit_days.is_none()));
}

#[test]
fn a_merged_project_id_follows_its_redirect_rather_than_failing() {
    // The page can be opened from a stale link while a scan merges two tiles underneath it.
    let rig = rig_with_nothing_computed();
    rig.project(9, "survivor");
    rig.conn()
        .execute("UPDATE project SET merged_into = 9 WHERE id = 1", [])
        .expect("tombstone");
    rig.conn()
        .execute(
            "INSERT INTO project_redirect (old_project_id, new_project_id, merged_at)
             VALUES (1, 9, ?1)",
            [NOW],
        )
        .expect("redirect");
    let detail = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    assert_eq!(detail.row.id.0, 9);
}

#[test]
fn an_unknown_project_is_a_refusal_and_not_an_empty_page() {
    // §2.4's enum is closed and has no NOT_FOUND. `PROTOCOL` is the honest one: the app asked
    // for something that does not exist. `INTERNAL` would claim a core defect that is not there.
    let rig = rig_with_nothing_computed();
    let err = handle_project_get(&rig.ctx(), json!({ "id": 4242 })).expect_err("refusal");
    assert_eq!(err.code, ErrorCode::Protocol);
}

#[test]
fn the_page_runs_no_git_at_all() {
    // §8.5: `projects.get` returns stored observations with their stored timestamps. The fake
    // backend answers nothing, so a handler that reached for git would fail rather than pass.
    let rig = rig_with_nothing_computed();
    rig.location(1, 1, "/srv/work/thing", "present", Some("main"), None, None);
    handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    assert!(
        rig.git.calls().is_empty(),
        "projects.get invoked git: {:?}",
        rig.git.calls()
    );
}

/// §6: an opened page asks for a current worktree reading for the copy it is showing, and asks
/// **once**.
///
/// `on_visible` had no caller anywhere in the product, which is what made "no changes as of T"
/// a stale answer rather than a current one.
#[test]
fn opening_a_page_asks_for_one_fresh_reading_of_the_copy_it_shows() {
    let rig = rig_with_nothing_computed();
    rig.location(1, 1, "/srv/work/thing", "present", Some("main"), None, None);

    let detail = handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get");
    assert_eq!(
        rig.jobs.visible.lock().expect("lock").clone(),
        vec![(1, 1)],
        "the page asks for the primary copy, once"
    );

    // The answer is the stored reading, not a claim about now: this project has had no job run
    // against it, so its worktree is *not observed* and must say so rather than reading clean.
    let v = serde_json::to_value(&detail).expect("encode");
    assert_eq!(v["row"]["isDirty"], json!(null));
    assert_eq!(v["row"]["worktreeObservedAt"], json!(null));
}

/// `projects.get` fills `remote` and `backup`, and **nothing else in this file asserted that**.
///
/// Both fields reach the renderer only through this handler, and every renderer test mounts a
/// fixture, so a handler that answered `None` for either would have passed the whole suite and
/// rendered an empty `REMOTE` tab. That is R90's shape one layer up: the producer exists, and
/// nothing on a production path checked that it produces. A jsdom test cannot see it and the
/// e2e spec needs a built app; this needs neither.
#[test]
fn the_page_carries_the_remote_facts_and_the_backup_state_it_is_the_only_source_of() {
    let rig = rig_with_nothing_computed();
    rig.location(1, 1, "/srv/work/thing", "present", Some("main"), None, None);
    rig.conn()
        .execute(
            "UPDATE project SET remote_key = 'github.com/acme/widget' WHERE id = 1",
            [],
        )
        .expect("give the project a remote");

    let v = serde_json::to_value(handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get"))
        .expect("encode");

    // Non-null because the row has a `remote_key`: the predicate is the key's presence, not
    // whether a forge has ever answered. An unobserved read is a *state* inside the payload.
    assert!(
        !v["remote"].is_null(),
        "a project with a remote_key must carry remote facts: {v}"
    );
    assert_eq!(v["remote"]["key"], json!("github.com/acme/widget"));
    // `no_account`, not `not_observed`: this rig has connected nothing, and §25.1 distinguishes
    // *there is no token to ask with* from *there is one and it has not asked yet*. Asserted
    // against what the handler answers rather than against what this test first assumed — the
    // two states render differently, and `no_account` draws no forge block at all rather than a
    // row of dashes advertising a feature nobody can use.
    assert_eq!(
        v["remote"]["state"],
        json!("no_account"),
        "with nothing connected the state is a fact about this machine, not about the forge"
    );
    // **Null, and that is the honest answer.** `backup_state` has a remote to compare against
    // but no `ahead`, no stash count and no recorded fetch, so whether a copy exists elsewhere
    // is *uncomputed* — and §25.3 draws no block rather than claiming one. Asserted explicitly
    // because the tempting reading of a null here is "the producer is not wired", and the test
    // below distinguishes the two by making the same producer answer.
    assert_eq!(
        v["backup"],
        json!(null),
        "an unobserved copy has no backup verdict, and silence is not a claim"
    );
}

/// A project with no `remote_key` carries **no** remote facts — the same predicate, the other way.
///
/// Asserted because a producer that returned a default `RemoteFacts` for every project would
/// satisfy the test above and would make §23's not-cloned rendering claim a remote that no
/// repository has.
#[test]
fn a_project_with_no_remote_key_carries_no_remote_facts() {
    let rig = rig_with_nothing_computed();
    rig.location(1, 1, "/srv/work/thing", "present", Some("main"), None, None);

    let v = serde_json::to_value(handle_project_get(&rig.ctx(), json!({ "id": 1 })).expect("get"))
        .expect("encode");
    assert_eq!(v["remote"], json!(null));
    // And the backup producer **is** wired on this path: a project with no remote is the only
    // copy whatever its working tree says, which is `backup_state`'s first row and the one that
    // cannot be reached by falling through. This is what separates "the producer answered null"
    // from "nothing called the producer" — R90's question asked of a value rather than a name.
    assert_eq!(
        v["backup"],
        json!("only_copy"),
        "a project with no remote is the only copy, and the handler must say so"
    );
}
