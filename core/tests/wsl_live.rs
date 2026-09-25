//! §13 — the part that cannot be tested without a live distro, marked so nobody mistakes a skip
//! for a pass.
//!
//!     cargo test --manifest-path core/Cargo.toml --test wsl_live -- --ignored --nocapture
//!
//! Every test here is `#[ignore]` and Windows-only. Each one uses a distro that is **already
//! running** and never starts a stopped one: §13's consent clause is not a test's to grant.
#![cfg(windows)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use codotheca_core::wsl::conn::{WslExeLauncher, WslWorkerPool};
use codotheca_core::wsl::distros::{installed_distros, DistroState, SystemWslCli};
use codotheca_core::wsl::proto::{WorkerGit, WorkerRequest};
use std::sync::Arc;

fn running_distro() -> Option<String> {
    let cli = SystemWslCli::new();
    installed_distros(&cli)
        .ok()?
        .into_iter()
        .find(|d| d.state == DistroState::Running)
        .map(|d| d.name)
}

fn worker_bytes() -> Arc<[u8]> {
    // Build it first, on a Linux target:
    //   cargo build --release --manifest-path core/Cargo.toml --bin codotheca-worker
    let path = std::env::var("CODOTHECA_WORKER_ELF")
        .expect("set CODOTHECA_WORKER_ELF to a linux codotheca-worker binary");
    Arc::from(std::fs::read(path).expect("reads the worker ELF"))
}

#[test]
#[ignore = "needs a live WSL distro"]
fn enumeration_reports_at_least_one_distro_and_starts_none() {
    let cli = SystemWslCli::new();
    let before = installed_distros(&cli).expect("lists");
    assert!(
        !before.is_empty(),
        "no distro installed; this test needs one"
    );
    let after = installed_distros(&cli).expect("lists again");
    // Listing twice must not have started anything.
    let started: Vec<&str> = after
        .iter()
        .filter(|a| {
            a.state == DistroState::Running
                && before
                    .iter()
                    .any(|b| b.name == a.name && b.state == DistroState::Stopped)
        })
        .map(|a| a.name.as_str())
        .collect();
    assert!(started.is_empty(), "listing started {started:?}");
}

#[test]
#[ignore = "needs a live WSL distro"]
fn the_worker_deploys_launches_and_answers() {
    let Some(distro) = running_distro() else {
        panic!("no running distro; start one and re-run");
    };
    let launcher = Arc::new(WslExeLauncher::new(
        Arc::new(SystemWslCli::new()),
        worker_bytes(),
        None,
    ));
    let pool = WslWorkerPool::new(launcher);
    let worker = pool.get(&distro).expect("connects");
    assert_eq!(
        worker.hello().protocol_version,
        codotheca_core::wsl::proto::WORKER_PROTOCOL_VERSION
    );
    assert_eq!(
        worker.call(&WorkerRequest::Ping).expect("pings")["pong"],
        true
    );

    // The real mount table: `/` must resolve, and it must not classify as the bridge.
    let facts = worker
        .call(&WorkerRequest::Mounts {
            path: "/".to_owned(),
        })
        .expect("resolves the root");
    assert_eq!(facts["store_key"], format!("wsl:{distro}:/"));
    assert_ne!(facts["class"], "network", "the distro root is not a bridge");

    // A second get must reuse the same worker: §13's whole point.
    let again = pool.get(&distro).expect("reuses");
    assert_eq!(again.hello().pid, worker.hello().pid);
    pool.shutdown_all();
}

#[test]
#[ignore = "needs a live WSL distro"]
fn a_second_launch_reuses_the_deployed_copy() {
    let Some(distro) = running_distro() else {
        panic!("no running distro; start one and re-run");
    };
    let bytes = worker_bytes();
    for _ in 0..2 {
        let pool = WslWorkerPool::new(Arc::new(WslExeLauncher::new(
            Arc::new(SystemWslCli::new()),
            Arc::clone(&bytes),
            None,
        )));
        let worker = pool.get(&distro).expect("connects");
        assert!(matches!(
            worker.git(),
            WorkerGit::Present { .. } | WorkerGit::Missing { .. }
        ));
        pool.shutdown_all();
    }
}
