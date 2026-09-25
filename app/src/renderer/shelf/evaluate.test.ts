import { describe, expect, it } from 'vitest';
import type { LocationRef, ProjectId, ProjectRow } from '../../generated/protocol.js';
import type { QueryTerm } from '../../shared/query/ast.js';
import { parseQuery } from '../../shared/query/parse.js';
import { isNewArrival } from '../firstrun/newArrivals.js';
import type { ShelfRow } from './row.js';
import { toShelfRow } from './row.js';
import type { QueryContext } from './evaluate.js';
import { evaluateQuery, partitionAnswerable, termTruth } from './evaluate.js';
import { projectionCapabilities } from './row.js';
import { makeProjectRow } from '../testing/projectRow.js';
import { HAS_ATTRIBUTES, IS_FLAGS } from '../../shared/query/grammar.js';

const NOW = 1_800_000_000;
const DAY = 86_400;

function base(id: number, over: Record<string, unknown> = {}): ShelfRow {
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
    health: false,
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
    expect(termTruth(base(1), only(`touched:${String(year)}`), ctx)).toBe(true);
    expect(termTruth(base(1), only(`touched:${String(year - 1)}`), ctx)).toBe(false);
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
    expect(evaluateQuery([base(1)], parseQuery('is:sideways'), ctx).ignored).toEqual([
      { text: 'is:sideways', reason: 'malformedValue' },
    ]);
  });
  // [p3] §31.1: the term filters, and **a row with no measurement matches neither comparison**.
  // It is not ignored — it is answered `Unknown`, which no polarity matches.
  it('answers completion for a measured row and excludes an unmeasured one from both sides', () => {
    const measured = { ...base(1), completionLit: 8, completionApplicable: 10 };
    const above = evaluateQuery([measured], parseQuery('completion:>5'), ctx);
    expect(above.ignored).toEqual([]);
    expect(above.rows.map((r) => r.id)).toEqual([1]);

    const unmeasured = base(2);
    expect(evaluateQuery([unmeasured], parseQuery('completion:>5'), ctx).rows).toEqual([]);
    expect(evaluateQuery([unmeasured], parseQuery('completion:<5'), ctx).rows).toEqual([]);
    expect(evaluateQuery([unmeasured], parseQuery('-completion:>5'), ctx).rows).toEqual([]);
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

/**
 * AC-P2-23-7, TypeScript engine — the same fixture shape the Rust half runs, so the two mirrors
 * are asserted against one contract rather than two.
 *
 * > A `has:` attribute, the working-copy `is:` flags, and `touched:` are predicates **about a
 * > working copy**. For a project with zero `location` rows every one of them evaluates to
 * > Unknown.
 */
describe('§23.6: the domain rule, and is:notcloned', () => {
  const notCloned = base(1, {
    primaryLocation: null,
    presence: null,
    primaryLanguage: 'Rust',
    lastTouchedAt: NOW - 10 * DAY,
    hasRemote: true,
  });
  const locatedWithRemote = base(2, { lastTouchedAt: NOW - 10 * DAY, hasRemote: true });
  const locatedWithoutRemote = base(3, { lastTouchedAt: NOW - 10 * DAY, hasRemote: false });
  const rows = [notCloned, locatedWithRemote, locatedWithoutRemote];

  // Everything the projection can answer, so `answerable` does not drop a term before the domain
  // rule gets to it — the two mechanisms must not be confused.
  const wide: QueryContext = {
    ...ctx,
    capabilities: {
      authoredByUser: true,
      location: true,
      hasReadme: true,
      hasLicense: true,
      hasTests: true,
      hasCi: true,
      hasRemote: true,
      hasSubmodules: true,
      health: true,
    },
  };

  const ids = (query: string): number[] =>
    evaluateQuery(rows, parseQuery(query), wide).rows.map((r) => r.id as number);

  it('answers Unknown for every working-copy predicate, under neither polarity', () => {
    const outside = [
      'has:remote',
      'has:submodules',
      'has:readme',
      'has:stash',
      'is:bare',
      'is:shallow',
      'is:dirty',
      'is:unpushed',
      'is:behind',
      'is:interrupted',
      'is:empty',
      'is:local',
      'is:wsl',
      'touched:>1d',
      'touched:<1d',
    ];
    let evaluated = 0;
    for (const query of outside) {
      expect(termTruth(notCloned, only(query), wide), `${query} answered about no copy`).toBeNull();
      expect(ids(query), `${query} matched the zero-location row`).not.toContain(1);
      expect(ids(`-${query}`), `-${query} matched the zero-location row`).not.toContain(1);
      evaluated += 3;
    }
    expect(evaluated, 'a run that evaluated no term proves nothing').toBe(outside.length * 3);
    expect(rows.length).toBeGreaterThan(0);
  });

  it('leaves the project-row facts known', () => {
    for (const query of [
      'is:archived',
      'is:pinned',
      'is:hidden',
      'is:reference',
      'is:fork',
      'is:notcloned',
    ]) {
      expect(termTruth(notCloned, only(query), wide), `${query} lost its answer`).not.toBeNull();
    }
  });

  it('returns exactly the zero-location rows for is:notcloned', () => {
    expect(ids('is:notcloned')).toEqual([1]);
    expect(ids('-is:notcloned')).toEqual([2, 3]);
  });

  it('keeps -has:remote meaning a local copy with no remote configured', () => {
    // The inversion, in one assertion: without the rule this returns [1, 3].
    expect(ids('-has:remote')).toEqual([3]);
    expect(ids('has:remote')).toEqual([2]);
  });

  it('keeps not-cloned rows in the base set and lets lang: match them', () => {
    expect(ids('')).toEqual([1, 2, 3]);
    expect(ids('lang:rust')).toEqual([1]);
  });

  it('drops no term for a not-cloned row: the rule is truth, not answerability', () => {
    // §8.3 drops a term the *projection* cannot answer. The domain rule is a different
    // mechanism — the projection answers it, and the answer is Unknown.
    const { runnable, ignored } = partitionAnswerable(parseQuery('has:remote is:notcloned'), wide);
    expect(runnable).toHaveLength(2);
    expect(ignored).toHaveLength(0);
  });
});

/**
 * AC-P2-23-12, **narrowed** (Deviation 5). §23.7's own sentence scopes it: *"`is:notcloned` needs
 * no new projection field; `has:remote` needs its producer."*
 *
 * `has:remote` was live in the core and **dead in the renderer** — the core answered it and the
 * projection carried no field, so `answerable` dropped it and §8.3's client-side filtering meant
 * the shipping behaviour was *ignored*. That is R1/R35a/R40/R46 in projection form.
 *
 * The literal reading — *no `is:` or `has:` value evaluates to `notAvailable` for any row* —
 * cannot hold: `ProjectRow` carries none of `ProjectRowExtras`, and `has:license|tests|ci` have
 * no column in the core either, so there is no producer to land. The narrowed claim is asserted
 * here and the remaining gap is **printed as an exact named list** rather than left silent.
 */
describe('§23.7: no dead predicate ships', () => {
  // Built through `toShelfRow` over a **real `ProjectRow`**, with no extras carried in. That is
  // the production path: the wire row is what the shelf receives, and a fixture that passed
  // `hasRemote` alongside it would answer the capability itself and assert nothing.
  const located = toShelfRow(makeProjectRow({ id: 1 as ProjectId, hasRemote: true }));
  const bare = toShelfRow(
    // `hasRemote: true` deliberately: in phase 2 every not-cloned project carries a remote_key
    // by construction, so this is the case that would invert. If the Unknown below passed for
    // the missing-field reason the plan would have shipped the defect it came to fix.
    makeProjectRow({ id: 2 as ProjectId, primaryLocation: null, presence: null, hasRemote: true }),
  );

  it('answers has:remote from the projection over a corpus with a located row', () => {
    const caps = projectionCapabilities([located, bare]);
    expect(caps.hasRemote).toBe(true);
  });

  it('makes has:remote and is:notcloned answerable for every row', () => {
    const wide: QueryContext = { ...ctx, capabilities: projectionCapabilities([located, bare]) };
    for (const query of ['has:remote', 'is:notcloned', '-has:remote', '-is:notcloned']) {
      const { runnable, ignored } = partitionAnswerable(parseQuery(query), wide);
      expect(runnable, `${query} was dropped`).toHaveLength(1);
      expect(ignored).toHaveLength(0);
    }
  });

  it('names the values that remain notAvailable, so the gap cannot grow in silence', () => {
    const wide: QueryContext = { ...ctx, capabilities: projectionCapabilities([located, bare]) };
    const candidates = [
      ...IS_FLAGS.map((flag) => `is:${flag}`),
      ...HAS_ATTRIBUTES.map((attribute) => `has:${attribute}`),
      'in:local',
      'in:wsl',
      'in:wsl:ubuntu',
    ];
    const notAvailable = candidates.filter(
      (query) => partitionAnswerable(parseQuery(query), wide).ignored.length > 0,
    );
    expect(candidates.length, 'a run that examined no term proves nothing').toBeGreaterThan(0);
    // The exact set, by name. `has:license|tests|ci` have no column in the core either, so no
    // producer exists to land; the rest are `ProjectRowExtras` fields `ProjectRow` does not
    // carry. Owner: a phase-1 projection gap that no phase-2 section owns.
    expect([...notAvailable].sort()).toEqual([
      'has:ci',
      'has:license',
      'has:readme',
      'has:submodules',
      'has:tests',
      'in:local',
      'in:wsl',
      'in:wsl:ubuntu',
      'is:local',
      'is:wsl',
    ]);
    expect(notAvailable).not.toContain('has:remote');
    expect(notAvailable).not.toContain('is:notcloned');
  });

  it('still answers has:remote Unknown for a not-cloned row — for the domain reason', () => {
    // If this passed for the *missing-field* reason the plan would have shipped the defect it
    // came to fix, so the fixture is given `hasRemote: true` explicitly and the answer must
    // still be Unknown.
    const wide: QueryContext = { ...ctx, capabilities: projectionCapabilities([located, bare]) };
    expect(bare.hasRemote, 'the row carries the field, so Unknown is the domain answer').toBe(true);
    expect(termTruth(bare, only('has:remote'), wide)).toBeNull();
    expect(termTruth(located, only('has:remote'), wide)).toBe(true);
  });
});
