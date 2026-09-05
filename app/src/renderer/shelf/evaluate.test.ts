import { describe, expect, it } from 'vitest';
import type { LocationRef, ProjectRow } from '../../generated/protocol.js';
import type { QueryTerm } from '../../shared/query/ast.js';
import { parseQuery } from '../../shared/query/parse.js';
import { isNewArrival } from '../firstrun/newArrivals.js';
import type { ShelfRow } from './row.js';
import { toShelfRow } from './row.js';
import type { QueryContext } from './evaluate.js';
import { evaluateQuery, partitionAnswerable, termTruth } from './evaluate.js';

const NOW = 1_800_000_000;
const DAY = 86_400;

function base(id: number, over: Record<string, unknown> = {}): ShelfRow {
  return toShelfRow({
    id,
    name: `p${id}`,
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

const ctx: QueryContext = {
  now: NOW,
  firstRunCompletedAt: NOW - 100 * DAY,
  collectionIdsByName: new Map([['side projects', 4]]),
  pathsAreCaseSensitive: false,
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
  commitSubjectHits: null,
};

function only(query: string): QueryTerm {
  const [term] = parseQuery(query).terms;
  if (term === undefined) throw new Error(`no term parsed from ${query}`);
  return term;
}

describe('termTruth is three-valued', () => {
  it('is null for is:dirty when the worktree has never been observed', () => {
    expect(termTruth(base(1), only('is:dirty'), ctx)).toBeNull();
  });
  it('is true when observed dirty and false when observed not dirty', () => {
    expect(termTruth(base(1, { isDirty: true }), only('is:dirty'), ctx)).toBe(true);
    expect(termTruth(base(1, { isDirty: false }), only('is:dirty'), ctx)).toBe(false);
  });
  it('is null for size when inventory has not run', () => {
    expect(termTruth(base(1), only('size:>1mb'), ctx)).toBeNull();
  });
  it('answers touched from lastTouchedAt', () => {
    const cold = base(1, { lastTouchedAt: NOW - 400 * DAY });
    expect(termTruth(cold, only('touched:>365d'), ctx)).toBe(true);
    expect(termTruth(base(2), only('touched:>365d'), ctx)).toBe(false);
  });
  it('answers a touched year against the local calendar year of lastTouchedAt', () => {
    const year = new Date(NOW * 1000).getFullYear();
    expect(termTruth(base(1), only(`touched:${year}`), ctx)).toBe(true);
    expect(termTruth(base(1), only(`touched:${year - 1}`), ctx)).toBe(false);
  });
  it('answers is:unpushed from ahead, and is null when ahead is unknown', () => {
    expect(termTruth(base(1, { ahead: 2 }), only('is:unpushed'), ctx)).toBe(true);
    expect(termTruth(base(1, { ahead: 0 }), only('is:unpushed'), ctx)).toBe(false);
    expect(termTruth(base(1), only('is:unpushed'), ctx)).toBeNull();
  });
  it('answers is:new from the acknowledgement clock, §10.5a', () => {
    const fresh = base(1, { createdAt: NOW - 2 * DAY });
    expect(termTruth(fresh, only('is:new'), ctx)).toBe(true);
    expect(
      termTruth(base(2, { createdAt: NOW - 2 * DAY, acknowledgedAt: NOW }), only('is:new'), ctx),
    ).toBe(false);
  });
  // R18 named two copies of the NEW predicate; this file carried a third. The flag now routes
  // through `isNewArrival`, so the chip on a card, the count in the arrivals row and the query
  // result cannot disagree about what "new" means — this reads the other side to prove it.
  it('answers is:new from the one predicate, not a second reading of the two columns', () => {
    const stamp = ctx.firstRunCompletedAt ?? 0;
    const rows = [
      base(1, { createdAt: NOW - 2 * DAY }),
      base(2, { createdAt: NOW - 2 * DAY, acknowledgedAt: NOW }),
      base(3, { createdAt: stamp }),
      base(4, { createdAt: stamp - 1 }),
    ];
    for (const row of rows) {
      expect(termTruth(row, only('is:new'), ctx)).toBe(isNewArrival(row, stamp));
    }
  });
  it('matches a bare word against name, owner, description, path and the last subject', () => {
    const row = base(1, { name: 'Codo', primaryLocation: { id: 3, pathDisplay: '/w/codo' } });
    expect(termTruth(row, only('codo'), ctx)).toBe(true);
    expect(termTruth(row, only('nothing'), ctx)).toBe(false);
  });
  it('answers collection membership by name', () => {
    expect(
      termTruth(base(1, { collectionIds: [4] }), only('collection:"side projects"'), ctx),
    ).toBe(true);
    expect(termTruth(base(1, { collectionIds: [] }), only('collection:"side projects"'), ctx)).toBe(
      false,
    );
  });
});

describe('evaluateQuery', () => {
  it('excludes a row whose term is unknown, positive or negated', () => {
    // Never render unknown as zero — and never filter on it either.
    const rows = [base(1, { isDirty: true }), base(2, { isDirty: false }), base(3)];
    expect(evaluateQuery(rows, parseQuery('is:dirty'), ctx).rows.map((r) => r.id)).toEqual([1]);
    expect(evaluateQuery(rows, parseQuery('-is:dirty'), ctx).rows.map((r) => r.id)).toEqual([2]);
  });
  it('ANDs terms', () => {
    const rows = [base(1, { isDirty: true, isPinned: true }), base(2, { isDirty: true })];
    expect(
      evaluateQuery(rows, parseQuery('is:dirty is:pinned'), ctx).rows.map((r) => r.id),
    ).toEqual([1]);
  });
  it('a bare query returns no reference rows and no hidden rows', () => {
    // §8.0b: this is what makes ALL and the headline agree.
    const rows = [base(1), base(2, { isReference: true }), base(3, { isHidden: true })];
    expect(evaluateQuery(rows, parseQuery(''), ctx).rows.map((r) => r.id)).toEqual([1]);
  });
  it('is:reference and is:hidden opt their own rows back in', () => {
    const rows = [base(1), base(2, { isReference: true })];
    expect(evaluateQuery(rows, parseQuery('is:reference'), ctx).rows.map((r) => r.id)).toEqual([2]);
  });
  it('drops a term the projection cannot answer and runs the rest', () => {
    const rows = [base(1, { isDirty: true }), base(2, { isDirty: false })];
    const result = evaluateQuery(rows, parseQuery('has:ci is:dirty'), ctx);
    expect(result.rows.map((r) => r.id)).toEqual([1]);
    expect(result.ignored).toEqual([{ text: 'has:ci', reason: 'notAvailable' }]);
  });
  it("carries the parser's own soft errors through", () => {
    expect(evaluateQuery([base(1)], parseQuery('completion:>5'), ctx).ignored).toEqual([
      { text: 'completion:>5', reason: 'notComputed' },
    ]);
  });
  it('drops is:new when the first-run clock is unavailable rather than guessing', () => {
    const blind = { ...ctx, firstRunCompletedAt: null };
    expect(evaluateQuery([base(1)], parseQuery('is:new'), blind).ignored).toEqual([
      { text: 'is:new', reason: 'notAvailable' },
    ]);
  });
});

describe('partitionAnswerable', () => {
  it('keeps a term the projection can answer', () => {
    expect(partitionAnswerable(parseQuery('is:dirty'), ctx).runnable).toHaveLength(1);
  });
  it('moves in:local out when the projection has no location kind', () => {
    expect(partitionAnswerable(parseQuery('in:local'), ctx).ignored).toEqual([
      { text: 'in:local', reason: 'notAvailable' },
    ]);
  });
  it('keeps in:<path prefix>, which primaryLocation.pathDisplay answers', () => {
    expect(partitionAnswerable(parseQuery('in:"/w"'), ctx).runnable).toHaveLength(1);
  });
});
