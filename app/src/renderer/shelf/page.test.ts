import { describe, expect, it } from 'vitest';
import type { LocationRef, ProjectRow } from '../../generated/protocol.js';
import type { ShelfRow } from './row.js';
import { toShelfRow } from './row.js';
import type { QueryContext } from './evaluate.js';
import { buildShelfPage, compareRows, orderKeyOf } from './page.js';

const DAY = 86_400;
const NOW = Math.floor(Date.UTC(2026, 5, 15, 12) / 1000);
function r(id: number, over: Record<string, unknown> = {}): ShelfRow {
  return toShelfRow({
    id,
    name: `p${String(id)}`,
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
    lastTouchedAt: NOW,
    lastInteractionAt: null,
    lastCommitAt: null,
    lastCommitSubject: null,
    firstCommitAt: null,
    createdAt: 0,
    acknowledgedAt: null,
    sizeTrackedBytes: null,
    trackedFiles: null,
    collectionIds: [],
    // §23: location and presence are one pair — a null location beside 'present'
    // describes a state the product cannot produce, and §23.4's classifier files
    // every such row under era:notcloned.
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
const ctx: QueryContext = {
  now: NOW,
  firstRunCompletedAt: null,
  collectionIdsByName: new Map(),
  pathsAreCaseSensitive: false,
  commitSubjectHits: null,
  capabilities: {
    authoredByUser: false,
    location: false,
    hasReadme: false,
    hasLicense: false,
    hasTests: false,
    hasCi: false,
    hasRemote: false,
    hasSubmodules: false,
  },
};
const build = (
  rows: ShelfRow[],
  query = '',
  sort: 'last_touched' | 'name' | 'size' = 'last_touched',
): ReturnType<typeof buildShelfPage> =>
  buildShelfPage({ rows, query, sort, now: NOW, generation: 3, ctx });

describe('buildShelfPage', () => {
  it('emits sections in order, and never an empty one', () => {
    const page = build([r(1), r(2, { lastTouchedAt: NOW - 400 * DAY })]);
    expect(page.sections.map((s) => s.id)).toEqual(['era:live', 'era:2025']);
  });
  it('gives every section a count and an aggregate before any row is read', () => {
    const page = build([r(1, { sizeTrackedBytes: 1024 ** 3 }), r(2)]);
    const [live] = page.sections;
    expect(live?.agg.count).toBe(2);
    expect(live?.agg.indexedCount).toBe(1);
  });
  it('recomputes the aggregates under a filter, not from the unfiltered set', () => {
    const rows = [r(1, { sizeTrackedBytes: 10, isDirty: true }), r(2, { sizeTrackedBytes: 90 })];
    const page = build(rows, 'is:dirty');
    expect(page.sections[0]?.agg.trackedBytes).toBe(10);
  });
  it('keeps Reference out of the sections and in its own uncapped list', () => {
    const rows = [r(1), ...Array.from({ length: 45 }, (_, i) => r(100 + i, { isReference: true }))];
    const page = build(rows);
    expect(page.sections.every((s) => s.rows.every((row) => !row.isReference))).toBe(true);
    expect(page.reference).toHaveLength(45);
  });
  it('counts renderedTotal without Reference, since the collapse threshold does', () => {
    const rows = [r(1), r(2, { isReference: true })];
    expect(build(rows).renderedTotal).toBe(1);
  });
  it('keeps Reference out of `matched`, which is the headline’s numerator', () => {
    // §8.0b prints `N OF <total> · <k> REFERENCE EXCLUDED`. Counting reference rows in `matched`
    // is the prototype's exact defect: excluded in words, included in the number.
    const rows = [r(1), r(2, { isReference: true })];
    expect(build(rows).matched).toBe(1);
  });
  it('narrows the Reference tail under a query, rather than ignoring the filter', () => {
    const rows = [
      r(1, { isReference: true, isDirty: true }),
      r(2, { isReference: true, isDirty: false }),
    ];
    expect(build(rows, 'is:dirty').reference.map((row) => row.id)).toEqual([1]);
  });
  it('never lists a hidden project in the tail, however reference it is', () => {
    // §8.0b's base predicate keeps hidden rows out unless the query asks; the tail opts reference
    // back in and nothing else.
    const rows = [r(1, { isReference: true, isHidden: true }), r(2, { isReference: true })];
    expect(build(rows).reference.map((row) => row.id)).toEqual([2]);
  });
  it('renders a reference row exactly once when the query asks for reference rows', () => {
    const rows = [r(1, { isReference: true })];
    const page = build(rows, 'is:reference');
    expect(page.reference.map((row) => row.id)).toEqual([1]);
    expect(page.sections).toHaveLength(0);
  });
  it('sorts the tail by the same key as the grid', () => {
    const rows = [
      r(1, { isReference: true, name: 'zulu' }),
      r(2, { isReference: true, name: 'alpha' }),
    ];
    expect(build(rows, '', 'name').reference.map((row) => row.name)).toEqual(['alpha', 'zulu']);
  });
  it('carries the ignored terms so the empty state can name them', () => {
    expect(build([r(1)], 'is:sideways').ignored).toEqual([
      { text: 'is:sideways', reason: 'malformedValue' },
    ]);
  });
  it('reports the query it was asked, not the one it ran the tail with', () => {
    // The tail opts reference in through the base predicate's own clause; that synthetic term is
    // an implementation detail and must not reach the pills.
    expect(build([r(1)], '').ast.terms).toHaveLength(0);
  });
  it('sets year and cutAgainstYear so a window cannot be labelled against a different year', () => {
    const page = build([r(1, { lastTouchedAt: Math.floor(Date.UTC(2019, 3, 1) / 1000) })]);
    expect(page.sections[0]).toMatchObject({
      id: 'era:2019',
      year: 2019,
      cutAgainstYear: 2026,
      label: '2019',
    });
  });
  it('gives non-year sections a null year', () => {
    expect(build([r(1)]).sections[0]?.year).toBeNull();
  });
  it('addresses the flat section order, which is what the mount window indexes', () => {
    const rows = [r(1), r(2, { lastTouchedAt: NOW - 400 * DAY }), r(3, { isReference: true })];
    const page = build(rows);
    expect(page.orderKey).toBe(orderKeyOf([1, 2]));
  });
});

describe('orderKeyOf', () => {
  it('is stable for the same ordered ids and changes when the order changes', () => {
    expect(orderKeyOf([1, 2, 3])).toBe(orderKeyOf([1, 2, 3]));
    expect(orderKeyOf([1, 2, 3])).not.toBe(orderKeyOf([3, 2, 1]));
  });
  it('changes when a row is inserted, which is the signal to re-cut the window', () => {
    expect(orderKeyOf([1, 2])).not.toBe(orderKeyOf([1, 2, 3]));
  });
});

describe('compareRows', () => {
  it('sorts last_touched newest first', () => {
    const rows = [r(1, { lastTouchedAt: 10 }), r(2, { lastTouchedAt: 20 })];
    expect([...rows].sort(compareRows('last_touched')).map((x) => x.id)).toEqual([2, 1]);
  });
  it('sorts name case-insensitively and breaks ties on id', () => {
    const rows = [r(2, { name: 'beta' }), r(1, { name: 'Alpha' })];
    expect([...rows].sort(compareRows('name')).map((x) => x.id)).toEqual([1, 2]);
  });
  it('sorts size largest first and puts unmeasured rows last, never at zero', () => {
    const rows = [r(1), r(2, { sizeTrackedBytes: 5 }), r(3, { sizeTrackedBytes: 50 })];
    expect([...rows].sort(compareRows('size')).map((x) => x.id)).toEqual([3, 2, 1]);
  });
  it('puts an unmeasured project behind a measured zero, which is a real zero', () => {
    const rows = [r(1), r(2, { sizeTrackedBytes: 0 })];
    expect([...rows].sort(compareRows('size')).map((x) => x.id)).toEqual([2, 1]);
  });
});
