import { describe, expect, it } from 'vitest';
import type { LocationDetail, RootId } from '../../../generated/protocol';
import { formatTrackedBytes } from '../../format/size';
import { DAY, locationFixture, NOW } from '../testFixtures';
import {
  actionLabel,
  DIFFERENT_COMMIT_NOTE,
  footerText,
  headerNote,
  kindChip,
  locationActions,
  locationFacts,
  locationNote,
  type LocationFact,
  MISSING_NOTE,
  OFFLINE_NOTE,
  rowTag,
  stateWord,
} from './locationCopy';

const ROOT = 3 as RootId;

const facts = (over: Partial<LocationDetail>, size: number | null = null): LocationFact[] =>
  locationFacts({ location: locationFixture(over), sizeTrackedBytes: size, now: NOW });

describe('the state word', () => {
  it('is the presence word for every state that is not present', () => {
    expect(stateWord(locationFixture({ presence: 'offline' }))).toBe('OFFLINE');
    expect(stateWord(locationFixture({ presence: 'missing' }))).toBe('MISSING');
    expect(stateWord(locationFixture({ presence: 'unscanned' }))).toBe('NOT SCANNED');
  });

  it('compares a present copy by identity and never by direction', () => {
    expect(stateWord(locationFixture({ headComparison: 'same_commit' }))).toBe('SAME COMMIT');
    expect(stateWord(locationFixture({ headComparison: 'different_commit' }))).toBe(
      'DIFFERENT COMMIT',
    );
    expect(stateWord(locationFixture({ headComparison: 'not_compared' }))).toBe('NOT COMPARED');
  });

  it('carries no directional word anywhere — no DIVERGED, no BEHIND PRIMARY, no CURRENT', () => {
    for (const p of ['present', 'offline', 'missing', 'unscanned'] as const) {
      expect(stateWord(locationFixture({ presence: p }))).not.toMatch(
        /DIVERGED|BEHIND PRIMARY|CURRENT/,
      );
    }
  });
});

describe('the fact strip', () => {
  it('reads no upstream where ahead and behind are NULL, and never AHEAD 0', () => {
    const f = facts({ ahead: null, behind: null });
    expect(f.find((x) => x.key === 'UPSTREAM')?.value).toBe('no upstream');
    expect(f.map((x) => x.key)).not.toContain('AHEAD');
    expect(f.map((x) => x.key)).not.toContain('BEHIND');
  });

  it('reports nothing at all when a fetched copy is level', () => {
    const f = facts({ ahead: 0, behind: 0 });
    expect(f.map((x) => x.key)).not.toContain('AHEAD');
    expect(f.map((x) => x.key)).not.toContain('BEHIND');
    expect(f.map((x) => x.key)).not.toContain('UPSTREAM');
  });

  it('carries the age of the fetch on every count it prints', () => {
    const f = facts({ ahead: 2, behind: 0, fetchHeadAt: NOW - 6 * DAY });
    expect(f.find((x) => x.key === 'AHEAD')?.value).toBe('2 · last fetch 6d');
  });

  it('says no fetch recorded rather than an age it does not have', () => {
    const f = facts({ ahead: 2, fetchHeadAt: null });
    expect(f.find((x) => x.key === 'AHEAD')?.value).toBe('2 · no fetch recorded');
    expect(JSON.stringify(f)).not.toContain('refstate');
  });

  it('prints tracked size on the primary row only, and never on a copy', () => {
    const primary = facts({ isPrimary: true }, 8_400_000);
    expect(primary.find((x) => x.key === 'TRACKED')?.value).toBe(formatTrackedBytes(8_400_000));
    expect(facts({ isPrimary: false }, 8_400_000).map((x) => x.key)).not.toContain('TRACKED');
    expect(facts({ isPrimary: true }, null).map((x) => x.key)).not.toContain('TRACKED');
  });

  it('gives an offline row exactly last-seen and the branch, and no drive name', () => {
    const f = facts({ presence: 'offline', lastSeenAt: NOW - 3 * DAY, branch: 'main' });
    expect(f).toEqual([
      { key: 'LAST SEEN', value: '3d' },
      { key: 'BRANCH', value: 'main' },
    ]);
    expect(JSON.stringify(f)).not.toMatch(/volume|drive|disk|mount point/i);
  });

  it('gives a missing row last-seen alone', () => {
    expect(facts({ presence: 'missing', lastSeenAt: NOW - 3 * DAY }).map((x) => x.key)).toEqual([
      'LAST SEEN',
    ]);
  });

  it('states an unscanned row as covered by a disabled root, naming no root', () => {
    const f = facts({ presence: 'unscanned', coveringRootId: ROOT });
    expect(f).toEqual([{ key: 'ROOT', value: 'DISABLED' }]);
    expect(facts({ presence: 'unscanned', coveringRootId: null })).toEqual([]);
  });
});

