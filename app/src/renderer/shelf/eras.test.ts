import { describe, expect, it } from 'vitest';
import type { LocationRef, ProjectRow } from '../../generated/protocol.js';
import { formatTrackedBytes } from '../format/size.js';
import type { ShelfRow } from './row.js';
import { toShelfRow } from './row.js';
import {
  aggregateSection,
  ERA_COLLAPSE_MIN_ORDER,
  eraSectionIdFor,
  eraSectionLabel,
  eraSectionOrder,
  flagLineText,
  formatTrackedBytes as erasFormatTrackedBytes,
  summaryText,
  summaryTextTruncated,
} from './eras.js';

const DAY = 86_400;
/** 2026-06-15T12:00:00Z, so "this year" is 2026 and the ten named years run 2025 → 2016. */
const NOW = Math.floor(Date.UTC(2026, 5, 15, 12) / 1000);

/** §23.1's shape: the same row with no working copy — the pair, never the location alone. */
function notCloned(daysAgo: number, over: Record<string, unknown> = {}): ShelfRow {
  return at(daysAgo, { primaryLocation: null, presence: null, ...over });
}

function at(daysAgo: number, over: Record<string, unknown> = {}): ShelfRow {
  return toShelfRow({
    id: 1,
    name: 'p',
    owner: null,
    description: null,
    descriptionSource: null,
    birthYear: null,
    primaryLanguage: null,
    archetype: null,
    artSceneHash: null,
    artState: 'pending',
    conditionSignal: null,
    completionLit: null,
    completionApplicable: null,
    isPinned: false,
    isArchived: false,
    isHidden: false,
    isReference: false,
    isFork: false,
    isBare: false,
    isShallow: false,
    isSubmodule: false,
    ambiguousLineage: false,
    lastTouchedAt: NOW - daysAgo * DAY,
    lastInteractionAt: null,
    lastCommitAt: null,
    lastCommitSubject: null,
    firstCommitAt: null,
    createdAt: 0,
    acknowledgedAt: null,
    sizeTrackedBytes: null,
    trackedFiles: null,
    collectionIds: [],
    // §23: the location and the presence are one pair. A null location with 'present'
    // beside it is a working copy that is here, for a row that has no copy at all — and
    // after §23.4's classifier every such fixture lands in era:notcloned.
    primaryLocation: { id: 10 as LocationRef['id'], pathDisplay: '/w/row' },
    presence: 'present',
    branch: null,
    isDirty: null,
    untrackedCount: null,
    ahead: null,
    behind: null,
    stashCount: null,
    interruptedOp: null,
    fetchHeadAt: null,
    refstateObservedAt: null,
    worktreeObservedAt: null,
    errorKind: null,
    errorAt: null,
    eraSectionId: '',
    ...over,
  } as unknown as ProjectRow);
}

