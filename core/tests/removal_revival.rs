//! §24.6a's rule and §46.9's revival, through the scan's hand-off. The repository a removed copy
//! held, found again at its path, takes its row back; a removed project whose repository is found
//! again is no longer removed; a copy that was only offline revives nothing; a repository with no
//! lineage never takes a removed row. A reclaim pays nothing.
//!
//! Compiled only under `testkit`: the reinstall case drives the install with the fake git seam.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/handoff_rig.rs"]
mod handoff_rig;

use std::path::{Path, PathBuf};

use codotheca_core::art::testsupport::CollectingSink;
use codotheca_core::assembly::handoff::{hand_off_discovered, HandoffCtx, Indexed};
use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{RepoFacts, RootCommit};
use codotheca_core::identity::decide::IdentityDecision;
use codotheca_core::install::queue::InstallRequest;
use codotheca_core::install::run::{paths_for, run_install, InstallCtx, RootFacts};
use codotheca_core::install::state::InstallStateStore;
use codotheca_core::jobs::NullJobSink;
use codotheca_core::mount::{MountFacts, StoreClass};
use codotheca_core::paths::{path_bytes, path_display, path_key};
use codotheca_core::protocol::{InstallDestination, InstallRunId, RootId};
use codotheca_core::scan::discover::{RepoCandidate, RepoKind};
use codotheca_core::scan::run::{platform_of, Discovered};
use codotheca_core::testing::{
    CloneBehaviour, FakeGitBackend, FakeMountResolver, FakeMutatingGit, GitReply,
};
use handoff_rig::{git_at, Rig, NOW};

/// A location row's `(removed_at, presence)`.
fn location_state(rig: &Rig, location: i64) -> (Option<i64>, String) {
    rig.read(|conn| {
        conn.query_row(
            "SELECT removed_at, presence FROM location WHERE id = ?1",
            [location],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    })
}

fn project_removed_at(rig: &Rig, project: i64) -> Option<i64> {
    rig.read(|conn| {
        conn.query_row(
            "SELECT removed_at FROM project WHERE id = ?1",
            [project],
            |r| r.get(0),
        )
        .unwrap()
    })
}

/// The user declares the whole project removed. Nothing writes this in production yet, so the
/// fixture does.
fn remove_project(rig: &Rig, project: i64) {
    rig.execute(
        "UPDATE project SET removed_at = ?2 WHERE id = ?1",
        [project, NOW],
    );
}

/// A `todo_marker` sweep that completed at the copy before it was removed.
fn plant_sweep(rig: &Rig, copy: Indexed) {
    rig.execute(
        "INSERT INTO debt_sweep (project_id, source, outcome, location_id, item_count,
                                 observed_at)
         VALUES (?1, 'todo_marker', 'complete', ?2, 0, ?3)",
        [copy.project.0, copy.location.0, NOW],
    );
}

