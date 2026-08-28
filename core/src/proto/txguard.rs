//! The database-lock / pipe-write interlock.
//!
//! §2.2's deadlock rule forbids holding a database lock while awaiting a pipe write. This
//! makes that checkable: the writer refuses rather than blocks, so the bug surfaces as a
//! returned error at the call site instead of as a hung process at 3 a.m.

use std::cell::Cell;

thread_local! {
    static TX_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// RAII marker: a database transaction is open on this thread.
#[derive(Debug)]
pub struct TxGuard(());

impl TxGuard {
    #[must_use]
    pub fn enter() -> Self {
        TX_DEPTH.with(|d| d.set(d.get().saturating_add(1)));
        Self(())
    }
}

impl Drop for TxGuard {
    fn drop(&mut self) {
        TX_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// True while any `TxGuard` is alive on this thread.
#[must_use]
pub fn in_transaction() -> bool {
    TX_DEPTH.with(Cell::get) > 0
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{in_transaction, TxGuard};

    #[test]
    fn nests_and_unwinds() {
        assert!(!in_transaction());
        let outer = TxGuard::enter();
        let inner = TxGuard::enter();
        assert!(in_transaction());
        drop(inner);
        assert!(in_transaction(), "the outer transaction is still open");
        drop(outer);
        assert!(!in_transaction());
    }

    #[test]
    fn the_counter_is_per_thread() {
        let _held = TxGuard::enter();
        let elsewhere = std::thread::spawn(in_transaction).join();
        assert_eq!(elsewhere.ok(), Some(false));
    }
}
