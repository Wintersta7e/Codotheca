//! §2.4's two identity commands (§1.5, §1.6, §11.1).

use serde_json::Value;

use crate::index::Index;
use crate::jobs::state::{reset_for, ResetCause};
use crate::proto::dispatch::{parse_args, CommandFailure}; // R15: one helper, plan 03's
use crate::proto::txguard::TxGuard;
use crate::proto::EventSink; // R16: the trait, not plan 07's WalkSink
use crate::protocol::{Flag, MergeResult, ProjectId, ProjectMerged};

use super::merge::{merge_projects, unmerge_hint, UnmergeHint};
use super::{AssociationKind, IdentityError};

/// Everything §2.4's two identity commands need.
///
/// `index` is shared, not `&mut`: `Connection::unchecked_transaction` produces a real
/// `Transaction` from `&Connection`, and plan 03's `TxGuard` supplies at runtime the
/// no-nesting guarantee `&mut` would have supplied statically. `now` is unix **seconds**,
/// passed in so nothing here reads the clock.
pub struct IdentityCtx<'a> {
    pub index: &'a Index,
    pub events: &'a dyn EventSink,
    pub now: i64,
}

impl std::fmt::Debug for IdentityCtx<'_> {
    /// Hand-written because `&dyn EventSink` is not `Debug` and the index holds a live
    /// connection; neither is printable and neither is what a reader wants here.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityCtx")
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeArgs {
    /// Neither argument picks the survivor — §1.5's rule does (`choose_survivor`).
    a: ProjectId,
    b: ProjectId,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct HintArgs {
    id: ProjectId,
}

/// `None` means "not mine". Plan 21's router chains the next dispatcher on it.
#[must_use]
pub fn dispatch_identity_command(
    ctx: &IdentityCtx<'_>,
    command: &str,
    args: Value,
) -> Option<Result<Value, CommandFailure>> {
    match command {
        "projects.merge" => Some(handle_merge(ctx, args)),
        "projects.unmergeHint" => Some(handle_unmerge_hint(ctx, args)),
        _ => None,
    }
}

/// `IdentityError` derives `Debug` and not `Display`, and the message is diagnostic only —
/// §2.4 gives every user-facing string to the shell.
fn failure(err: &IdentityError) -> CommandFailure {
    CommandFailure {
        code: err.code(),
        message: format!("{err:?}"),
        outcome: None,
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Result<Value, CommandFailure> {
    serde_json::to_value(value).map_err(|e| CommandFailure::internal(e.to_string()))
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn count_i64(n: i64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// §1.5, as one transaction. **Zero call sites outside the test suite** (§11.1, criterion 63):
/// no phase-1 surface issues this, and `20b`'s gate fails if one starts to.
fn handle_merge(ctx: &IdentityCtx<'_>, args: Value) -> Result<Value, CommandFailure> {
    let args: MergeArgs = parse_args(args)?;
    let evidence = serde_json::json!({"rule": "manual", "command": "projects.merge"});

    let outcome = {
        let _guard = TxGuard::enter();
        let tx = ctx
            .index
            .conn()
            .unchecked_transaction()
            .map_err(|e| CommandFailure::internal(e.to_string()))?;
        let outcome = merge_projects(
            &tx,
            args.a.0,
            args.b.0,
            AssociationKind::Manual,
            &evidence,
            ctx.now,
        )
        .map_err(|e| failure(&e))?;

        // Task 14's `needs_history_recompute`, answered inside the same transaction: Task 12
        // deleted the survivor's git-derived rows, so a commit that did not also mark the jobs
        // due would leave nothing scheduled to put them back.
        if outcome.needs_history_recompute {
            reset_for(
                &tx,
                ProjectId(outcome.survivor),
                ResetCause::UserRequested,
                ctx.now,
            )
            .map_err(|e| CommandFailure::internal(e.to_string()))?;
        }
        tx.commit()
            .map_err(|e| CommandFailure::internal(e.to_string()))?;
        outcome
    };

    // After the commit, never before: §2.4's `projects/merged` is what collapses two visible
    // tiles into one, and an event for a merge that rolled back cannot be taken back.
    ctx.events.emit(
        "projects",
        "merged",
        encode(&ProjectMerged {
            from: ProjectId(outcome.absorbed),
            into: ProjectId(outcome.survivor),
        })?,
    );

    encode(&MergeResult {
        survivor: ProjectId(outcome.survivor),
        absorbed: ProjectId(outcome.absorbed),
        locations_reparented: count(outcome.reparented.location),
        sessions_reparented: count(outcome.reparented.session),
        collection_members_merged: count(outcome.reparented.collection_member),
        xp_events_recomputed: count(outcome.derived.xp_events_reparented),
    })
}

/// §1.5: performs no split, writes nothing, takes no confirmation. The transaction is dropped
/// rather than committed, so a future edit that writes by accident cannot persist it.
fn handle_unmerge_hint(ctx: &IdentityCtx<'_>, args: Value) -> Result<Value, CommandFailure> {
    let args: HintArgs = parse_args(args)?;
    let _guard = TxGuard::enter();
    let tx = ctx
        .index
        .conn()
        .unchecked_transaction()
        .map_err(|e| CommandFailure::internal(e.to_string()))?;
    let hints = unmerge_hint(&tx, args.id.0).map_err(|e| failure(&e))?;
    drop(tx); // rolls back — read-only is enforced, not merely intended

    let wire: Vec<crate::protocol::UnmergeHint> = hints.iter().map(to_wire).collect();
    encode(&wire)
}

/// The generated `Flag` has three variants, and `is_reference` is not one of them: it is
/// computed, not user-set, so it has no wire representation and is dropped here. Deliberate —
/// do not add a variant to `Flag` to close it.
fn flags(names: &[String]) -> Vec<Flag> {
    names
        .iter()
        .filter_map(|n| match n.as_str() {
            "is_pinned" => Some(Flag::Pinned),
            "is_archived" => Some(Flag::Archived),
            "is_hidden" => Some(Flag::Hidden),
            _ => None,
        })
        .collect()
}

fn wire_kind(kind: AssociationKind) -> AssociationKind {
    kind
}

/// `merge_record_id` and `evidence_json` stay core-side: §8.5.2 renders the counts and the
/// association, and the renderer has no use for a row id it can never quote back.
fn to_wire(hint: &UnmergeHint) -> crate::protocol::UnmergeHint {
    crate::protocol::UnmergeHint {
        absorbed_project_id: ProjectId(hint.absorbed_project_id),
        association_kind: wire_kind(hint.association_kind),
        merged_at: hint.merged_at,
        locations: count_i64(hint.locations),
        sessions: count_i64(hint.sessions),
        collection_memberships: count_i64(hint.collection_members),
        xp_events: count_i64(hint.xp_events),
        notes_concatenated: hint.notes_were_concatenated,
        flags_orred: flags(&hint.flags_lost_by_or),
        flags_anded: flags(&hint.flags_lost_by_and),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]
    use super::super::testutil::{insert_location, insert_project, NewProject};
    use super::{dispatch_identity_command, IdentityCtx};
    use serde_json::json;

    const NOW: i64 = 1_760_000_000;

    /// Counts every emission so a test can prove one was published — and that the read-only
    /// command published none.
    #[derive(Default)]
    struct RecordingSink(std::sync::Mutex<Vec<(String, String, serde_json::Value)>>);

    impl crate::proto::EventSink for RecordingSink {
        fn emit(&self, topic: &str, event: &str, payload: serde_json::Value) {
            if let Ok(mut v) = self.0.lock() {
                v.push((topic.to_owned(), event.to_owned(), payload));
            }
        }
    }

    fn open() -> (tempfile::TempDir, crate::index::Index) {
        let dir = tempfile::tempdir().unwrap();
        let index = crate::index::Index::open_at(dir.path(), NOW).unwrap();
        (dir, index)
    }

    fn p(index: &crate::index::Index, name: &'static str, created_at: i64) -> i64 {
        insert_project(
            index.conn(),
            NewProject {
                name,
                lineage_key: None,
                remote_key: None,
                created_at,
            },
        )
    }

    #[test]
    fn a_command_this_module_does_not_own_is_declined_so_the_router_can_chain() {
        let (_dir, index) = open();
        let sink = RecordingSink::default();
        let ctx = IdentityCtx {
            index: &index,
            events: &sink,
            now: NOW,
        };
        assert!(dispatch_identity_command(&ctx, "projects.list", json!({})).is_none());
        assert!(dispatch_identity_command(&ctx, "roots.list", json!({})).is_none());
    }

    #[test]
    fn a_malformed_argument_object_is_a_protocol_failure_and_not_a_panic() {
        let (_dir, index) = open();
        let sink = RecordingSink::default();
        let ctx = IdentityCtx {
            index: &index,
            events: &sink,
            now: NOW,
        };
        let failure = dispatch_identity_command(&ctx, "projects.merge", json!({"a": 1}))
            .unwrap()
            .unwrap_err();
        assert_eq!(failure.code, crate::protocol::ErrorCode::Protocol);
    }

    // §1.5: one transaction, and the survivor is the core's choice — not argument `b`.
    #[test]
    fn merging_commits_once_reports_the_chosen_survivor_and_announces_it() {
        let (_dir, index) = open();
        let survivor = p(&index, "s", 100);
        let absorbed = p(&index, "a", 200);
        insert_location(index.conn(), absorbed, "/w/a", None);
        let sink = RecordingSink::default();
        let ctx = IdentityCtx {
            index: &index,
            events: &sink,
            now: NOW,
        };

        // Argument order deliberately puts the younger project in `b`.
        let out = dispatch_identity_command(
            &ctx,
            "projects.merge",
            json!({"a": survivor, "b": absorbed}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            out["survivor"],
            json!(survivor),
            "earliest created_at wins, not argument `b`"
        );
        assert_eq!(out["absorbed"], json!(absorbed));
        assert_eq!(out["locationsReparented"], json!(1));

        // Committed: the tombstone and the redirect are both visible outside the transaction.
        let merged_into: Option<i64> = index
            .conn()
            .query_row(
                "SELECT merged_into FROM project WHERE id = ?1",
                rusqlite::params![absorbed],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(merged_into, Some(survivor));

        // §2.4: the renderer collapses two visible tiles on this event, so it must be published
        // — and only after the commit that makes it true.
        let events = sink.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            (events[0].0.as_str(), events[0].1.as_str()),
            ("projects", "merged")
        );
        assert_eq!(events[0].2, json!({"from": absorbed, "into": survivor}));
    }

    // §1.6: one hop, never a chain — a caller holding an id from an earlier merge still lands.
    #[test]
    fn a_stale_id_resolves_through_the_redirect_before_anything_is_written() {
        let (_dir, index) = open();
        let a = p(&index, "a", 100);
        let b = p(&index, "b", 200);
        let c = p(&index, "c", 300);
        let sink = RecordingSink::default();
        let ctx = IdentityCtx {
            index: &index,
            events: &sink,
            now: NOW,
        };

        dispatch_identity_command(&ctx, "projects.merge", json!({"a": a, "b": b}))
            .unwrap()
            .unwrap();
        // `b` is now stale. Merging it again must land on its survivor, not error.
        let out = dispatch_identity_command(&ctx, "projects.merge", json!({"a": b, "b": c}))
            .unwrap()
            .unwrap();
        assert_eq!(out["survivor"], json!(a));
        assert_eq!(out["absorbed"], json!(c));
    }

    // §1.5, §8.5.2: it reports what a split would have to decide, and decides nothing.
    #[test]
    fn the_hint_is_read_only_writes_nothing_and_publishes_nothing() {
        let (_dir, index) = open();
        let survivor = p(&index, "s", 100);
        let absorbed = p(&index, "a", 200);
        insert_location(index.conn(), absorbed, "/w/a", None);
        let sink = RecordingSink::default();
        let ctx = IdentityCtx {
            index: &index,
            events: &sink,
            now: NOW,
        };
        dispatch_identity_command(
            &ctx,
            "projects.merge",
            json!({"a": survivor, "b": absorbed}),
        )
        .unwrap()
        .unwrap();

        let before: i64 = index
            .conn()
            .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
            .unwrap();
        let emitted_before = sink.0.lock().unwrap().len();

        let out = dispatch_identity_command(&ctx, "projects.unmergeHint", json!({"id": survivor}))
            .unwrap()
            .unwrap();
        let rows = out.as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["absorbedProjectId"], json!(absorbed));
        assert_eq!(rows[0]["locations"], json!(1));

        let after: i64 = index
            .conn()
            .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after, "the hint reports; it never reverses");
        assert_eq!(
            sink.0.lock().unwrap().len(),
            emitted_before,
            "and it announces nothing"
        );
    }

    // A project that absorbed nothing has an empty list — which is a known-empty answer, not an
    // error and not a null.
    /// §1.5 says `unmergeHint` "performs no split, writes nothing, and takes no confirmation",
    /// and the handler honours that by dropping its transaction rather than committing. **No
    /// behavioural test can catch the removal of that `drop`**: `unmerge_hint` writes nothing
    /// today, so committing an empty transaction is a no-op and every assertion above still
    /// passes. The rollback exists to contain a *future* edit that writes by accident, so what
    /// guards it has to be the source itself — the same shape as the read-only git audit.
    #[test]
    fn the_hints_transaction_is_never_committed() {
        let source = include_str!("commands.rs");
        let handler = source
            .split_once("fn handle_unmerge_hint")
            .expect("the handler must exist")
            .1;
        let body = handler.split_once("\nfn ").map_or(handler, |(b, _)| b);
        assert!(
            !body.contains(".commit()"),
            "handle_unmerge_hint commits its transaction; §1.5 makes it read-only, and the \
             rollback is what stops a later write persisting"
        );
        assert!(
            body.contains("drop(tx)"),
            "the transaction must be dropped explicitly, so the intent is legible at the site"
        );
    }

    #[test]
    fn a_project_that_absorbed_nothing_answers_an_empty_list() {
        let (_dir, index) = open();
        let s = p(&index, "s", 100);
        let sink = RecordingSink::default();
        let ctx = IdentityCtx {
            index: &index,
            events: &sink,
            now: NOW,
        };
        let out = dispatch_identity_command(&ctx, "projects.unmergeHint", json!({"id": s}))
            .unwrap()
            .unwrap();
        assert_eq!(out, json!([]));
    }
}
