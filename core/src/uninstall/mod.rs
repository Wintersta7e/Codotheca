//! §24.6–§24.8: Uninstall's two commands.
//!
//! **This module declares no removal primitive** — `core/src/removal/` is p2-24's, extended with
//! one warrant variant rather than a second module — **and computes no blocker**: §45's analyser
//! (`crate::analyser`) is the one that does.

pub mod command;
pub mod handle;

pub use command::uninstall_location;
pub use handle::{handle_preflight_off_lock, handle_uninstall_off_lock};