fn sweep(rig: &Rig) -> String {
    rig.read(|conn| {
        conn.query_row(
            "SELECT outcome FROM debt_sweep WHERE source = 'todo_marker'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    })
}

/// Every XP event, as its count and its dedupe keys: the table stores no amount, so a reclaim
/// that paid anything shows as a changed row set.
fn xp(rig: &Rig) -> (i64, String) {
    rig.read(|conn| {
        conn.query_row(
            "SELECT count(*), COALESCE(group_concat(dedupe_key, ' '), '') FROM xp_events",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    })
}

/// The copy is uninstalled, its directory moved aside and back — the same repository — and the
/// walk finds it at its path: it takes its own row back, and the sweeps anchored at it no longer
/// read what they read before the bytes went.
#[test]
fn ac_p4_46_18_a_root_matching_copy_at_an_uninstalled_path_clears_removed_at() {
    let rig = Rig::new();
    let path = rig.repo("x", "first");
    let a = rig.hand_off(&path);
    plant_sweep(&rig, a);
    let paid = xp(&rig);
    rig.remove_row(a.location);
    let aside = rig.path("aside");
    std::fs::rename(&path, &aside).unwrap();
    std::fs::rename(&aside, &path).unwrap();

    let back = rig.hand_off(&path);
    let state = location_state(&rig, a.location.0);
    eprintln!(
        "indexed {a:?}, found again {back:?}; L1 {state:?}, {} rows, sweep {}, xp {:?}",
        rig.count("location"),
        sweep(&rig),
        xp(&rig)
    );
    assert_eq!(back, a, "the copy did not take its own row back");
    assert_eq!(
        state.0, None,
        "removed_at still set after a root-matching find"
    );
    assert_eq!(rig.count("location"), 1);
    assert_eq!(sweep(&rig), "unobservable");
    assert_eq!(xp(&rig), paid, "the reclaim paid");
}

#[test]
fn ac_p4_46_18_a_root_matching_find_clears_project_removed_at() {
    let rig = Rig::new();
    let path = rig.repo("x", "first");
    let a = rig.hand_off(&path);
    let paid = xp(&rig);
    rig.remove_row(a.location);
    remove_project(&rig, a.project.0);
    let aside = rig.path("aside");
    std::fs::rename(&path, &aside).unwrap();
    std::fs::rename(&aside, &path).unwrap();

    let back = rig.hand_off(&path);
    let state = location_state(&rig, a.location.0);
    let project = project_removed_at(&rig, a.project.0);
    eprintln!("found again {back:?}; L1 {state:?}, project removed_at {project:?}");
    assert_eq!(back, a);
    assert_eq!(
        state.0, None,
        "removed_at still set after a root-matching find"
    );
    assert_eq!(
        project, None,
        "the project is still removed after its copy came back"
    );
    assert_eq!(xp(&rig), paid, "the reclaim paid");
}

/// The removed project's repository is found at another path — the same root commit, so the same
/// lineage: a new row for the project, and the project is no longer removed. The old row stays
/// removed.
#[test]
fn ac_p4_46_18_a_removed_project_found_at_a_new_path_is_revived() {
    let rig = Rig::new();
    let path = rig.repo("x", "first");
    let a = rig.hand_off(&path);
    let paid = xp(&rig);
    rig.uninstall(a.location, &path);
    remove_project(&rig, a.project.0);

    let elsewhere = rig.repo("y", "first");
    let found = rig.hand_off(&elsewhere);
    let old = location_state(&rig, a.location.0);
    let project = project_removed_at(&rig, a.project.0);
    eprintln!("indexed {a:?}, found at a new path {found:?}; L1 {old:?}, project {project:?}");
    assert_eq!(
        found.project, a.project,
        "the repository is not its project's"
    );
    assert_ne!(found.location, a.location);
    assert_eq!(
        project, None,
        "the project is still removed after its repository came back"
    );
    assert_eq!(old.0, Some(NOW), "the removed copy's row came back with it");
    assert_eq!(xp(&rig), paid, "the revival paid");
}

/// The URL every clone below names. A local path canonicalises to no remote at all, so a clone
/// made from one would carry no remote; this one canonicalises like a hosted repository's.
const REMOTE_URL: &str = "https://example.invalid/fixture/project.git";

/// A bare repository holding one commit, at the path `REMOTE_URL` is rewritten to.
fn fixture_remote(rig: &Rig) {
    let seed = rig.repo("seed", "first");
    let bare = rig.path("remotes").join("fixture").join("project.git");
    git_at(
        rig.dir.path(),
        &rig.path("home"),
        &[
            "clone",
            "--bare",
            seed.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
}

/// `git clone REMOTE_URL <tmp>/<name>`, rewritten on this one command line to the bare fixture:
/// the clone records the URL as its `origin`, and nothing leaves the machine.
fn clone_at(rig: &Rig, name: &str) -> PathBuf {
    let dest = rig.path(name);
    let rewrite = format!(
        "url.{}/.insteadOf=https://example.invalid/",
        rig.path("remotes").display()
    );
    git_at(
        rig.dir.path(),
        &rig.path("home"),
        &["-c", &rewrite, "clone", REMOTE_URL, dest.to_str().unwrap()],
    );
    dest
}

fn association_kind(rig: &Rig, project: i64) -> Option<String> {
    rig.read(|conn| {
        conn.query_row(
            "SELECT association_kind FROM project WHERE id = ?1",
            [project],
            |r| r.get(0),
        )
        .unwrap()
    })
}

/// What Restore does: the removed project is cloned again from its own remote, at another path.
/// The clone joins the project by that remote — the removed copy's row is evidence of nothing —
/// on a new row, and the project is no longer removed.
#[test]
fn a_removed_project_with_a_remote_cloned_to_a_new_path_is_revived() {
    let rig = Rig::new();
    fixture_remote(&rig);
    let path = clone_at(&rig, "x");
    let a = rig.hand_off(&path);
    let paid = xp(&rig);
    let kind_before = association_kind(&rig, a.project.0);
    rig.uninstall(a.location, &path);
    remove_project(&rig, a.project.0);

    let elsewhere = clone_at(&rig, "y");
    let decision = rig.decision(&elsewhere);
    let found = rig.hand_off(&elsewhere);
    let old = location_state(&rig, a.location.0);
    let project = project_removed_at(&rig, a.project.0);
    let kind_after = association_kind(&rig, a.project.0);
    eprintln!(
        "indexed {a:?}; decision for the clone at a new path: {decision:?}; found {found:?}; \
         L1 {old:?}, project removed_at {project:?}, association {kind_before:?} -> \
         {kind_after:?}, {} location rows, xp {:?}",
        rig.count("location"),
        xp(&rig)
    );
    assert_eq!(
        decision,
        IdentityDecision::AttachStrong {
            project_id: a.project.0
        },
        "the clone did not join its project by its remote"
    );
    assert_eq!(found.project, a.project, "the clone is not its project's");
    assert_ne!(found.location, a.location, "the clone took the removed row");
    assert_eq!(rig.count("location"), 2);
    assert_eq!(kind_after.as_deref(), Some("strong"));
    assert_eq!(
        project, None,
        "the project is still removed after its repository came back"
    );
    assert_eq!(old.0, Some(NOW), "the removed copy's row came back with it");
    assert_eq!(xp(&rig), paid, "the revival paid");
}

/// The identity probe the install uses, answering for the repository at `dest` with one root
/// commit and one remote. One probe serves both finds, so both lineages come from one root commit
/// and one function.
fn install_probe(dest: &Path, url: &str) -> FakeGitBackend {
    let probe = FakeGitBackend::new();
    probe.always_repo_facts(GitReply::Ok(RepoFacts {
        is_bare: false,
        is_shallow: false,
        git_dir: dest.join(".git"),
        common_dir: dest.join(".git"),
    }));
    probe.always_root_commits(GitReply::Ok(vec![RootCommit {
        oid: "a".repeat(40),
        committed_at: 1_700_000_000,
        tz_offset_min: 0,
    }]));
    probe.always_remote_urls(GitReply::Ok(vec![("origin".to_owned(), url.to_owned())]));
    probe
}

/// What the walk hands on for the copy at `dest`, in the shape the install's own hand-off builds.
fn discovered_at(dest: &Path) -> Discovered {
    Discovered {
        candidate: RepoCandidate {
            path: dest.to_path_buf(),
            kind: RepoKind::WorkTree,
            git_dir: dest.join(".git"),
            common_dir: dest.join(".git"),
        },
        root_id: 1,
        kind: "linux".to_owned(),
        distro: String::new(),
        path_bytes: path_bytes(dest),
        path_key: path_key(dest, platform_of("linux")),
        path_display: path_display(dest),
        store_key: "store".to_owned(),
        volume_key: Some("vol".to_owned()),
    }
}

/// Through the install: the copy is indexed with the probe the install uses, uninstalled, and
/// installed again at the same root under the same name. The install's hand-off takes the row
/// back.
#[test]
fn ac_p4_46_18_reinstalling_at_the_uninstalled_path_clears_removed_at() {
    const URL: &str = "https://forge.example/owner/alpha";
    let rig = Rig::new();
    let root = rig.path("root");
    std::fs::create_dir_all(&root).unwrap();
    let paths = paths_for(&root, "alpha").unwrap();
    let dest = paths.destination.clone();
    let probe = install_probe(&dest, URL);
    let mounts = FakeMountResolver::new();
    mounts.map(
        root.clone(),
        MountFacts {
            store_key: "store".to_owned(),
            volume_key: Some("vol".to_owned()),
            class: StoreClass::Local,
        },
    );
    let cancel = CancelToken::new();

    std::fs::create_dir_all(dest.join(".git")).unwrap();
    let handoff = HandoffCtx {
        git: &probe,
        cancel: &cancel,
        store_class: StoreClass::Local,
        generation: 1,
        now: NOW,
    };
    let original = hand_off_discovered(&rig.index, &handoff, &discovered_at(&dest)).unwrap();
    let paid = xp(&rig);
    rig.uninstall(original.location, &dest);

    let git = FakeMutatingGit::new(CloneBehaviour::Succeed);
    let jobs = NullJobSink;
    let stages = InstallStateStore::new();
    let events = CollectingSink::default();
    let ctx = InstallCtx {
        git: &git,
        probe: &probe,
        index: &rig.index,
        jobs: &jobs,
        mounts: &mounts,
        stages: &stages,
        events: &events,
        cancel: &cancel,
        now: NOW,
    };
    let request = InstallRequest {
        project: original.project,
        root: RootId(1),
        destination: InstallDestination {
            root_id: RootId(1),
            seed_basename: "alpha".to_owned(),
            display: "<root>/alpha".to_owned(),
        },
    };
    let facts = RootFacts {
        root_id: 1,
        path: root,
        kind: "linux".to_owned(),
        distro: String::new(),
    };
    let installed = run_install(&ctx, InstallRunId(1), &request, &facts, &paths, URL).unwrap();
    let state = location_state(&rig, original.location.0);
    eprintln!(
        "indexed {original:?}, reinstalled at {installed:?}; L1 {state:?}, {} rows",
        rig.count("location")
    );
    assert_eq!(
        installed, original.location,
        "the reinstall wrote another row"
    );
    assert_eq!(state.0, None, "removed_at still set after the reinstall");
    assert_eq!(rig.count("location"), 1);
    assert_eq!(xp(&rig), paid, "the reinstall paid");
}

/// A copy the user never removed comes back from offline while its project is removed: the copy
/// is present again and the project stays removed until the user takes the removal back.
#[test]
fn an_offline_copy_returning_does_not_revive_its_removed_project() {
    let rig = Rig::new();
    let path = rig.repo("x", "first");
    let a = rig.hand_off(&path);
    remove_project(&rig, a.project.0);
    rig.execute(
        "UPDATE location SET presence = 'offline' WHERE id = ?1",
        [a.location.0],
    );

    let back = rig.hand_off(&path);
    let state = location_state(&rig, a.location.0);
    let project = project_removed_at(&rig, a.project.0);
    eprintln!("found again {back:?}; L1 {state:?}, project removed_at {project:?}");
    assert_eq!(back, a);
    assert_eq!(state, (None, "present".to_owned()));
    assert_eq!(
        project,
        Some(NOW),
        "an offline copy's return revived its removed project"
    );
}

/// Two never-committed repositories at one path have no lineage to tell them apart, so the second
/// never takes the first's removed row: it gets its own, and the removed row keeps its project.
#[test]
fn a_lineage_less_repository_never_reclaims_a_removed_row() {
    let rig = Rig::new();
    let path = rig.empty_repo("x");
    let a = rig.hand_off(&path);
    rig.uninstall(a.location, &path);

    rig.empty_repo("x");
    let b = rig.hand_off(&path);
    let old = location_state(&rig, a.location.0);
    eprintln!(
        "indexed {a:?}, then {b:?}; L1 {old:?}, {} rows",
        rig.count("location")
    );
    assert_ne!(
        b.location, a.location,
        "a repository with no lineage took a removed row"
    );
    assert_eq!(old.0, Some(NOW), "the removed row was revived");
    assert_eq!(rig.count("location"), 2);
}
