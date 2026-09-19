import { describe, expect, it } from 'vitest';
import type {
  HealthState,
  HealthSummary,
  LocationRef,
  ProjectRow,
  SortKey,
} from '../../generated/protocol.js';
import type { ShelfRow } from './row.js';
import { rankOf, toShelfRow } from './row.js';
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
    // [p3] §30.1's most boring reading is the one that says nothing was computed: `absent`, every
    // quantity null. `healthSummary` is non-nullable (R116), so a fixture without it is a shape
    // the wire cannot produce — and a fixture defaulting to `live` with a zero count would rank
    // every row that never asked about health.
    healthSummary: {
      state: 'absent',
      scoredOpen: null,
      unverified: null,
      unknownChecks: null,
      observedAt: null,
    },
    lifecycle: 'active',
    ...over,
  } as unknown as ProjectRow);
}

/** A reading, for the rows a health test is about. */
function reading(state: HealthState, scoredOpen: number | null): HealthSummary {
  return { state, scoredOpen, unverified: null, unknownChecks: null, observedAt: null };
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
    health: false,
  },
};
const build = (
  rows: ShelfRow[],
  query = '',
  sort: SortKey = 'last_touched',
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
    expect(build([r(1)], 'completion:>5').ignored).toEqual([
      { text: 'completion:>5', reason: 'notComputed' },
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
    // The other side is `core/tests/projects_list.rs:278`, in the test named
    // `the_order_key_is_the_same_cursor_the_renderer_computes`. **The literal is the shared
    // value, not a golden of this implementation** — and it sits after the two assertions above
    // on purpose: change the FNV offset basis and they still pass while this one fails, which is
    // the whole of §27.7's finding in one run.
    expect(orderKeyOf([1, 2, 3])).toBe('794671b5');
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

  // §35.3. `compareRows` ends in an unconditional return, so before the branch below landed
  // `needs_attention` sorted as `last_touched` and `tsc` stayed green — which is why this case
  // was written first.
  it('AC-P3-35-2 the tail never interleaves and is never ordered as zero', () => {
    const rows = [
      r(1, { healthSummary: reading('live', 3) }),
      r(2, { healthSummary: reading('live', 7) }),
      r(3, { healthSummary: reading('live', 0) }),
      r(4, { healthSummary: reading('absent', null) }),
      r(5, { healthSummary: reading('suppressed', null) }),
    ];
    const ranked = rows.filter((row) => rankOf(row) !== null).length;
    const tail = rows.length - ranked;
    expect(ranked, `${String(ranked)} ranked rows`).toBeGreaterThan(0);
    expect(tail, `${String(tail)} tail rows`).toBeGreaterThan(0);

    const ids = [...rows].sort(compareRows('needs_attention')).map((x) => x.id as number);
    expect(ids).toEqual([2, 1, 3, 4, 5]);
    const at = (id: number): number => ids.indexOf(id);
    for (const tailId of [4, 5]) {
      for (const rankedId of [1, 2, 3]) {
        expect(at(rankedId), `row ${String(tailId)} carries no reading`).toBeLessThan(at(tailId));
      }
    }
    // A computed zero is ranked, at the bottom of the ranked run — never in the tail.
    expect(at(1)).toBeLessThan(at(3));
    expect(at(2)).toBeLessThan(at(3));
  });

  // §35.4 as corrected by R131/F6 (Done ranks, archived tails) and R128/F10 (`unverified` is
  // counted nowhere). Nine cases, one mechanism — so this needed no production change.
  it('AC-P3-35-1 the exclusion set is applied once, upstream', () => {
    const rows = [
      r(1, { healthSummary: reading('live', 9) }),
      r(2, { healthSummary: reading('live', 4) }),
      r(3, { healthSummary: reading('frozen', 5) }),
      // Done ranks, at the bottom of the ranked run: §30 gives it a `live` reading by construction.
      r(4, { healthSummary: reading('live', 0), lifecycle: 'done' }),
      // §23.4 keeps a not-cloned row in the base set, so it ranks if it has a reading.
      r(5, { healthSummary: reading('live', 2), primaryLocation: null, presence: null }),
      r(6, { healthSummary: reading('absent', null), isReference: true }),
      // Archived tails on the `surface_suppressed` gate that produces it, not on the flag.
      r(7, { healthSummary: reading('suppressed', null), isArchived: true, lifecycle: 'archived' }),
      r(8, { healthSummary: reading('suppressed', null) }),
    ];
    expect(rows.length, `${String(rows.length)} fixture rows`).toBeGreaterThanOrEqual(8);

    const order = (from: readonly ShelfRow[]): number[] =>
      [...from].sort(compareRows('needs_attention')).map((x) => x.id as number);
    const expected = [1, 3, 2, 5, 4, 6, 7, 8];
    expect(order(rows)).toEqual(expected);

    // The discriminating half: the readings are held and the flags move. A comparator carrying a
    // second gate moves the row; one carrying none does not.
    const flipped = rows.map((row, index) =>
      index === 1 ? { ...row, isReference: true, isArchived: true } : row,
    );
    expect(order(flipped), 'the comparator read a flag §35.4 forbids it to read').toEqual(expected);

    // A rank may not drift as a reading ages — *presence freezes decay*.
    const aged = rows.map((row, index) =>
      index === 2
        ? { ...row, healthSummary: { ...row.healthSummary, observedAt: NOW - 4000 * DAY } }
        : row,
    );
    expect(order(aged)).toEqual(expected);

    // A12b: two units never enter one expression. Neither `unverified` nor `unknownChecks` is an
    // addend, a weight or a tiebreak.
    const noisy = rows.map((row, index) => ({
      ...row,
      healthSummary: {
        ...row.healthSummary,
        unverified: index * 7,
        unknownChecks: (9 - index) * 3,
      },
    }));
    expect(order(noisy)).toEqual(expected);

    // §8.1's Reference block is sorted by the same comparator, so with no reference row carrying
    // a reading the block renders in the default order.
    const references = [
      r(20, { isReference: true, lastTouchedAt: NOW - 3 * DAY }),
      r(21, { isReference: true, lastTouchedAt: NOW - DAY }),
      r(22, { isReference: true, lastTouchedAt: NOW - 2 * DAY }),
    ];
    expect(build(references, '', 'needs_attention').reference.map((row) => row.id)).toEqual([
      21, 22, 20,
    ]);
  });
});
