//! §56.5 row 11 (AC-P4-56-25): the in-distro walk receives the user's names and that distro's
//! path entries as bytes, and the worker's walk skips both.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use codotheca_core::exclusion::gate::{ExclusionGate, GateSite};
use codotheca_core::exclusion::list::{ExclusionList, PathEntry};
use codotheca_core::index::path::PathPlatform;
use codotheca_core::paths::path_key;
use codotheca_core::scan::wsl::WslBridgeRef;
use codotheca_core::scan::{WalkEvent, WalkOptions};
use codotheca_core::testing::wsl::LoopbackLauncher;
use codotheca_core::testing::FakeGitBackend;
use codotheca_core::wsl::conn::WslWorkerPool;
use codotheca_core::wsl::dispatch::{walk_request, DistroOutcome, WslDispatcher};
use codotheca_core::wsl::distros::{DistroInfo, DistroState};
use codotheca_core::wsl::mounts::MountTable;
use codotheca_core::wsl::proto::{WorkerGit, WorkerRequest};
use codotheca_core::wsl::serve::WorkerContext;

const MOUNTINFO: &str = "28 1 8:32 / / rw - ext4 /dev/sdc rw\n";

fn entry(distro: &str, path: &Path) -> PathEntry {
    PathEntry {
        distro: distro.to_owned(),
        platform: PathPlatform::Unix,
        match_key: path_key(path, PathPlatform::Unix),
    }
}

/// A directory name holding a byte no UTF-8 string can, where the host can name one.
#[cfg(unix)]
fn private_name() -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(b"priv\xff"))
}

#[cfg(not(unix))]
fn private_name() -> PathBuf {
    PathBuf::from("priv")
}

#[test]
fn ac_p4_56_25_the_request_carries_the_names_and_this_distros_path_bytes() {
    let d_path = Path::new("/home/u").join(private_name());
    let gate = ExclusionGate::new(ExclusionList::new(
        &["scratch".to_owned()],
        vec![entry("d", &d_path), entry("e", Path::new("/home/u/other"))],
    ));

    let request = walk_request(&gate, "d", "/home/u", &WalkOptions::default());

    assert_eq!(request.skip_extra, vec!["scratch".to_owned()]);
    assert_eq!(
        request.skip_paths,
        vec![path_key(&d_path, PathPlatform::Unix)],
        "distro d's entry only, its bytes intact"
    );
    // The bytes survive the wire: a lossy string would have replaced the non-UTF-8 byte.
    let wire = serde_json::to_vec(&WorkerRequest::Walk(request)).expect("serialises");
    let WorkerRequest::Walk(back) = serde_json::from_slice(&wire).expect("parses") else {
        panic!("not a walk");
    };
    assert_eq!(back.skip_paths, vec![path_key(&d_path, PathPlatform::Unix)]);
    assert_eq!(gate.checks(), vec![(GateSite::Wsl, true)]);
    eprintln!(
        "request: {} user name(s), {} of 2 path entries (distro d), bytes intact across the wire",
        back.skip_extra.len(),
        back.skip_paths.len()
    );
}

/// Walks `root` through the production dispatcher and a loopback worker; answers the
/// discovered repositories' directory names.
fn discovered(root: &Path, gate: ExclusionGate) -> BTreeSet<String> {
    let launcher = Arc::new(LoopbackLauncher::new(|distro: &str| WorkerContext {
        distro: distro.to_owned(),
        git: Box::new(FakeGitBackend::new()),
        mounts: MountTable::from_mountinfo(MOUNTINFO),
        presence: WorkerGit::Missing {
            detail: "not on PATH".to_owned(),
        },
    }));
    let dispatcher = WslDispatcher::new(
        Arc::new(WslWorkerPool::new(launcher)),
        vec![DistroInfo {
            name: "up".to_owned(),
            state: DistroState::Running,
        }],
        BTreeSet::new(),
        Arc::new(gate),
    );
    let seen: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    let sink = |event: WalkEvent| {
        if let WalkEvent::Discovered(d) = event {
            let name = Path::new(&d.path_display)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            seen.lock().expect("lock").insert(name);
        }
    };
    let outcome = dispatcher.scan(
        &WslBridgeRef {
            distro: "up".to_owned(),
        },
        &root.to_string_lossy(),
        7,
        &WalkOptions::default(),
        &sink,
    );
    assert!(
        matches!(outcome, DistroOutcome::Scanned { .. }),
        "outcome was {outcome:?}"
    );
    seen.into_inner().expect("lock")
}

#[test]
fn ac_p4_56_25_the_workers_walk_skips_a_user_name_and_a_path_entry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    for name in [
        PathBuf::from("keep"),
        PathBuf::from("scratch"),
        private_name(),
    ] {
        let git = root.join(name).join(".git");
        std::fs::create_dir_all(&git).expect("mkdir");
        std::fs::write(git.join("HEAD"), b"ref: refs/heads/main\n").expect("HEAD");
    }

    // The control: with the built-ins alone the worker finds all three, so a skip below is the
    // list's doing and not a walk that found nothing.
    let all = discovered(root, ExclusionGate::default());
    assert_eq!(all.len(), 3, "control walk found {all:?}");

    let gate = ExclusionGate::new(ExclusionList::new(
        &["scratch".to_owned()],
        vec![
            entry("up", &root.join(private_name())),
            // Another distro's entry for the kept repository must not cross.
            entry("other", &root.join("keep")),
        ],
    ));
    let kept = discovered(root, gate);
    assert_eq!(kept, BTreeSet::from(["keep".to_owned()]));
    eprintln!(
        "worker walk: {} of {} repositories discovered; skipped by name 1, by path 1",
        kept.len(),
        all.len()
    );
}
