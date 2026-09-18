#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §24.8's mutating call, and the verdict it recomputes for itself.

use std::path::{Path, PathBuf};

use codotheca_core::git::RootCommit;
use codotheca_core::protocol::{LocationId, UninstallBlocker, UninstallDisposition};
use codotheca_core::removal::{RemovalOutcome, Warrant};
use codotheca_core::uninstall::command::{cleared_columns, uninstall_location};
use codotheca_core::uninstall::gates::RemoteOutcome;
use codotheca_core::uninstall::verdict::VerdictSeal;
use codotheca_core::uninstall::{compute_verdict, LocationSnapshot, VerdictInputs};

fn identity() -> RootCommit {
    RootCommit {
        oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        committed_at: 0,
        tz_offset_min: 0,
    }
}

/// A copy that clears every gate: observed, not shallow, under a root, fully pushed, clean,
/// nothing stashed, no live session, remote reached.
fn clean_inputs(root: &Path, copy: &Path) -> VerdictInputs {
    VerdictInputs {
        snapshot: LocationSnapshot {
            id: LocationId(1),
            path: copy.to_path_buf(),
            refstate_observed_at: Some(10),
            worktree_observed_at: Some(11),
            is_shallow: false,
            removed_at: None,
        },
        roots: vec![root.to_path_buf()],
        remote: RemoteOutcome::Reached,
        unique: Vec::new(),
        live_session: false,
        now: 1_700_000_000,
    }
}

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().join("library");
    let copy = root.join("widget");
    std::fs::create_dir_all(copy.join(".git")).expect("mkdir");
    std::fs::write(copy.join("a.txt"), b"one").expect("write");
    (dir, root, copy)
}

#[test]
fn a_copy_that_clears_every_gate_is_safe_and_names_no_blocker() {
    let (_dir, root, copy) = fixture();
    let (verdict, _seal) = compute_verdict(&clean_inputs(&root, &copy)).expect("computed");
    assert_eq!(verdict.disposition, UninstallDisposition::Safe);
    assert!(verdict.blockers.is_empty(), "{:?}", verdict.blockers);
    assert_eq!(
        verdict.remote_verified_at,
        Some(1_700_000_000),
        "VERIFIED <age>, never PUSHED — so the instant is real"
    );
    assert_eq!(verdict.computed_at, 1_700_000_000);
}

/// Every blocker found is reported, of both classes — the fold decides a disposition, it never
/// edits the list. A surface that learned only *blocked* could not say why.
#[test]
fn every_blocker_found_is_reported_and_none_is_hidden() {
    let (_dir, root, copy) = fixture();
    let mut inputs = clean_inputs(&root, &copy);
    inputs.is_shallow_and_dirty();
    let (verdict, _seal) = compute_verdict(&inputs).expect("computed");
    assert_eq!(verdict.disposition, UninstallDisposition::Blocked);
    assert!(verdict.blockers.contains(&UninstallBlocker::ShallowClone));
    assert!(verdict
        .blockers
        .contains(&UninstallBlocker::UncommittedChanges));
}

trait Seeded {
    fn is_shallow_and_dirty(&mut self);
}

impl Seeded for VerdictInputs {
    fn is_shallow_and_dirty(&mut self) {
        self.snapshot.is_shallow = true;
        self.unique.push(UninstallBlocker::UncommittedChanges);
    }
}

/// **AC-P2-24-13, and the assertion the whole boundary rests on.**
///
/// The pre-flight said `safe`; the working copy changed; the mutating call recomputes for itself
/// and **refuses**, and the directory is still there.
#[test]
fn a_copy_that_changed_after_the_preflight_is_refused_and_survives() {
    let (_dir, root, copy) = fixture();
    let index = tempfile::tempdir().expect("index");
    let db = codotheca_core::index::Index::open(&index.path().join("index")).expect("index");

    // The pre-flight's answer, which the user acted on.
    let before = clean_inputs(&root, &copy);
    let (verdict, _seal) = compute_verdict(&before).expect("computed");
    assert_eq!(verdict.disposition, UninstallDisposition::Safe);

    // Between the two calls, the copy gains work. Nothing tells the core; that is the point.
    std::fs::write(copy.join("untracked.txt"), b"unsaved work").expect("write");
    let mut after = clean_inputs(&root, &copy);
    after.unique.push(UninstallBlocker::UntrackedPrecious);

    let warrant = Warrant::for_uninstall_in_test(
        LocationId(1),
        copy.clone(),
        identity(),
        VerdictSeal::of(&[] as &[UninstallBlocker], UninstallDisposition::Safe),
    );

    let _guard = codotheca_core::proto::txguard::TxGuard::enter();
    let tx = db.conn().unchecked_transaction().expect("tx");
    let refused = uninstall_location(&tx, &after, &warrant, Some(&identity()));

    assert!(
        refused.is_err(),
        "the mutating call must recompute and refuse, not trust the pre-flight"
    );
    assert!(
        copy.join("untracked.txt").exists(),
        "the directory and its unsaved work are still there"
    );
    assert!(copy.exists());
}

