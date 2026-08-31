/**
 * §9's focus protocol, in the one file both the shell and the renderer import.
 *
 * The core decides whether a project's own view is focused, and it measures the age of the
 * last report with `monotonic_ms`. A report older than FOCUS_STALE_SECS stops extending a
 * segment — that is what makes a renderer that died holding focus harmless.
 */

/** Mirrors `core::session::FOCUS_STALE_SECS`. The test asserts the two agree. */
export const FOCUS_STALE_SECS = 120;

/**
 * Four heartbeats fit inside the staleness window, so three consecutive losses survive and a
 * genuinely dead renderer is believed dead within two minutes.
 */
export const FOCUS_HEARTBEAT_SECS = 30;

export const FOCUS_STALE_MS = FOCUS_STALE_SECS * 1000;
export const FOCUS_HEARTBEAT_MS = FOCUS_HEARTBEAT_SECS * 1000;
export const HEARTBEATS_BEFORE_STALE = FOCUS_STALE_SECS / FOCUS_HEARTBEAT_SECS;

/** `session.focus`'s entire argument. `null` means no project's view is focused. */
export interface FocusReport {
  readonly projectId: number | null;
}

export function focusArgs(projectId: number | null): FocusReport {
  return { projectId };
}
