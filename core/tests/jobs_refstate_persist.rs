#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! **[p2-24b] R51, at the column.** An unreadable stash reflog must reach the database as NULL.
//!
//! The struct saying `None` is not the same claim as the column holding NULL: `persist` is where
//! the two meet, and a `map` that collapsed to 0 there would be invisible to every test above it.

use codotheca_core::git::{RefFingerprint, RefState};
use codotheca_core::jobs::j1_refstate::persist;
use codotheca_core::protocol::LocationId;
use codotheca_core::testing::TempIndex;

const DIGEST: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

fn state(stash_count: Option<u32>) -> RefState {
    RefState {
        head_oid: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
        branch: Some("main".to_owned()),
        upstream: None,
        ahead: None,
        behind: None,
        tag_count: 0,
        stash_count,
        is_shallow: false,
        is_bare: false,
        interrupted_op: None,
        fetch_head_at: None,
        reflog_tail_at: None,
        basis: RefFingerprint::from_hex(DIGEST).expect("parses"),
        observed_at: 1,
    }
}

/// Against a **real migrated database**, not a mock: the column's own nullability is what is
/// being asserted.
fn persisted_stash_count(stash: Option<u32>) -> Option<i64> {
    let index = TempIndex::new();
    let project = index.insert_project();
    let location = index.insert_location(project, "/r/widget");

    let _guard = codotheca_core::proto::txguard::TxGuard::enter();
    let binding = index.index();
    let conn = binding.conn();
    let tx = conn.unchecked_transaction().expect("tx");
    persist(&tx, LocationId(location.0), &state(stash)).expect("persisted");
    tx.commit().expect("commit");

    conn.query_row(
        "SELECT stash_count FROM location WHERE id = ?1",
        [location.0],
        |row| row.get::<_, Option<i64>>(0),
    )
    .expect("read back")
}

#[test]
fn an_unreadable_stash_reflog_reaches_the_column_as_null_and_not_zero() {
    assert_eq!(
        persisted_stash_count(None),
        None,
        "NULL is *unreadable*; 0 would be a false all-clear to a deletion gate"
    );
}

#[test]
fn a_real_zero_reaches_the_column_as_zero() {
    assert_eq!(
        persisted_stash_count(Some(0)),
        Some(0),
        "*no stash* is a real observation and must not be flattened into NULL"
    );
}

#[test]
fn a_count_reaches_the_column_intact() {
    assert_eq!(persisted_stash_count(Some(3)), Some(3));
}
