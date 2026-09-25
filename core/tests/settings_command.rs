#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §11.3's settings, read and patched through `app_meta`.

use codotheca_core::index::Index;
use codotheca_core::protocol::{EffectsTier, LogLevel, RootId, SettingsPatch};
use codotheca_core::surfaces::settings;

const fn empty_patch() -> SettingsPatch {
    SettingsPatch {
        effects_tier: None,
        reduced_motion_override: None,
        autostart: None,
        resident_shortcut: None,
        roast_enabled: None,
        log_level: None,
        install_root_id: None,
        content_scan_enabled: None,
        health_checks: None,
    }
}

#[test]
fn a_fresh_index_answers_with_the_stated_defaults() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    let s = settings::read(index.conn()).expect("read");
    assert_eq!(s.effects_tier, EffectsTier::Auto);
    assert!(!s.reduced_motion_override);
    assert!(!s.autostart, "§11.3: the residency ask defaults off");
    assert_eq!(
        s.resident_shortcut, None,
        "§8.6: the resident shortcut ships unbound"
    );
    assert!(s.roast_enabled, "§11.3: SET_DEFAULT roasts: 1");
    assert_eq!(s.log_level, LogLevel::Info);
}

#[test]
fn a_null_field_leaves_its_setting_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    settings::write(
        index.conn(),
        &SettingsPatch {
            roast_enabled: Some(false),
            ..empty_patch()
        },
        NOW,
    )
    .expect("first write");
    let after = settings::write(
        index.conn(),
        &SettingsPatch {
            autostart: Some(true),
            ..empty_patch()
        },
        NOW,
    )
    .expect("second write");
    assert!(
        !after.roast_enabled,
        "an untouched field survives the next patch"
    );
    assert!(after.autostart);
}

#[test]
fn clearing_the_resident_shortcut_is_expressible_and_is_not_the_same_as_leaving_it_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    settings::write(
        index.conn(),
        &SettingsPatch {
            resident_shortcut: Some("Control+Alt+K".into()),
            ..empty_patch()
        },
        NOW,
    )
    .expect("bind");
    let untouched = settings::write(
        index.conn(),
        &SettingsPatch {
            autostart: Some(true),
            ..empty_patch()
        },
        NOW,
    )
    .expect("leave it alone");
    assert_eq!(
        untouched.resident_shortcut.as_deref(),
        Some("Control+Alt+K"),
        "None is `leave it alone`, so the binding survives an unrelated patch"
    );
    let cleared = settings::write(
        index.conn(),
        &SettingsPatch {
            resident_shortcut: Some(String::new()),
            ..empty_patch()
        },
        NOW,
    )
    .expect("clear");
    assert_eq!(
        cleared.resident_shortcut, None,
        "the empty string is the clear"
    );
}

#[test]
fn an_unreadable_stored_value_falls_back_to_the_default_rather_than_failing_the_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    index
        .conn()
        .execute(
            "INSERT OR REPLACE INTO app_meta (k, v) VALUES ('effects_tier', 'holographic')",
            [],
        )
        .expect("write junk");
    assert_eq!(
        settings::read(index.conn()).expect("read").effects_tier,
        EffectsTier::Auto
    );
}

#[test]
fn a_chord_with_no_modifier_is_refused() {
    assert!(settings::validate_shortcut("Control+Alt+K"));
    assert!(settings::validate_shortcut("Super+Shift+F12"));
    assert!(
        !settings::validate_shortcut("K"),
        "a bare key would swallow every K on the machine"
    );
    assert!(
        !settings::validate_shortcut("Control+"),
        "a modifier alone binds nothing"
    );
    assert!(!settings::validate_shortcut(""));
    assert!(
        !settings::validate_shortcut("Control+Alt"),
        "two modifiers and no key is still no key"
    );
}

#[test]
fn a_refused_chord_is_a_protocol_failure_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    let ctx = codotheca_core::surfaces::SurfaceCtx {
        index: &index,
        now: 900,
    };
    let failure = settings::handle_set(
        &ctx,
        serde_json::json!({ "patch": { "residentShortcut": "K" } }),
    )
    .expect_err("a bare key is refused");
    assert_eq!(failure.code, codotheca_core::protocol::ErrorCode::Protocol);
    assert_eq!(
        failure.outcome, None,
        "a refused chord definitely did not take effect"
    );
    assert_eq!(
        settings::read(index.conn())
            .expect("read")
            .resident_shortcut,
        None,
        "the refusal is checked before anything is written"
    );
}