describe('eraSectionIdFor', () => {
  it('cuts the four recency bands', () => {
    expect(eraSectionIdFor(at(1), NOW)).toBe('era:live');
    expect(eraSectionIdFor(at(20), NOW)).toBe('era:month');
    expect(eraSectionIdFor(at(60), NOW)).toBe('era:q');
    expect(eraSectionIdFor(at(150), NOW)).toBe('era:year');
  });
  it('files a project touched last calendar year under that year, not EARLIER THIS YEAR', () => {
    // The design's `2026 - floor(touch/365)` files an October-2025 touch under EARLIER THIS YEAR
    // in January: two of its own labels claim one project.
    const octoberLast = Math.floor(Date.UTC(2025, 9, 1) / 1000);
    expect(eraSectionIdFor(at(0, { lastTouchedAt: octoberLast }), NOW)).toBe('era:2025');
  });
  it('files anything older than ten named years under the tail', () => {
    const old = Math.floor(Date.UTC(2014, 2, 1) / 1000);
    expect(eraSectionIdFor(at(0, { lastTouchedAt: old }), NOW)).toBe('era:tail');
  });
  it('archived and submodules are overrides, and archived wins', () => {
    expect(eraSectionIdFor(at(1, { isArchived: true }), NOW)).toBe('era:archived');
    expect(eraSectionIdFor(at(1, { isSubmodule: true }), NOW)).toBe('era:submodules');
    expect(eraSectionIdFor(at(1, { isArchived: true, isSubmodule: true }), NOW)).toBe(
      'era:archived',
    );
  });
  // AC-P2-23-9, replacing `never emits era:notcloned in phase 1`. The old bar iterated day
  // offsets over rows that were **all zero-location**, so after §23.4 it would have kept passing
  // while proving nothing. The replacement builds the shape both ways and prints the number of
  // zero-location fixtures it scanned; a passing run that scanned none of them fails.
  it('emits era:notcloned for a zero-location row and for nothing else', () => {
    const offsets = [0, 5, 40, 100, 400, 4000];
    let zeroLocation = 0;
    let located = 0;
    for (const d of offsets) {
      expect(eraSectionIdFor(notCloned(d), NOW)).toBe('era:notcloned');
      zeroLocation += 1;
      expect(eraSectionIdFor(at(d), NOW)).not.toBe('era:notcloned');
      located += 1;
    }
    // A floor, not `toBe(offsets.length)`: with an empty list that comparison is 0 === 0 and
    // the whole case passes having scanned nothing — the exact shape this test replaces.
    expect(
      zeroLocation,
      'a run that scanned no zero-location fixture proves nothing',
    ).toBeGreaterThan(0);
    expect(zeroLocation).toBe(offsets.length);
    expect(located).toBe(offsets.length);
  });

  // AC-P2-23-4's ordering half: the location is tested **first**, before isArchived and before
  // isSubmodule. `isArchived` is a user flag and a user may archive a not-cloned project;
  // `era:archived` is an interleaved section whose header sums tracked bytes, and a tile with no
  // bytes and no Play does not belong among tiles that have both.
  it('classifies a not-cloned project before archived and before submodules', () => {
    expect(eraSectionIdFor(notCloned(1, { isArchived: true }), NOW)).toBe('era:notcloned');
    expect(eraSectionIdFor(notCloned(1, { isSubmodule: true }), NOW)).toBe('era:notcloned');
    expect(eraSectionIdFor(notCloned(1, { isArchived: true, isSubmodule: true }), NOW)).toBe(
      'era:notcloned',
    );
  });
});

describe('eraSectionOrder', () => {
  it('orders the recency bands, then years newest first, then the three tails', () => {
    expect(
      ['era:live', 'era:month', 'era:q', 'era:year'].map((id) => eraSectionOrder(id, 2026)),
    ).toEqual([0, 1, 2, 3]);
    expect(eraSectionOrder('era:2025', 2026)).toBe(11);
    expect(eraSectionOrder('era:2016', 2026)).toBe(20);
    expect(
      ['era:tail', 'era:archived', 'era:submodules', 'era:notcloned'].map((id) =>
        eraSectionOrder(id, 2026),
      ),
    ).toEqual([90, 92, 94, 98]);
  });
  it('keeps every named year strictly below the tail, so no year can sort past it', () => {
    // The oldest named year is `cut - 10`, whose order is 20; the tail is 90. A rolled cut year
    // must never close that gap, or a year section renders after `<Y-11> AND EARLIER`.
    const oldest = eraSectionOrder('era:2016', 2026);
    expect(oldest).toBeLessThan(eraSectionOrder('era:tail', 2026));
    // §8.1 auto-collapses from this order down, and every year section must be inside it.
    expect(oldest).toBeGreaterThanOrEqual(ERA_COLLAPSE_MIN_ORDER);
    expect(eraSectionOrder('era:2025', 2026)).toBeGreaterThanOrEqual(ERA_COLLAPSE_MIN_ORDER);
    expect(eraSectionOrder('era:year', 2026)).toBeLessThan(ERA_COLLAPSE_MIN_ORDER);
  });
});

