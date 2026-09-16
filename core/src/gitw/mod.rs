//! The git **write** boundary (§24.1).
//!
//! **An invocation may create bytes and may never remove or overwrite one.** Phase 2's whole git
//! write surface is two subcommands — `clone` and `fetch`, and `fetch` never with `--prune`.
//! `push`, `pull`, `checkout`/`switch` and `worktree` are phase 3, and everything that removes or
//! overwrites — `clean`, `reset`, `restore`, `rm`, `stash`, `gc`, `prune`, `commit`, `merge`,
//! `rebase`, `update-ref`, `write-tree` — is never at this boundary at all.
//!
//! This module is the **sibling** of [`crate::git`], not a widening of it. That module stays
//! provably read-only: its `ALLOWED` and `FORBIDDEN` lists and its `source_files()` glob are
//! byte-identical to phase 1's, and `fetch` stays denied *for that directory* forever. Mutating
//! argv lives here, and `core/tests/git_write_audit.rs` is its audit.
//!
//! **The audit enumerates variants; it does not grep source text.** [`Intent`] is closed and
//! [`Intent::argv`] is total over it, so the flag denylist — `--prune`, `--force`, `-f`,
//! `--hard`, `--delete`, `-d`, `-D`, `--mirror` — can be asserted against what the product
//! actually renders. The phase-1 audit structurally cannot make that assertion: it skips every
//! literal beginning with `-` except `--version`, and *the difference between an additive
//! `fetch` and a destructive one is a flag*.
//!
//! > `concept.md`'s *"Git: fetch, pull, push, new branch, clone"* is a **feature list, not a
//! > phase list**. The four exclusions above are scheduling, not omission, and a reader who takes
//! > that line as a phase list adds them back.

pub mod intent;

#[cfg(feature = "testkit")]
pub use intent::AuditFixture;
pub use intent::{Intent, IntentKind, IntentRefusal, RemoteName, RemoteUrl};
