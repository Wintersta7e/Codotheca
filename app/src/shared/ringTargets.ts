/**
 * §39.12's two target ranges, as the core validates them. The drawer's steppers are bounded by
 * these, so no step it offers is one `settings.set` refuses.
 */

/** Mirrors `core::rings::FOCUS_TARGET_MIN_MINUTES`. The test asserts the two agree. */
export const FOCUS_TARGET_MIN_MINUTES = 30;
/** Mirrors `core::rings::FOCUS_TARGET_MAX_MINUTES`. The test asserts the two agree. */
export const FOCUS_TARGET_MAX_MINUTES = 1380;
/** Mirrors `core::rings::targets`' rhythm minimum: one active day a week. */
export const RHYTHM_TARGET_MIN_DAYS = 1;
/** Mirrors `core::rings::targets`' rhythm maximum: every day of the week. */
export const RHYTHM_TARGET_MAX_DAYS = 7;