describe('eraSectionLabel', () => {
  it('renders the shell-owned prose the core does not send', () => {
    expect(eraSectionLabel('era:live', 2026)).toBe('LIVE');
    expect(eraSectionLabel('era:year', 2026)).toBe('EARLIER THIS YEAR');
    expect(eraSectionLabel('era:2019', 2026)).toBe('2019');
    expect(eraSectionLabel('era:archived', 2026)).toBe('ARCHIVED');
    expect(eraSectionLabel('era:submodules', 2026)).toBe('SUBMODULES');
    // AC-P2-23-4's label half. §8.1's table left this one blank and §23.4 owns it. Asserted
    // through `eraSectionLabel`, never against the id string: with no FIXED_LABEL entry the
    // fallback prints the header as lowercase `notcloned`, which is what this catches.
    expect(eraSectionLabel('era:notcloned', 2026)).toBe('NOT CLONED');
  });
  it('rolls the tail label with the year and carries no year in the id', () => {
    expect(eraSectionLabel('era:tail', 2026)).toBe('2015 AND EARLIER');
    expect(eraSectionLabel('era:tail', 2027)).toBe('2016 AND EARLIER');
  });
});

describe('aggregateSection and its header text', () => {
  const rows = [
    at(1, { sizeTrackedBytes: 40 * 1024 ** 3, ahead: 2, refstateObservedAt: NOW }),
    at(1, { sizeTrackedBytes: 1024 ** 3, isDirty: true, refstateObservedAt: NOW }),
    at(1, { interruptedOp: 'rebase', refstateObservedAt: NOW }),
    at(1),
  ];

  it('counts coverage, not just the observed values', () => {
    const agg = aggregateSection(rows);
    expect(agg).toEqual({
      count: 4,
      trackedBytes: 41 * 1024 ** 3,
      indexedCount: 2,
      unpushed: 1,
      uncommitted: 1,
      interrupted: 1,
      unchecked: 1,
    });
  });
  it('carries coverage in the summary, since the pending repositories are the largest', () => {
    // `41 GB`, not `41.0 GB`: §8.1's "one decimal of GB" is one decimal of *precision*, which is
    // what `formatTrackedBytes` implements and what §8.1's own examples print.
    expect(summaryText(aggregateSection(rows))).toBe('4 projects · 41 GB tracked (of 2 indexed)');
  });
  it('drops the coverage parenthetical once every project is indexed', () => {
    const indexed = [at(1, { sizeTrackedBytes: 1024 ** 3, refstateObservedAt: NOW })];
    expect(summaryText(aggregateSection(indexed))).toBe('1 project · 1 GB tracked');
  });
  it('drops the byte aggregate and its parenthetical as one unit, never the count', () => {
    // Ellipsis cuts from the end: `41 GB tracked (of 198 ind…` reads as a complete figure.
    expect(summaryTextTruncated(aggregateSection(rows))).toBe('4 projects');
  });
  it('renders the flag line while anything is unchecked, even at all zeros', () => {
    const unchecked = [at(1), at(1)];
    expect(flagLineText(aggregateSection(unchecked))).toBe('2 unchecked');
  });
  it('omits the flag line only when everything was observed and there is nothing to report', () => {
    const clean = [at(1, { refstateObservedAt: NOW, ahead: 0, isDirty: false })];
    expect(flagLineText(aggregateSection(clean))).toBeNull();
  });
  it('names the three flags and appends unchecked last', () => {
    expect(flagLineText(aggregateSection(rows))).toBe(
      '1 unpushed · 1 uncommitted · 1 interrupted · 1 unchecked',
    );
  });
  it('never counts an unmeasured inventory as a zero-byte project', () => {
    // `indexedCount` is the coverage denominator; a NULL `size_tracked_bytes` is *not measured*,
    // so it neither adds bytes nor claims an indexed project.
    const agg = aggregateSection([at(1), at(1, { sizeTrackedBytes: 0 })]);
    expect(agg.indexedCount).toBe(1);
    expect(agg.trackedBytes).toBe(0);
  });
});

describe('formatTrackedBytes', () => {
  // R12: the value table — 41 GB, 0.2 GB, 100 MB, 0 MB — and the mutation test that proves the
  // divisor is 1024 and not 1000 live in `app/src/renderer/format/size.test.ts` (plan 12c). What
  // this file asserts is that `eras.ts` re-exports that one function instead of growing a second,
  // which is the drift R12 exists to prevent: one 8 MiB repository printed `8 MB` here and
  // `8.4 MB` on the project page.
  it('is the shared formatter, re-exported and not re-implemented', () => {
    expect(erasFormatTrackedBytes).toBe(formatTrackedBytes);
  });
});
