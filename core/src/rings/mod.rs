//! §39: the three rings — Focus, Ship and Tend — and the weekly rhythm.
//!
//! This module owns §39.12's constants; the renderer never restates one and reads its targets
//! from `RingsState`. `targets` holds the one reader and the one writer of each stored target.

pub mod targets;

/// [p4] §39.12 — the constants §39 owns. 45, 3, Monday weeks and "no daily Ship" are the
/// defaults the user may overturn.
pub const FOCUS_TARGET_DEFAULT_MINUTES: u32 = 45;
/// §39.3's floor: one touch must never close the ring (the assert lives in `session/mod.rs`).
pub const FOCUS_TARGET_MIN_MINUTES: u32 = 30;
/// The shortest local day, so every allowed target is closable on every day.
pub const FOCUS_TARGET_MAX_MINUTES: u32 = 1_380;
/// How many settled weeks the first settlement reads back when it derives the rhythm target.
pub const RHYTHM_WINDOW_WEEKS: u32 = 52;
/// The fewest active weeks in that window from which a target is derived rather than defaulted.
pub const RHYTHM_MIN_ACTIVE_WEEKS: usize = 4;
/// The rhythm target when too few weeks are active to derive one; unmeasured.
pub const RHYTHM_FALLBACK_TARGET: u32 = 3;