#[test]
fn every_setting_round_trips_through_the_stored_form() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    let written = settings::write(
        index.conn(),
        &SettingsPatch {
            effects_tier: Some(EffectsTier::Reduced),
            reduced_motion_override: Some(true),
            autostart: Some(true),
            resident_shortcut: Some("Control+Alt+K".into()),
            roast_enabled: Some(false),
            log_level: Some(LogLevel::Debug),
            install_root_id: Some(RootId(7)),
            content_scan_enabled: Some(true),
            health_checks: None,
        },
        NOW,
    )
    .expect("write");
    assert_eq!(written, settings::read(index.conn()).expect("read"));
    assert_eq!(written.effects_tier, EffectsTier::Reduced);
    assert_eq!(written.log_level, LogLevel::Debug);
    assert!(written.reduced_motion_override);
    assert!(!written.roast_enabled);
    // §24.3a's install root round-trips like every other key. It is set here rather than left
    // `None` because this is the one test that writes every field: a field excluded from it is a
    // field nothing round-trips.
    assert_eq!(written.install_root_id, Some(RootId(7)));
}

// ---------------------------------------------------------------------------
// Gap C: `settings.set` stamps the end of first run when its patch carries
// `autostart`. Three things must be true together and each is in a different
// plan — the residency card sends the patch, this stamps on it, and first run
// exposes the stamp. Any one missing and the `NEW` chip is silently dead: it
// does not fail, it never appears.
// ---------------------------------------------------------------------------

const NOW: i64 = 1_770_000_000;

#[test]
fn an_autostart_patch_stamps_the_end_of_first_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    assert_eq!(
        codotheca_core::firstrun::first_run_completed_at(index.conn()).expect("read"),
        None,
        "nothing has answered the residency ask yet"
    );

    settings::write(
        index.conn(),
        &SettingsPatch {
            autostart: Some(true),
            ..empty_patch()
        },
        NOW,
    )
    .expect("write");

    assert_eq!(
        codotheca_core::firstrun::first_run_completed_at(index.conn()).expect("read"),
        Some(NOW),
        "without this, isNewArrival is false for every project forever"
    );
}

/// **`false` is an answer too.** Declining autostart still ends first run — the ask was
/// answered, which is the event §11.3a stamps, not the value chosen.
#[test]
fn declining_autostart_also_ends_first_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    settings::write(
        index.conn(),
        &SettingsPatch {
            autostart: Some(false),
            ..empty_patch()
        },
        NOW,
    )
    .expect("write");
    assert_eq!(
        codotheca_core::firstrun::first_run_completed_at(index.conn()).expect("read"),
        Some(NOW)
    );
}

/// A patch that does not carry `autostart` must not stamp — the turn's button is not the
/// residency answer, and stamping there would begin the second-launch path with the question
/// still unasked, leaving a row that can never reappear.
#[test]
fn a_patch_without_autostart_does_not_stamp() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    settings::write(
        index.conn(),
        &SettingsPatch {
            roast_enabled: Some(false),
            ..empty_patch()
        },
        NOW,
    )
    .expect("write");
    assert_eq!(
        codotheca_core::firstrun::first_run_completed_at(index.conn()).expect("read"),
        None
    );
}

/// The stamp is the *first* answer's time and never moves afterwards.
#[test]
fn a_later_autostart_change_does_not_move_the_stamp() {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    settings::write(
        index.conn(),
        &SettingsPatch {
            autostart: Some(true),
            ..empty_patch()
        },
        NOW,
    )
    .expect("first");
    settings::write(
        index.conn(),
        &SettingsPatch {
            autostart: Some(false),
            ..empty_patch()
        },
        NOW + 10_000,
    )
    .expect("second");
    assert_eq!(
        codotheca_core::firstrun::first_run_completed_at(index.conn()).expect("read"),
        Some(NOW),
        "first run ended once"
    );
}
