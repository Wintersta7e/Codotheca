#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §46.7 through the production handlers: **why the recycle bin cannot take a copy is named
//! before the click**, and a copy it cannot take is never sent to it.

mod support;

use std::sync::Arc;

use codotheca_core::protocol::TrashRefusalKind;
use codotheca_core::removal::BinFacts;
use codotheca_core::testing::{CountingTrash, FixedBinSettings};
use support::analyser_world::Library;

/// The bin facts that produce each reason, for a copy of a few hundred bytes.
const fn facts_for(kind: TrashRefusalKind) -> BinFacts {
    match kind {
        TrashRefusalKind::Unsupported => BinFacts::NoTrash,
        TrashRefusalKind::NetworkDrive => BinFacts::Volume {
            network: true,
            nuke_on_delete: false,
            capacity_left_bytes: u64::MAX,
        },
        TrashRefusalKind::OversizedFolder => BinFacts::Volume {
            network: false,
            nuke_on_delete: false,
            capacity_left_bytes: 1,
        },
        TrashRefusalKind::DisabledOnVolume => BinFacts::Volume {
            network: false,
            nuke_on_delete: true,
            capacity_left_bytes: u64::MAX,
        },
        TrashRefusalKind::CapacityUnknown => BinFacts::Unreadable,
    }
}

/// **AC-P4-46-13.** Through `locations.uninstallPreflight`'s production handler, each of the
/// schema's reasons is carried on the verdict, `trashAvailable` is its absence from the same
/// reading, and through the act the trash is sent **nothing**. A bin with no limits carries no
/// reason and the act goes through.
#[test]
fn ac_p4_46_13() {
    let mut produced = Vec::new();
    for kind in TrashRefusalKind::ALL {
        let lib = Library::new();
        let copy = lib.pushed_repo("widget");
        let id = lib.register(&copy);
        let trash = CountingTrash::with_bins(Arc::new(FixedBinSettings(facts_for(kind))));
        let verdict = lib.preflight_with(id, &lib.verifier(), &trash);
        let refusal = verdict.0["trashRefusal"].clone();
        let available = verdict.0["trashAvailable"]
            .as_bool()
            .expect("trashAvailable");
        let act = lib.uninstall(id, &lib.verifier(), &trash);
        eprintln!(
            "trash refusal {kind:?}: trashRefusal {refusal}, trashAvailable {available}; the \
             act {act:?}; {} send(s)",
            trash.sends()
        );
        assert_eq!(
            refusal,
            serde_json::to_value(kind).expect("kind"),
            "{kind:?} was not carried"
        );
        assert_eq!(
            available,
            refusal.is_null(),
            "{kind:?}: one reading, two fields"
        );
        assert!(act.is_err(), "{kind:?}: the act went through");
        assert!(copy.exists());
        assert_eq!(
            trash.sends(),
            0,
            "{kind:?}: the bin was sent a copy it cannot take"
        );
        produced.push(kind);
    }

    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    let id = lib.register(&copy);
    let trash = CountingTrash::with_bins(Arc::new(FixedBinSettings(BinFacts::NoLimits)));
    let verdict = lib.preflight_with(id, &lib.verifier(), &trash);
    let act = lib.uninstall(id, &lib.verifier(), &trash);
    eprintln!(
        "no limits: trashRefusal {}, trashAvailable {}; the act {}; {} send(s); {} of {} \
         reasons produced",
        verdict.0["trashRefusal"],
        verdict.0["trashAvailable"],
        if act.is_ok() {
            "went through"
        } else {
            "refused"
        },
        trash.sends(),
        produced.len(),
        TrashRefusalKind::ALL.len()
    );
    assert!(verdict.0["trashRefusal"].is_null());
    assert_eq!(verdict.0["trashAvailable"], true);
    assert!(act.is_ok(), "{act:?}");
    assert_eq!(trash.sends(), 1);
    assert!(!produced.is_empty(), "no reason was produced");
    assert_eq!(produced.len(), TrashRefusalKind::ALL.len());
}

/// Windows-native only: the system's own bin settings answer a value or `Unreadable` — never a
/// panic and never a default. It prints what it read; §46.7's probe is the measured record.
#[cfg(windows)]
#[test]
fn the_system_bin_settings_read_returns_a_value_or_unknown() {
    use codotheca_core::removal::{BinSettings as _, SystemBinSettings};

    let dir = tempfile::tempdir().expect("tempdir");
    let facts = SystemBinSettings.for_path(dir.path());
    eprintln!(
        "system bin settings for {}: {facts:?}",
        dir.path().display()
    );
    assert!(matches!(
        facts,
        BinFacts::Volume { .. } | BinFacts::Unreadable | BinFacts::NoLimits | BinFacts::NoTrash
    ));
}
