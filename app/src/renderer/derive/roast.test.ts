import { describe, expect, it } from 'vitest';
import { roastLine, type RoastInput, type ShownLocation } from './roast';
import { asOfClause } from './observation';

const NOW = 1_800_000_000;
const DAY = 86_400;

const shown = (over: Partial<ShownLocation> = {}): ShownLocation => ({
  locationId: 'L1',
  presence: 'present',
  interruptedOp: null,
  isDirty: false,
  removedAt: null,
  worktreeObservedAt: NOW - 60,
  ahead: 0,
  fetchHeadAt: null,
  stashCount: 0,
  headOid: 'aaa',
  lastCommitAt: NOW - DAY,
  ...over,
});

const input = (over: Partial<RoastInput> = {}): RoastInput => ({
  roastsEnabled: true,
  isReference: false,
  isArchived: false,
  neverSucceeded: false,
  shown: shown(),
  primary: { locationId: 'L1', headOid: 'aaa' },
  now: NOW,
  ...over,
});

describe('the producer as a whole', () => {
  it('produces nothing when nothing matches — silence, never a completion claim', () => {
    expect(roastLine(input())).toBeNull();
  });

  it.each([
    ['the switch is off', { roastsEnabled: false }],
    ['the project is Reference', { isReference: true }],
    ['the project is archived', { isArchived: true }],
    ['no job has ever succeeded', { neverSucceeded: true }],
  ])('produces nothing when %s', (_name, over) => {
    expect(roastLine(input({ ...over, shown: shown({ stashCount: 3 }) }))).toBeNull();
  });

  it('produces nothing when the shown location is not present', () => {
    // §4.6: absent is not abandoned.
    for (const presence of ['offline', 'missing', 'unscanned'] as const) {
      expect(roastLine(input({ shown: shown({ presence, stashCount: 3 }) }))).toBeNull();
    }
  });
});

describe('the five clauses, first match wins', () => {
  it('an open operation outranks everything else', () => {
    const line = roastLine(
      input({ shown: shown({ interruptedOp: 'rebase', isDirty: true, stashCount: 4 }) }),
    );
    expect(line).toBe('A rebase is open here and unfinished.');
  });

  it('uncommitted work outranks an unpushed branch and a stash', () => {
    const at = NOW - 60;
    const line = roastLine(
      input({
        shown: shown({
          isDirty: true,
          worktreeObservedAt: at,
          ahead: 7,
          fetchHeadAt: NOW - 6 * DAY,
          lastCommitAt: NOW - 40 * DAY,
          stashCount: 2,
        }),
      }),
    );
    expect(line).toBe(`Uncommitted work here ${asOfClause(at, NOW)}.`);
  });

  it('carries its own age past the staleness threshold', () => {
    const at = NOW - 3 * 3600;
    expect(roastLine(input({ shown: shown({ isDirty: true, worktreeObservedAt: at }) }))).toBe(
      'Uncommitted work here as of ' + asOfClause(at, NOW).replace('as of ', '') + '.',
    );
    expect(roastLine(input({ shown: shown({ isDirty: true, worktreeObservedAt: at }) }))).toContain(
      'observed 3h ago',
    );
  });

  it('is not produced at all past 24 hours, and falls through', () => {
    // A sentence has no room for a hedge wide enough to cover a day-old worktree reading.
    const line = roastLine(
      input({
        shown: shown({
          isDirty: true,
          worktreeObservedAt: NOW - 25 * 3600,
          ahead: 7,
          fetchHeadAt: NOW - 6 * DAY,
          lastCommitAt: NOW - 40 * DAY,
        }),
      }),
    );
    expect(line).toBe('7 commits ahead of its upstream, the newest 40d old. Last fetch 6d ago.');
  });

  it('never phrases an unobserved worktree as dirty or as anything else', () => {
    expect(
      roastLine(input({ shown: shown({ isDirty: null, worktreeObservedAt: null }) })),
    ).toBeNull();
    expect(
      roastLine(input({ shown: shown({ isDirty: true, worktreeObservedAt: null }) })),
    ).toBeNull();
  });

  it('will not state an ahead count with no fetch recorded', () => {
    // Criterion 63: with no FETCH_HEAD, `ahead` is a count against a ref of unknown age.
    const line = roastLine(
      input({ shown: shown({ ahead: 7, fetchHeadAt: null, lastCommitAt: NOW - 40 * DAY }) }),
    );
    expect(line).toBeNull();
  });

  it('will not state an ahead count for a branch committed to recently', () => {
    const line = roastLine(
      input({ shown: shown({ ahead: 7, fetchHeadAt: NOW - DAY, lastCommitAt: NOW - 5 * DAY }) }),
    );
    expect(line).toBeNull();
  });

  it('states stashes when nothing above matches', () => {
    expect(roastLine(input({ shown: shown({ stashCount: 3 }) }))).toBe(
      '3 stashes here. Nothing pushes a stash.',
    );
  });

  it('compares two copies by identity, never by direction', () => {
    const line = roastLine(
      input({
        shown: shown({ locationId: 'L2', headOid: 'bbb' }),
        primary: { locationId: 'L1', headOid: 'aaa' },
      }),
    );
    expect(line).toBe(
      'This is not the copy the app treats as primary. The two are at different commits.',
    );
  });

  it('says nothing when two copies sit on the same commit', () => {
    const line = roastLine(
      input({
        shown: shown({ locationId: 'L2', headOid: 'aaa' }),
        primary: { locationId: 'L1', headOid: 'aaa' },
      }),
    );
    expect(line).toBeNull();
  });

  it('says nothing when either head is not computed', () => {
    // A NULL head — bare, or an unborn HEAD — is "not computed", never a divergence.
    expect(
      roastLine(
        input({
          shown: shown({ locationId: 'L2', headOid: null }),
          primary: { locationId: 'L1', headOid: 'aaa' },
        }),
      ),
    ).toBeNull();
  });
});

