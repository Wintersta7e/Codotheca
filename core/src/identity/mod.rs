//! §1.1, §1.5 and §1.6 — lineage, association kinds, merge and redirect. **Plan 08's module.**
//!
//! Four layers. Two pure ones — [`remote`] and [`lineage`] — turn raw git output into evidence;
//! [`decide`] maps that evidence plus the projects already indexed onto one of six decisions with
//! no I/O at all; [`store`], [`merge`], [`redirect`] and [`submodule`] apply a decision through a
//! borrowed `rusqlite::Transaction`. **Git is never spawned from here**: this module exports the
//! argv vectors and the parsers for their output, and they are run through `GitBackend` so a
//! test can inject slow, failing and torn-read git (§15.2). [`probe`] composes the three reads
//! §1.1 needs into one `IdentityProbe` — through the seam, holding no lock and no connection.
//!
//! Every function that writes takes `&rusqlite::Transaction<'_>` and an explicit `now: i64`. The
//! caller owns the transaction and the clock, which is what lets §1.5 be one transaction and
//! keeps the `Clock` seam out of this module.

pub mod commands;
pub mod confirm;
pub mod decide;
pub mod lineage;
pub mod merge;
pub mod people;
pub mod probe;
pub mod redirect;
pub mod remote;
pub mod store;
pub mod submodule;
#[cfg(test)]
pub mod testutil;
pub mod user;

use crate::protocol::ErrorCode;

/// Every failure this module can produce. The `message` a caller derives from it is diagnostic
/// and is never shown raw (§2.4); the user-facing prose belongs to the shell.
#[derive(Debug)]
pub enum IdentityError {
    Sqlite(rusqlite::Error),
    /// A read that had to go through `core::index` — §1.10 gives `path_display` exactly one
    /// door, so a surface that renders a path borrows the index's error rather than losing it.
    Index(crate::index::IndexError),
    /// No `project` row with this id, and no redirect for it either.
    UnknownProject(i64),
    /// The id resolved to a tombstoned row: the caller holds a stale id and must refresh (§1.6).
    ProjectMerged {
        requested: i64,
        into: i64,
    },
    /// A redirect pointed at a row that is itself tombstoned. Redirects resolve one hop (§1.6),
    /// so this is a defect in a writer, not a case to chain through.
    RedirectChain {
        requested: i64,
        via: i64,
    },
    /// `merge_projects` was asked to merge a project into itself.
    SameProject(i64),
}

impl From<rusqlite::Error> for IdentityError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl IdentityError {
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::ProjectMerged { .. } => ErrorCode::ProjectMerged,
            Self::Sqlite(_)
            | Self::Index(_)
            | Self::UnknownProject(_)
            | Self::RedirectChain { .. }
            | Self::SameProject(_) => ErrorCode::Internal,
        }
    }
}

/// How the locations under one project came to be under it (§1.1). Stored as text in
/// `project.association_kind` and rendered as §8.5.2's Locations footer.
///
/// **R31: declared in `protocol/schema/protocol.json` and generated into `crate::protocol`.**
/// Re-exported so this module's path still names it, and declared nowhere else — a second
/// hand-written copy compiles and then drifts from the wire form, and this one is *also* a
/// stored vocabulary with two DDL CHECKs behind it (`project.association_kind` and
/// `merge_record.association_kind`), so a drift fails at insert time rather than in review.
/// Same treatment as `LocationKind` in `core/src/derive.rs` and `Presence` in
/// `core/src/scan/presence.rs`; the three methods below are an inherent impl on the generated
/// type, which is legal because both modules are in this crate.
pub use crate::protocol::AssociationKind;

impl AssociationKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Definitive => "definitive",
            Self::Strong => "strong",
            Self::Inferred => "inferred",
            Self::Manual => "manual",
        }
    }

    /// `as_str`'s inverse. `None` for anything else: a kind this build does not know is a row
    /// from a newer schema, and guessing would name evidence the app does not have.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "definitive" => Some(Self::Definitive),
            "strong" => Some(Self::Strong),
            "inferred" => Some(Self::Inferred),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }

    /// The footer names one evidence for all of a project's copies, so it must name the
    /// weakest — except that `manual` is a statement by the user and outranks every inference.
    #[must_use]
    pub fn combine(self, other: Self) -> Self {
        if self == Self::Manual || other == Self::Manual {
            return Self::Manual;
        }
        if self.rank() <= other.rank() {
            self
        } else {
            other
        }
    }

    const fn rank(self) -> u8 {
        match self {
            Self::Inferred => 0,
            Self::Strong => 1,
            Self::Definitive => 2,
            Self::Manual => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::AssociationKind::{Definitive, Inferred, Manual, Strong};

    const PROJECT_DDL: &str = include_str!("../../migrations/0001_meta_and_projects.sql");

    #[test]
    fn association_kind_round_trips_through_its_stored_text() {
        for k in [Definitive, Strong, Inferred, Manual] {
            assert_eq!(super::AssociationKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(super::AssociationKind::parse("weak"), None);
        assert_eq!(Definitive.as_str(), "definitive");
    }

    /// R31's regression guard. The kind is generated from `protocol.json`, so the column
    /// spelling and the wire spelling are one value; a hand-written second copy would compile
    /// and then drift. Same test `LocationKind` and `Presence` already carry.
    #[test]
    fn the_stored_text_and_the_wire_form_are_one_value() {
        for k in [Definitive, Strong, Inferred, Manual] {
            let json = serde_json::to_string(&k).unwrap();
            assert_eq!(json, format!("\"{}\"", k.as_str()));
            assert_eq!(
                serde_json::from_str::<super::AssociationKind>(&json).unwrap(),
                k
            );
            assert!(
                PROJECT_DDL.contains(&format!("'{}'", k.as_str())),
                "association_kind '{}' is emitted by the core and rejected by the DDL",
                k.as_str()
            );
        }
    }

    #[test]
    fn combining_keeps_the_weakest_evidence_and_manual_always_wins() {
        // §8.5.2 draws one footer for the copies a project has, so the footer must name the
        // weakest evidence that put any of them there.
        assert_eq!(Definitive.combine(Inferred), Inferred);
        assert_eq!(Inferred.combine(Definitive), Inferred);
        assert_eq!(Definitive.combine(Strong), Strong);
        assert_eq!(Strong.combine(Strong), Strong);
        // Manual is not weaker or stronger; it is asserted by the user and outranks inference.
        assert_eq!(Inferred.combine(Manual), Manual);
        assert_eq!(Manual.combine(Inferred), Manual);
    }
}
