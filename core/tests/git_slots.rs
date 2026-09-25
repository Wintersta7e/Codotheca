#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §3.4's caps: global, per store, and the tighter ceiling J4 runs under.

use std::sync::Arc;
use std::time::Duration;

use codotheca_core::cancel::CancelToken;
use codotheca_core::git::{GitError, GitSlots, JobClass, StoreKey};
use codotheca_core::mount::StoreClass;

fn slots(global: usize) -> Arc<GitSlots> {
    Arc::new(GitSlots::new(global))
}

#[test]
fn the_machine_cap_is_sixteen_or_the_core_count() {
    let s = GitSlots::for_machine();
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    assert_eq!(s.global_limit(), cores.min(16));
    assert!(s.global_limit() >= 1);
}

#[test]
fn a_slow_store_admits_one_and_a_fast_store_admits_four() {
    let s = slots(16);
    let slow = StoreKey::new("net");
    let a = s
        .acquire(
            &slow,
            StoreClass::Network,
            JobClass::Background,
            &CancelToken::new(),
        )
        .unwrap();
    let blocked = s.clone();
    let key = slow;
    let cancel = CancelToken::new();
    let waiter = cancel.clone();
    let t = std::thread::spawn(move || {
        blocked.acquire(&key, StoreClass::Network, JobClass::Background, &waiter)
    });
    std::thread::sleep(Duration::from_millis(150));
    cancel.cancel();
    assert_eq!(t.join().unwrap().unwrap_err(), GitError::Cancelled);
    drop(a);

    let fast = StoreKey::new("nvme");
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(
            s.acquire(
                &fast,
                StoreClass::Local,
                JobClass::Background,
                &CancelToken::new(),
            )
            .unwrap(),
        );
    }
    let fifth = s;
    let key = fast;
    let cancel = CancelToken::new();
    let waiter = cancel.clone();
    let t = std::thread::spawn(move || {
        fifth.acquire(&key, StoreClass::Local, JobClass::Background, &waiter)
    });
    std::thread::sleep(Duration::from_millis(150));
    cancel.cancel();
    assert_eq!(t.join().unwrap().unwrap_err(), GitError::Cancelled);
}

#[test]
fn the_global_cap_holds_across_stores() {
    let s = slots(2);
    let a = s
        .acquire(
            &StoreKey::new("a"),
            StoreClass::Local,
            JobClass::Background,
            &CancelToken::new(),
        )
        .unwrap();
    let b = s
        .acquire(
            &StoreKey::new("b"),
            StoreClass::Local,
            JobClass::Background,
            &CancelToken::new(),
        )
        .unwrap();
    let third = s;
    let cancel = CancelToken::new();
    let waiter = cancel.clone();
    let t = std::thread::spawn(move || {
        third.acquire(
            &StoreKey::new("c"),
            StoreClass::Local,
            JobClass::Background,
            &waiter,
        )
    });
    std::thread::sleep(Duration::from_millis(150));
    cancel.cancel();
    assert_eq!(t.join().unwrap().unwrap_err(), GitError::Cancelled);
    drop(a);
    drop(b);
}

#[test]
fn history_takes_at_most_a_quarter_of_the_slots_and_one_per_store() {
    let s = slots(8);
    assert_eq!(s.history_limit(), 2);
    let fast = StoreKey::new("nvme");
    let _h1 = s
        .acquire(
            &fast,
            StoreClass::Local,
            JobClass::History,
            &CancelToken::new(),
        )
        .unwrap();

    // A second history job on the SAME store is refused even though the store cap is 4.
    let same = s.clone();
    let key = fast;
    let cancel = CancelToken::new();
    let waiter = cancel.clone();
    let t = std::thread::spawn(move || {
        same.acquire(&key, StoreClass::Local, JobClass::History, &waiter)
    });
    std::thread::sleep(Duration::from_millis(150));
    cancel.cancel();
    assert_eq!(t.join().unwrap().unwrap_err(), GitError::Cancelled);

    // A history job on another store fits, and a third exceeds the class ceiling.
    let _h2 = s
        .acquire(
            &StoreKey::new("sata"),
            StoreClass::Local,
            JobClass::History,
            &CancelToken::new(),
        )
        .unwrap();
    let third = s;
    let cancel = CancelToken::new();
    let waiter = cancel.clone();
    let t = std::thread::spawn(move || {
        third.acquire(
            &StoreKey::new("usb"),
            StoreClass::Local,
            JobClass::History,
            &waiter,
        )
    });
    std::thread::sleep(Duration::from_millis(150));
    cancel.cancel();
    assert_eq!(t.join().unwrap().unwrap_err(), GitError::Cancelled);
}

#[test]
fn an_interactive_waiter_is_served_before_a_background_one() {
    let s = slots(1);
    let held = s
        .acquire(
            &StoreKey::new("a"),
            StoreClass::Local,
            JobClass::Background,
            &CancelToken::new(),
        )
        .unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let bg = s.clone();
    let bg_tx = tx.clone();
    let bg_thread = std::thread::spawn(move || {
        let g = bg
            .acquire(
                &StoreKey::new("a"),
                StoreClass::Local,
                JobClass::Background,
                &CancelToken::new(),
            )
            .unwrap();
        let _ = bg_tx.send("background");
        drop(g);
    });
    std::thread::sleep(Duration::from_millis(100));
    let it = s;
    let it_thread = std::thread::spawn(move || {
        let g = it
            .acquire(
                &StoreKey::new("a"),
                StoreClass::Local,
                JobClass::Interactive,
                &CancelToken::new(),
            )
            .unwrap();
        let _ = tx.send("interactive");
        drop(g);
    });
    std::thread::sleep(Duration::from_millis(100));
    drop(held);

    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        "interactive"
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        "background"
    );
    it_thread.join().unwrap();
    bg_thread.join().unwrap();
}

#[test]
fn a_settings_override_replaces_a_store_cap() {
    let s = slots(16);
    let store = StoreKey::new("net");
    s.set_store_limit(&store, 3);
    let mut held = Vec::new();
    for _ in 0..3 {
        held.push(
            s.acquire(
                &store,
                StoreClass::Network,
                JobClass::Background,
                &CancelToken::new(),
            )
            .unwrap(),
        );
    }
    let fourth = s;
    let key = store;
    let cancel = CancelToken::new();
    let waiter = cancel.clone();
    let t = std::thread::spawn(move || {
        fourth.acquire(&key, StoreClass::Network, JobClass::Background, &waiter)
    });
    std::thread::sleep(Duration::from_millis(150));
    cancel.cancel();
    assert_eq!(t.join().unwrap().unwrap_err(), GitError::Cancelled);
}

#[test]
fn dropping_a_guard_releases_the_slot() {
    let s = slots(1);
    let store = StoreKey::new("a");
    {
        let _g = s
            .acquire(
                &store,
                StoreClass::Local,
                JobClass::Background,
                &CancelToken::new(),
            )
            .unwrap();
    }
    let _g2 = s
        .acquire(
            &store,
            StoreClass::Local,
            JobClass::Background,
            &CancelToken::new(),
        )
        .unwrap();
}
