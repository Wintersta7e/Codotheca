//! §32 — dependency health: **what this library installs, and what the forge says about it.**
//!
//! One process-wide, unauthenticated, scheduled sync task reads the forge's global advisories
//! endpoint; a bounded worktree read finds each project's lockfiles; and the verdict is the join
//! of the two, derived at read time and stored nowhere.
//!
//! **Three things this module does not own.** The debt item's DDL, its states and its writer are
//! §28's — this module produces `dependency_advisory` items *through* that writer and declares no
//! second one. The `deps` completion check is §31's, and is an aggregate consumer of the item set
//! rather than one item keyed `deps`. The health arithmetic an `unknown` verdict is excluded from
//! is §30's; what this module owns is the input state.

pub mod store;

use crate::protocol::{DependencyReadState, Ecosystem};

/// An inherent const on a generated enum: legal because both are in this crate.
///
/// **The set is a property of this app's parser coverage**, not of the source's vocabulary — an
/// ecosystem is listed once a lockfile of its shape can be read. Every slug is character-identical
/// to the endpoint's own `ecosystem` parameter because it is sent as one.
impl Ecosystem {
    pub const ALL: [Ecosystem; 3] = [Ecosystem::Npm, Ecosystem::Rust, Ecosystem::Pip];
}

/// Two variants and deliberately not three: *the scan has not run* is the **absence** of a
/// `project_dependency_scan` row, because a file that was not read produces no `(package,
/// version)` key for a third variant to sit on.
impl DependencyReadState {
    pub const ALL: [DependencyReadState; 2] =
        [DependencyReadState::Parsed, DependencyReadState::NotRead];
}
