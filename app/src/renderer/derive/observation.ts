/**
 * §6 — the phrasing of a timestamped observation.
 *
 * Worktree state is never cacheable, so every value derived from it is an observation with a
 * time, and the shelf never asserts currency it does not have. There is no phrasing here for
 * "clean": absence of dirty means "no changes as of T", and the words `clean`, `verified`,
 * `none` and `all` appear in no string this module produces.
 */

/**
 * §6 says "any surfaced value older than a threshold renders with its age" and names no number.
 * This is that number. It is a rendering decision and lives only here: the core uses no
 * threshold — visible tiles re-observe on scroll-idle and on focus regardless.
 */
export const WORKTREE_STALE_AFTER_SECS = 900;

/** §5.6: past 24 hours the dirty clause is not produced at all. */
export const ROAST_DIRTY_MAX_AGE_SECS = 86_400;

/**
 * [p2] §25.1 defers the remote staleness threshold to §21, and **§21 sets no number**. This is
 * that number, and it is derived rather than invented: six hours is §21.5's own `account_repos`
 * cadence, so a remote observation older than it has missed at least one scheduled read.
 *
 * It lives here beside `WORKTREE_STALE_AFTER_SECS` because it is the same kind of decision — a
 * rendering threshold with one owner. The core uses no threshold at all.
 */
export const REMOTE_STALE_AFTER_SECS = 21_600;

export function formatClock(epochSecs: number): string {
  return new Date(epochSecs * 1000).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  });
}

/**
 * Coarse on purpose: a false precision is a claim. A negative age — clock skew across a network
 * share is ordinary — floors to zero rather than rendering "-2h".
 */
export function formatAge(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86_400) return `${Math.floor(s / 3600)}h`;
  if (s < 365 * 86_400) return `${Math.floor(s / 86_400)}d`;
  return `${Math.floor(s / (365 * 86_400))}y`;
}

/** `as of 14:02`, or `as of 14:02 — observed 3h ago` once it is stale. */
export function asOfClause(
  observedAtSecs: number,
  nowSecs: number,
  staleAfterSecs: number = WORKTREE_STALE_AFTER_SECS,
): string {
  const base = `as of ${formatClock(observedAtSecs)}`;
  const age = nowSecs - observedAtSecs;
  return age >= staleAfterSecs ? `${base} — observed ${formatAge(age)} ago` : base;
}

/** §8.4's line. Returns null when nothing has been observed — absence, never a verdict. */
export function noChangesLine(observedAtSecs: number | null, nowSecs: number): string | null {
  if (observedAtSecs === null) return null;
  return `no changes ${asOfClause(observedAtSecs, nowSecs)}`;
}