/// **AC-P2-24-17.** The row survives, ten columns read NULL and not 0, `head_oid` is retained and
/// `removed_at` is set — against a real migrated database.
#[test]
fn a_successful_removal_keeps_the_row_and_nulls_the_ten_columns() {
    let (_dir, root, copy) = fixture();
    let index = codotheca_core::testing::TempIndex::new();
    let project = index.insert_project();
    let location = index.insert_location(project, "/r/widget");

    let _guard = codotheca_core::proto::txguard::TxGuard::enter();
    let binding = index.index();
    let conn = binding.conn();
    // Seed every column the removal must clear, plus the one it must keep.
    conn.execute(
        "UPDATE location SET is_dirty = 1, untracked_count = 4, ahead = 2, behind = 1,
             stash_count = 3, interrupted_op = 'merge', branch = 'main',
             worktree_observed_at = 5, refstate_observed_at = 6, refstate_basis = 'abc',
             head_oid = 'deadbeef'
         WHERE id = ?1",
        [location.0],
    )
    .expect("seed");

    let mut inputs = clean_inputs(&root, &copy);
    inputs.snapshot.id = location;
    let warrant = Warrant::for_uninstall_in_test(
        location,
        copy.clone(),
        identity(),
        VerdictSeal::of(&[] as &[UninstallBlocker], UninstallDisposition::Safe),
    );

    let tx = conn.unchecked_transaction().expect("tx");
    let removed = uninstall_location(&tx, &inputs, &warrant, Some(&identity())).expect("removed");
    tx.commit().expect("commit");

    assert_eq!(removed.location, location);
    assert_eq!(
        removed.outcome,
        RemovalOutcome::Trashed,
        "a user's working copy goes to the trash, never a hard delete"
    );

    // The row survives.
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM location WHERE id = ?1",
            [location.0],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(rows, 1, "the row is a tombstone, not a deletion");

    // Ten columns NULL, and **not 0**.
    for column in cleared_columns() {
        let value: Option<String> = conn
            .query_row(
                &format!("SELECT CAST({column} AS TEXT) FROM location WHERE id = ?1"),
                [location.0],
                |r| r.get(0),
            )
            .expect("read back");
        assert_eq!(
            value, None,
            "{column} must be NULL — 0 would claim an observation nobody made"
        );
    }

    let (head, removed_at): (Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT head_oid, removed_at FROM location WHERE id = ?1",
            [location.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("read back");
    assert_eq!(
        head.as_deref(),
        Some("deadbeef"),
        "head_oid is retained: what a re-clone can be checked against"
    );
    assert_eq!(removed_at, Some(1_700_000_000));
}

/// R34 is undisturbed: a removal is not a job, because a job is a retry surface and a removal must
/// never be auto-replayed.
///
/// **The property is a set property and is stated as one.** It asserted `ALL.len() == 7`, which
/// goes red for any eighth job whether or not that job is a removal — §29's `j7` is not — and
/// which would have read as a removal defect. What must stay true is that no `JobKind` names a
/// removal (R132/F16).
#[test]
fn removal_added_no_job_kind() {
    let slugs: Vec<&'static str> = codotheca_core::jobs::JobKind::ALL
        .iter()
        .map(|k| k.slug())
        .collect();
    eprintln!("job slugs checked for a removal: {slugs:?}");
    assert!(!slugs.is_empty(), "the job vocabulary is empty");
    for slug in &slugs {
        for banned in ["remove", "removal", "uninstall", "delete"] {
            assert!(
                !slug.contains(banned),
                "{slug} names a removal, and a removal must never be a retry surface"
            );
        }
    }
    for kind in codotheca_core::jobs::JobKind::ALL {
        let named = format!("{kind:?}").to_lowercase();
        for banned in ["remove", "removal", "uninstall", "delete"] {
            assert!(
                !named.contains(banned),
                "{named} names a removal, and a removal must never be a retry surface"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// §24.6a: `removed_at` takes precedence over `presence`, in the producer.
// ---------------------------------------------------------------------------

/// **The uninstalled row still reads `presence = 'present'` in the database**, because `presence`
/// is a scan observation and the scan has not run. That is the whole reason the precedence lives
/// in the producer rather than in six renderers.
#[test]
fn an_uninstalled_copy_is_demoted_below_a_live_one_whatever_presence_says() {
    use codotheca_core::projects::rows::{any_present_dirty, pick_primary, LocationFacts};
    use codotheca_core::protocol::{LocationKind, Presence, ProjectId};

    fn facts(id: i64, touched: i64, removed_at: Option<i64>, dirty: Option<bool>) -> LocationFacts {
        LocationFacts {
            id: LocationId(id),
            project_id: ProjectId(1),
            kind: LocationKind::Linux,
            distro: String::new(),
            path_display: String::new(),
            // Both rows say `present`: the removal wrote `removed_at`, not this.
            presence: Presence::Present,
            branch: None,
            is_dirty: dirty,
            untracked_count: None,
            ahead: None,
            behind: None,
            stash_count: None,
            interrupted_op: None,
            head_oid: None,
            trusted_at: None,
            fetch_head_at: None,
            refstate_observed_at: None,
            worktree_observed_at: None,
            worktree_newest_mtime: Some(touched),
            last_seen_at: None,
            removed_at,
        }
    }

    // The uninstalled one is the more recently touched, so it would win on every other key.
    let uninstalled = facts(1, 2_000, Some(1_700_000_000), Some(true));
    let live = facts(2, 1_000, None, Some(false));
    let locations = vec![uninstalled, live];

    let primary = pick_primary(&locations).expect("a primary");
    assert_eq!(
        primary.id,
        LocationId(2),
        "a project with one live copy and one uninstalled copy reports the live one"
    );

    // And the aggregates exclude it: the uninstalled row is dirty, the live one is not.
    assert_eq!(
        any_present_dirty(&locations),
        Some(false),
        "a removed copy's dirtiness is not a fact about the project any more"
    );
}

/// `presence` itself is **not** nulled. §23.2 defines `ProjectRow.presence = null` as *a
/// zero-location project* and evaluates `is:notcloned` as `primary_location IS NULL`; an
/// uninstalled project **has** a location row, so it is not `is:notcloned` and not a blueprint.
#[test]
fn an_uninstalled_project_is_not_a_not_cloned_project() {
    use codotheca_core::projects::rows::{pick_primary, LocationFacts};
    use codotheca_core::protocol::{LocationKind, Presence, ProjectId};

    let only = LocationFacts {
        id: LocationId(1),
        project_id: ProjectId(1),
        kind: LocationKind::Linux,
        distro: String::new(),
        path_display: String::new(),
        presence: Presence::Present,
        branch: None,
        is_dirty: None,
        untracked_count: None,
        ahead: None,
        behind: None,
        stash_count: None,
        interrupted_op: None,
        head_oid: None,
        trusted_at: None,
        fetch_head_at: None,
        refstate_observed_at: None,
        worktree_observed_at: None,
        worktree_newest_mtime: None,
        last_seen_at: None,
        removed_at: Some(1_700_000_000),
    };
    let locations = vec![only];
    assert!(
        pick_primary(&locations).is_some(),
        "an uninstalled project still HAS a location, so it is never is:notcloned"
    );
}

/// Seed one project, one copy and one open debt item anchored at it.
fn debt_fixture() -> (
    tempfile::TempDir,
    PathBuf,
    PathBuf,
    codotheca_core::testing::TempIndex,
    codotheca_core::protocol::ProjectId,
    LocationId,
) {
    let (dir, root, copy) = fixture();
    let index = codotheca_core::testing::TempIndex::new();
    let project = index.insert_project();
    let location = index.insert_location(project, "/r/widget");
    {
        let binding = index.index();
        binding
            .conn()
            .execute(
                "INSERT INTO debt_item (project_id, subject_key, source, fingerprint, state,
                                        scoring, last_seen_location_id, basis, first_seen_at,
                                        last_seen_at)
                 VALUES (?1, 'lineage:l|remote:', 'todo_marker', 'f', 'open', 'scored', ?2,
                         'head', 1, 1)",
                rusqlite::params![project.0, location.0],
            )
            .expect("seed item");
    }
    (dir, root, copy, index, project, location)
}

fn debt_warrant(location: LocationId, copy: PathBuf) -> Warrant {
    Warrant::for_uninstall_in_test(
        location,
        copy,
        identity(),
        VerdictSeal::of(&[] as &[UninstallBlocker], UninstallDisposition::Safe),
    )
}

/// **[p3] `AC-P3-28-8`, the *one transaction* half.**
///
/// A rolled-back removal leaves **neither** change behind. A second transaction after the commit
/// would leave `removed_at` set with the items still `open`, which is the window the whole
/// ordering exists to close.
///
/// It is two tests rather than one because the removal is **not replayable**: it trashes the
/// directory, so a second call against the same copy is refused `RefusedPath`.
#[test]
fn a_rolled_back_removal_marks_no_debt_item() {
    let (_dir, root, copy, index, project, location) = debt_fixture();
    let _guard = codotheca_core::proto::txguard::TxGuard::enter();
    let binding = index.index();
    let conn = binding.conn();

    let mut inputs = clean_inputs(&root, &copy);
    inputs.snapshot.id = location;

    let tx = conn.unchecked_transaction().expect("tx");
    uninstall_location(
        &tx,
        &inputs,
        &debt_warrant(location, copy),
        Some(&identity()),
    )
    .expect("removed");
    tx.rollback().expect("rollback");

    let (state, removed_at): (String, Option<i64>) = conn
        .query_row(
            "SELECT i.state, l.removed_at FROM debt_item i
               JOIN location l ON l.id = i.last_seen_location_id
              WHERE i.project_id = ?1",
            [project.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("read back");
    assert_eq!(
        (state.as_str(), removed_at),
        ("open", None),
        "a rolled-back removal left one of the two changes behind"
    );
}

/// **[p3] `AC-P3-28-8`. The uninstall hole, and the second of two guards against it.**
///
/// `locations.uninstall` removes the bytes and **keeps the row**, so `presence` still reads
/// `present` and only `removed_at` says otherwise. A naive sweep afterwards finds a
/// readable-looking absence, reports `complete` with zero items, **closes every item and pays for
/// it**. The mark is `state = 'unverified'` and nothing else, and `last_seen_location_id` is
/// **kept**: it is what the reap later compares against.
#[test]
fn a_removal_marks_the_projects_debt_items_unverified() {
    let (_dir, root, copy, index, project, location) = debt_fixture();
    let _guard = codotheca_core::proto::txguard::TxGuard::enter();
    let binding = index.index();
    let conn = binding.conn();

    let mut inputs = clean_inputs(&root, &copy);
    inputs.snapshot.id = location;

    let tx = conn.unchecked_transaction().expect("tx");
    uninstall_location(
        &tx,
        &inputs,
        &debt_warrant(location, copy),
        Some(&identity()),
    )
    .expect("removed");
    tx.commit().expect("commit");

    let (state, anchor): (String, Option<i64>) = conn
        .query_row(
            "SELECT state, last_seen_location_id FROM debt_item WHERE project_id = ?1",
            [project.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("read back");
    assert_eq!(state, "unverified");
    assert_eq!(
        anchor,
        Some(location.0),
        "the anchor was cleared, which strands the item for ever instead of reaping it"
    );

    // A sweep run immediately afterwards reports `unobservable` and closes nothing: the root is
    // gone, however `presence` still reads.
    let presence: String = conn
        .query_row(
            "SELECT presence FROM location WHERE id = ?1",
            [location.0],
            |r| r.get(0),
        )
        .expect("presence");
    assert_eq!(presence, "present", "the fixture is not the hole it claims");

    let tx = conn.unchecked_transaction().expect("tx");
    let outcome = codotheca_core::debt::sweep::outcome_at_root(
        &tx,
        Some(location),
        codotheca_core::protocol::DebtSweepOutcome::Complete,
    )
    .expect("outcome");
    assert_eq!(
        outcome,
        codotheca_core::protocol::DebtSweepOutcome::Unobservable,
        "a sweep after an uninstall reported complete and would close every item"
    );
}