describe('the actions', () => {
  it('offers open and reveal on a present copy', () => {
    expect(locationActions(locationFixture({ presence: 'present' }))).toEqual(['open', 'reveal']);
  });

  it('offers RELOCATE and nothing else on an unreachable copy', () => {
    expect(locationActions(locationFixture({ presence: 'offline' }))).toEqual(['relocate']);
    expect(locationActions(locationFixture({ presence: 'missing' }))).toEqual(['relocate']);
  });

  it('offers ENABLE ROOT only when a root id exists to enable', () => {
    expect(
      locationActions(locationFixture({ presence: 'unscanned', coveringRootId: ROOT })),
    ).toEqual(['enableRoot']);
    expect(
      locationActions(locationFixture({ presence: 'unscanned', coveringRootId: null })),
    ).toEqual([]);
  });

  it('knows no destructive verb', () => {
    const labels = (['open', 'reveal', 'relocate', 'enableRoot'] as const).map(actionLabel);
    expect(labels).toEqual(['OPEN', 'REVEAL', 'RELOCATE', 'ENABLE ROOT']);
    expect(labels.join(' ')).not.toMatch(/FORGET|REMOVE|DELETE|UNINSTALL/i);
  });
});

describe('the chip and the tag', () => {
  it('names the side a copy lives on, and the distro where that is the question', () => {
    expect(kindChip('win', '')).toBe('WIN');
    expect(kindChip('linux', '')).toBe('LINUX');
    expect(kindChip('wsl', 'distro-a')).toBe('WSL · distro-a');
    expect(kindChip('wsl', '')).toBe('WSL');
  });

  it('plates one row and tags the rest', () => {
    expect(rowTag(true)).toBe('PRIMARY');
    expect(rowTag(false)).toBe('COPY');
  });
});

describe('the header note', () => {
  it('states the single-copy case as a sentence, not a count', () => {
    expect(headerNote([locationFixture()])).toBe('ONE COPY ON THIS MACHINE');
  });

  it('claims all reachable only when every row is present', () => {
    expect(headerNote([locationFixture(), locationFixture()])).toBe('2 COPIES · ALL REACHABLE');
  });

  it('names the exception rather than claiming reachability it does not have', () => {
    expect(headerNote([locationFixture(), locationFixture({ presence: 'offline' })])).toBe(
      '2 COPIES · ONE OFFLINE',
    );
    expect(
      headerNote([
        locationFixture(),
        locationFixture({ presence: 'offline' }),
        locationFixture({ presence: 'missing' }),
      ]),
    ).toBe('3 COPIES · ONE OFFLINE · ONE MISSING');
  });
});

describe('the footer', () => {
  it('draws nothing on a single-copy project', () => {
    expect(footerText('strong', 1)).toBeNull();
  });

  it('names the evidence that actually made the association', () => {
    expect(footerText('definitive', 2)).toBe('SAME GIT DIRECTORY · LINKED WORKTREES');
    expect(footerText('strong', 2)).toBe('SAME LINEAGE · SAME REMOTE');
    expect(footerText('inferred', 2)).toBe('SAME LINEAGE · ONE COPY HAS NO REMOTE · INFERRED');
    expect(footerText('manual', 2)).toBe('MERGED BY YOU');
    expect(footerText(null, 2)).toBeNull();
  });

  it('never asserts the root-commit inference §1.1 forbids', () => {
    const all = (['definitive', 'strong', 'inferred', 'manual'] as const)
      .map((k) => footerText(k, 2))
      .join(' ');
    expect(all).not.toContain('SAME ROOT COMMIT');
    expect(all).not.toContain('ONE PROJECT · NOT');
  });
});

describe('the three notes', () => {
  it('says a frozen copy is frozen and not rotting', () => {
    expect(locationNote(locationFixture({ presence: 'offline' }))).toBe(OFFLINE_NOTE);
    expect(OFFLINE_NOTE).toBe(
      'The drive is not mounted. This copy is frozen, not rotting — its condition stops here rather than decaying.',
    );
  });

  it('says a missing folder was not removed by us', () => {
    expect(locationNote(locationFixture({ presence: 'missing' }))).toBe(MISSING_NOTE);
    expect(MISSING_NOTE).toBe(
      'The drive is mounted and the folder is not on it. It may have been moved or deleted; Codotheca removes nothing either way.',
    );
  });

  it('says what a different commit means without saying which way', () => {
    expect(locationNote(locationFixture({ headComparison: 'different_commit' }))).toBe(
      DIFFERENT_COMMIT_NOTE,
    );
    expect(DIFFERENT_COMMIT_NOTE).toBe(
      'This copy is on a different commit. Opening it is not the same as opening the project.',
    );
    expect(locationNote(locationFixture({ headComparison: 'not_compared' }))).toBeNull();
  });
});