describe('what no line may ever do', () => {
  it('mentions no absence and praises no volume', () => {
    const lines = [
      roastLine(input({ shown: shown({ interruptedOp: 'merge' }) })),
      roastLine(input({ shown: shown({ isDirty: true }) })),
      roastLine(
        input({ shown: shown({ ahead: 7, fetchHeadAt: NOW - DAY, lastCommitAt: NOW - 40 * DAY }) }),
      ),
      roastLine(input({ shown: shown({ stashCount: 3 }) })),
      roastLine(
        input({
          shown: shown({ locationId: 'L2', headOid: 'bbb' }),
          primary: { locationId: 'L1', headOid: 'aaa' },
        }),
      ),
    ];
    expect(lines.every((l) => l !== null)).toBe(true);
    for (const line of lines) {
      expect(line).not.toMatch(/untouched|years?\s+away|still builds|nothing outstanding/i);
      expect(line).not.toMatch(/\b(clean|none|all)\b/i);
      expect(line).not.toMatch(/\blines?\b/i);
    }
  });
});

/**
 * [p2] §24.6a. A copy uninstalled seconds ago still carries a `stashCount` and a `presence` of
 * `present`, because neither is re-observed until a scan runs. Roasting it for a stash that is no
 * longer on disk is a sentence about a directory that does not exist — and *roasting only inside
 * an opened project card* does not make it correct.
 */
describe('an uninstalled copy', () => {
  it('is never roasted, however loudly its stale numbers read', () => {
    const noisy = shown({
      presence: 'present',
      stashCount: 4,
      isDirty: true,
      ahead: 12,
      worktreeObservedAt: NOW - DAY,
    });
    // It roasts while it is installed…
    expect(roastLine(input({ shown: noisy }))).not.toBeNull();
    // …and says nothing once it is not.
    expect(roastLine(input({ shown: { ...noisy, removedAt: NOW - 60 } }))).toBeNull();
  });
});
