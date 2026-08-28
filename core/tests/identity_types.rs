#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! The two cross-plan types the scanner consumes and must not duplicate: plan 09's
//! `LocationKind` (R21) and plan 08's `LocationInput` (R27). Landed here as declarations only —
//! the production `location` writer is plan 08's and is deliberately absent (R1).
//!
//! Both are stored vocabularies with a DDL CHECK behind them, which is R26's shape: a value the
//! core emits and the column rejects fails at runtime, not in review. These tests read the
//! migration.

use codotheca_core::derive::LocationKind;
use codotheca_core::identity::store::LocationInput;
use codotheca_core::index::path::{PathPlatform, StoredPath};
use codotheca_core::protocol::Presence;
use codotheca_core::scan::discover::RepoKind;

const LOCATIONS_DDL: &str = include_str!("../migrations/0002_locations_and_roots.sql");

#[test]
fn every_location_kind_round_trips_through_text_and_through_serde() {
    for kind in LocationKind::ALL {
        assert_eq!(LocationKind::parse(kind.as_str()), Some(kind));
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(
            json,
            format!("\"{}\"", kind.as_str()),
            "the wire spelling and the column spelling are one value"
        );
        assert_eq!(serde_json::from_str::<LocationKind>(&json).unwrap(), kind);
    }
    assert_eq!(LocationKind::parse("darwin"), None);
}

#[test]
fn every_location_kind_is_a_value_the_check_constraint_accepts() {
    for kind in LocationKind::ALL {
        assert!(
            LOCATIONS_DDL.contains(&format!("'{}'", kind.as_str())),
            "location.kind '{}' is emitted by the core and rejected by the DDL",
            kind.as_str()
        );
    }
}

/// R27's shape, field by field. Each of the five corrections it made to R1 is a column that would
/// otherwise be written wrong, so the struct is asserted rather than assumed.
#[test]
fn a_location_input_carries_presence_and_a_nullable_volume_key_explicitly() {
    let input = LocationInput {
        kind: LocationKind::Linux,
        distro: None,
        path: StoredPath::from_bytes(b"/r/a".to_vec(), PathPlatform::Unix),
        store_key: "s1".to_owned(),
        // A bind mount, overlayfs or tmpfs has no stable identifier. `None` is not `""`.
        volume_key: None,
        // The column has no default, so the writer would otherwise hard-code 'present' — which
        // is exactly what plan 04's test helper does and what R27 exists to stop.
        presence: Presence::Present,
        repo_kind: RepoKind::WorkTree,
        // Raw bytes, not a folded key: `path_bytes` and `path_key` are separate columns because
        // bytes-versus-key is a semantic distinction, not a rename.
        common_dir_bytes: None,
        generation: 7,
        last_seen_at: None,
    };
    assert_eq!(input.volume_key, None);
    assert_eq!(input.presence, Presence::Present);
    assert_eq!(input.path.key(), b"/r/a");
    assert_eq!(input.generation, 7);
}

/// §1.3: `distro` is NOT NULL and `''` when the location is not WSL, and the CHECK enforces it.
/// The struct models absence as `None`, so the writer converts — and this pins that the two
/// spellings of "not a WSL location" are known to be different.
#[test]
fn the_ddl_requires_an_empty_distro_for_every_non_wsl_kind() {
    assert!(
        LOCATIONS_DDL.contains("CHECK (kind = 'wsl' OR distro = '')"),
        "LocationInput.distro is Option<String>; whoever writes the row maps None to ''"
    );
}
