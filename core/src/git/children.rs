//! Every git child the core starts is admitted here, so an orderly exit can end them all (§3.4).
//!
//! On Windows a child is created suspended, put in its Job Object and only then resumed, so a
//! core that exits while a thread is between those steps leaves a child that never runs and never
//! ends, holding every handle it inherited — the shell's pipes among them. Closing therefore
//! refuses new children, tells the running ones to stop, and waits for each spawn to come back
//! and each child to be killed and reaped before the process may exit. A run that is still waiting
//! on its child polls [`Children::is_closing`] beside its own cancel token.

use std::process::Command;
use std::sync::{Condvar, Mutex, PoisonError};

use command_group::{CommandGroup, GroupChild};
use std::time::{Duration, Instant};

/// The process-wide register: one core, one set of children.
pub(crate) static CHILDREN: Children = Children::new();

/// How many children are between admission and reaping, and whether the core is closing.
#[derive(Debug)]
pub(crate) struct Children {
    state: Mutex<State>,
    settled: Condvar,
}

#[derive(Debug)]
struct State {
    closing: bool,
    live: usize,
}

/// One admitted child. Held from before its spawn until it has been waited for; dropping it is
/// what lets a closing core go.
#[derive(Debug)]
pub(crate) struct Admitted<'a>(&'a Children);

impl Children {
    pub(crate) const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                closing: false,
                live: 0,
            }),
            settled: Condvar::new(),
        }
    }

    /// Admit one child, or `None` once the core is closing: a spawn after that point would start
    /// a process nothing is left to end.
    pub(crate) fn admit(&self) -> Option<Admitted<'_>> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.closing {
            return None;
        }
        state.live += 1;
        drop(state);
        Some(Admitted(self))
    }

    /// Whether the core has begun to close; a run waiting on its child kills it when this is set.
    pub(crate) fn is_closing(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closing
    }

    /// Refuse new children and wait up to `limit` for every admitted one to be released.
    /// Returns how many were still live when the wait ended.
    pub(crate) fn close(&self, limit: Duration) -> usize {
        let until = Instant::now() + limit;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.closing = true;
        while state.live > 0 {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            state = self
                .settled
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        state.live
    }
}

impl Drop for Admitted<'_> {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.live = state.live.saturating_sub(1);
        drop(state);
        self.0.settled.notify_all();
    }
}

/// Start `cmd` as its own process group: a Job Object on Windows, created to kill every process
/// in it when its last handle closes — on drop, or when the core exits without closing.
#[cfg(windows)]
pub(crate) fn spawn_group(cmd: &mut Command) -> std::io::Result<GroupChild> {
    cmd.group().kill_on_drop(true).spawn()
}

/// Start `cmd` as its own process group. Unix has no kill-on-close; an exiting core ends its
/// children through [`close_children`], and one it never reached is reparented and ends with its
/// own work, its pipes to the core closed.
#[cfg(not(windows))]
pub(crate) fn spawn_group(cmd: &mut Command) -> std::io::Result<GroupChild> {
    cmd.group_spawn()
}

/// End every git child before the core exits.
///
/// Refuses new spawns, has each running one killed with its process group, and waits up to
/// `limit` for all of them. Returns how many were still live when the wait ended — zero on an
/// orderly exit.
///
/// Called once, after the dispatch loop has stopped and before `main` returns.
#[must_use]
pub fn close_children(limit: Duration) -> usize {
    CHILDREN.close(limit)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::Children;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn close_waits_for_an_admitted_child_and_then_refuses_new_ones() {
        let children = Arc::new(Children::new());
        let held = Arc::clone(&children);
        let (admitted, release) = std::sync::mpsc::channel::<()>();
        let worker = std::thread::spawn(move || {
            let guard = held.admit().expect("admitted while open");
            admitted.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(150));
            drop(guard);
        });
        release.recv().unwrap();
        let started = Instant::now();
        assert_eq!(children.close(Duration::from_secs(10)), 0);
        assert!(
            started.elapsed() >= Duration::from_millis(100),
            "close returned before the admitted child was released"
        );
        worker.join().unwrap();
        assert!(children.is_closing());
        assert!(children.admit().is_none(), "a closing core admits no child");
    }

    #[test]
    fn close_gives_up_at_its_limit_and_names_what_is_left() {
        let children = Children::new();
        let _held = children.admit().expect("admitted");
        assert_eq!(children.close(Duration::from_millis(20)), 1);
    }
}
