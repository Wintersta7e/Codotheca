#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §39.12's two targets, through the real `settings.set` handler over a migrated index.

use codotheca_core::index::Index;
use codotheca_core::protocol::{ErrorCode, RhythmTargetBasis};
use codotheca_core::rings::targets::{self, KEY_RHYTHM_TARGET, KEY_RHYTHM_TARGET_BASIS};
use codotheca_core::rings::{FOCUS_TARGET_DEFAULT_MINUTES, FOCUS_TARGET_MIN_MINUTES};
use codotheca_core::session::ONE_TOUCH_CREDIT_BOUND_SECS;
use codotheca_core::surfaces::{settings, SurfaceCtx};

const NOW: i64 = 1_800_000_000;

fn open() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    (dir, index)
}

fn set(
    index: &Index,
    patch: &serde_json::Value,
) -> Result<codotheca_core::protocol::Settings, codotheca_core::proto::dispatch::CommandFailure> {
    let ctx = SurfaceCtx { index, now: NOW };
    settings::handle_set(&ctx, serde_json::json!({ "patch": patch }))
}

fn meta_rows(index: &Index) -> i64 {
    index
        .conn()
        .query_row("SELECT COUNT(*) FROM app_meta", [], |r| r.get(0))
        .unwrap()
}

fn meta(index: &Index, key: &str) -> Option<String> {
    index
        .conn()
        .query_row("SELECT v FROM app_meta WHERE k = ?1", [key], |r| r.get(0))
        .ok()
}

fn put_raw(index: &Index, key: &str, value: &str) {
    index
        .conn()
        .execute(
            "INSERT INTO app_meta (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            [key, value],
        )
        .unwrap();
}

#[test]
fn focus_target_round_trips_and_absent_reads_45() {
    let (_dir, index) = open();
    let ctx = SurfaceCtx {
        index: &index,
        now: NOW,
    };
    let fresh = settings::handle_get(&ctx).unwrap();
    assert_eq!(fresh.focus_target_minutes, 45, "absent reads the default");
    assert_eq!(fresh.focus_target_minutes, FOCUS_TARGET_DEFAULT_MINUTES);
    assert_eq!(fresh.rhythm_target, None, "no target before any choice");

    let after = set(&index, &serde_json::json!({ "focusTargetMinutes": 90 })).unwrap();
    assert_eq!(after.focus_target_minutes, 90);
    assert_eq!(settings::handle_get(&ctx).unwrap().focus_target_minutes, 90);
    assert_eq!(
        after.rhythm_target, None,
        "a focus patch leaves the rhythm alone"
    );
}

#[test]
fn out_of_range_targets_are_refused_and_write_nothing() {
    let (_dir, index) = open();
    let before = meta_rows(&index);
    for (field, value) in [
        ("focusTargetMinutes", 29),
        ("focusTargetMinutes", 1381),
        ("rhythmTarget", 0),
        ("rhythmTarget", 8),
    ] {
        let answer = set(&index, &serde_json::json!({ field: value }));
        eprintln!("{field} = {value} -> {answer:?}");
        let failure = answer.expect_err("an out-of-range target is refused");
        assert_eq!(failure.code, ErrorCode::Protocol, "{field} = {value}");
        assert_eq!(
            failure.outcome, None,
            "a refusal definitely did not take effect"
        );
        assert!(
            failure.message.contains(field),
            "the message names the field: {}",
            failure.message
        );
        assert_eq!(
            meta_rows(&index),
            before,
            "a {value} {field} target was stored"
        );
    }
    let read = settings::read(index.conn()).unwrap();
    assert_eq!(
        read.focus_target_minutes, 45,
        "a 29-minute target was stored"
    );
    assert_eq!(read.rhythm_target, None);
}

#[test]
fn the_range_edges_are_accepted() {
    let (_dir, index) = open();
    for minutes in [30, 1380] {
        let s = set(
            &index,
            &serde_json::json!({ "focusTargetMinutes": minutes }),
        )
        .unwrap();
        assert_eq!(s.focus_target_minutes, minutes);
    }
    for target in [1, 7] {
        let s = set(&index, &serde_json::json!({ "rhythmTarget": target })).unwrap();
        assert_eq!(s.rhythm_target, Some(target));
    }
}

#[test]
fn choosing_a_rhythm_target_records_basis_chosen() {
    let (_dir, index) = open();
    let s = set(&index, &serde_json::json!({ "rhythmTarget": 5 })).unwrap();
    assert_eq!(s.rhythm_target, Some(5));
    assert_eq!(meta(&index, KEY_RHYTHM_TARGET).as_deref(), Some("5"));
    assert_eq!(
        meta(&index, KEY_RHYTHM_TARGET_BASIS).as_deref(),
        Some("chosen")
    );
    assert_eq!(
        targets::rhythm_target(index.conn()).unwrap(),
        Some((5, RhythmTargetBasis::Chosen))
    );
}

#[test]
fn a_stored_value_this_build_cannot_read_is_the_default() {
    let (_dir, index) = open();
    put_raw(&index, targets::KEY_FOCUS_TARGET_MINUTES, "abc");
    put_raw(&index, KEY_RHYTHM_TARGET, "9");
    put_raw(&index, KEY_RHYTHM_TARGET_BASIS, "chosen");
    assert_eq!(targets::focus_target_minutes(index.conn()).unwrap(), 45);
    assert_eq!(targets::rhythm_target(index.conn()).unwrap(), None);
    let s = settings::read(index.conn()).unwrap();
    assert_eq!(s.focus_target_minutes, 45);
    assert_eq!(s.rhythm_target, None);

    // A value without its basis, or a basis this build cannot read, is no target either.
    put_raw(&index, KEY_RHYTHM_TARGET, "4");
    put_raw(&index, KEY_RHYTHM_TARGET_BASIS, "guessed");
    assert_eq!(targets::rhythm_target(index.conn()).unwrap(), None);
    index
        .conn()
        .execute(
            "DELETE FROM app_meta WHERE k = ?1",
            [KEY_RHYTHM_TARGET_BASIS],
        )
        .unwrap();
    assert_eq!(targets::rhythm_target(index.conn()).unwrap(), None);
}

#[test]
fn a_direct_write_out_of_range_rolls_back() {
    let (_dir, index) = open();
    let before = meta_rows(&index);
    let patch = codotheca_core::protocol::SettingsPatch {
        effects_tier: None,
        reduced_motion_override: None,
        autostart: None,
        resident_shortcut: None,
        roast_enabled: Some(false),
        log_level: None,
        install_root_id: None,
        content_scan_enabled: None,
        health_checks: None,
        focus_target_minutes: Some(29),
        rhythm_target: None,
    };
    let err = settings::write(index.conn(), &patch, NOW).expect_err("29 minutes is refused");
    eprintln!("write(29) -> {err}");
    assert_eq!(
        meta_rows(&index),
        before,
        "nothing in the patch was written"
    );
}

#[test]
fn the_floor_exceeds_one_touch() {
    let floor = i64::from(FOCUS_TARGET_MIN_MINUTES) * 60;
    eprintln!("FOCUS_TARGET_MIN_MINUTES * 60 = {floor}; ONE_TOUCH_CREDIT_BOUND_SECS = {ONE_TOUCH_CREDIT_BOUND_SECS}");
    assert!(floor > ONE_TOUCH_CREDIT_BOUND_SECS);
}
