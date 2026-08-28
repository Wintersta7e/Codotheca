import { describe, expect, it } from 'vitest';
import {
  asOfClause,
  formatAge,
  formatClock,
  noChangesLine,
  WORKTREE_STALE_AFTER_SECS,
} from './observation';

// A fixed instant with a known local wall clock, so the assertions do not depend on the
// machine's zone: the test computes the expected string the same way the renderer does.
const AT = 1_700_000_000;
const clockOf = (t: number): string =>
  new Date(t * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });

describe('the observation clause', () => {
  it('states a time and never a verdict when it is fresh', () => {
    // §6: absence of dirty means "no changes as of T", never "verified clean".
    expect(asOfClause(AT, AT + 60)).toBe(`as of ${clockOf(AT)}`);
  });

  it('renders its own age past the staleness threshold', () => {
    expect(asOfClause(AT, AT + 3 * 3600)).toBe(`as of ${clockOf(AT)} — observed 3h ago`);
    expect(WORKTREE_STALE_AFTER_SECS).toBe(900);
  });

  it('crosses the threshold exactly once, at the threshold', () => {
    const at = AT;
    expect(asOfClause(at, at + WORKTREE_STALE_AFTER_SECS - 1)).not.toContain('observed');
    expect(asOfClause(at, at + WORKTREE_STALE_AFTER_SECS)).toContain('observed');
  });

  it('never says clean, verified, none or all', () => {
    const banned = /\b(clean|verified|none|all)\b/i;
    expect(banned.test(noChangesLine(AT, AT + 60) ?? '')).toBe(false);
    expect(noChangesLine(AT, AT + 60)).toBe(`no changes as of ${clockOf(AT)}`);
  });

  it('renders nothing at all when nothing was observed', () => {
    // An unobserved worktree is not a clean one, and there is no phrasing for it.
    expect(noChangesLine(null, AT)).toBeNull();
  });

  it('formats ages coarsely, because a false precision is a claim', () => {
    expect(formatAge(30)).toBe('just now');
    expect(formatAge(12 * 60)).toBe('12m');
    expect(formatAge(3 * 3600)).toBe('3h');
    expect(formatAge(6 * 86_400)).toBe('6d');
    expect(formatAge(400 * 86_400)).toBe('1y');
  });

  it('formats a 24-hour clock with a leading zero', () => {
    expect(formatClock(AT)).toMatch(/^\d{2}:\d{2}$/);
  });

  it('treats a clock from the future as an age of zero rather than a negative one', () => {
    // Clock skew across a network share is ordinary; "observed -2h ago" is not a sentence.
    expect(formatAge(-7200)).toBe('just now');
    expect(asOfClause(AT, AT - 7200)).not.toContain('observed');
  });
});
