import { describe, expect, it } from 'vitest';
import type { LocationRef, ProjectRow } from '../../generated/protocol.js';
import type { ShelfRow } from './row.js';
import { toShelfRow } from './row.js';
import type { QueryContext } from './evaluate.js';
import countsRaw from './counts.ts?raw';
import schemaRaw from '../../../../protocol/schema/protocol.json?raw';
import { healthIdentifiers } from '../../../../scripts/check-health-escape.mjs';
import { parseQuery } from '../../shared/query/parse.js';
import { evaluateQuery } from './evaluate.js';
import { ATTENTION_CHIPS, attentionCounts, headlineText, shelfCounts } from './counts.js';

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
    health: false,
  },
};

describe('shelfCounts', () => {
  it('never counts a reference row in the denominator it says it excluded', () => {
    const rows = [base(1), base(2), base(3, { isReference: true })];
    const counts = shelfCounts(rows, 2);
    expect(counts.total).toBe(2);
    expect(counts.reference).toBe(1);
  });
  it('reports classification as unknown while authored_by_user is NULL everywhere', () => {
    expect(shelfCounts([base(1)], 1).classificationKnown).toBe(false);
  });
  it('counts classified rows once the field arrives', () => {
    const rows = [
      base(1),
      toShelfRow({ ...base(2), authoredByUser: true } as unknown as ProjectRow),
    ];
    const counts = shelfCounts(rows, 2);
    expect(counts.classificationKnown).toBe(true);
    expect(counts.classified).toBe(1);
  });
});

describe('headlineText', () => {
  it('reads N OF total · SORT · reference excluded', () => {
    const counts = {
      matched: 12,
      total: 180,
      reference: 24,
      classified: 141,
      classificationKnown: true,
    };
    expect(headlineText(counts, 'LAST TOUCHED')).toBe(
      '12 OF 180 · LAST TOUCHED · 24 REFERENCE EXCLUDED (OF 141 CLASSIFIED)',
    );
  });
  it('qualifies the exclusion wherever classification is not complete', () => {
    const counts = {
      matched: 1,
      total: 2,
      reference: 0,
      classified: 0,
      classificationKnown: false,
    };
    expect(headlineText(counts, 'NAME')).toBe(
      '1 OF 2 · NAME · 0 REFERENCE EXCLUDED (OF 0 CLASSIFIED)',
    );
  });
  it('drops the qualifier only when every row is classified', () => {
    const counts = { matched: 2, total: 2, reference: 0, classified: 2, classificationKnown: true };
    expect(headlineText(counts, 'NAME')).toBe('2 OF 2 · NAME · 0 REFERENCE EXCLUDED');
  });
});

describe('attentionCounts', () => {
  it("runs each chip's own query through the grammar", () => {
    const rows = [
      base(1, { ahead: 2 }),
      base(2, { isDirty: true }),
      base(3, { lastTouchedAt: NOW - 400 * DAY }),
      base(4, { isReference: true }),
    ];
    const counts = attentionCounts(rows, ctx);
    expect(counts['all']).toBe(3);
    expect(counts['unpushed']).toBe(1);
    expect(counts['uncommitted']).toBe(1);
    expect(counts['cold']).toBe(1);
  });
  it('agrees with the headline denominator on ALL', () => {
    const rows = [base(1), base(2, { isReference: true })];
    expect(attentionCounts(rows, ctx)['all']).toBe(shelfCounts(rows, 1).total);
  });
});

// [p3] `AC-P3-35-8`, §35.8.1–2. Every chip's rendered number is the row count of its **own**
// query through the same evaluator the shelf filters with — never a second, separately written
// predicate — and never an aggregate, a score or a band.
//
// **This lane adds no chip**, and that is recorded rather than defaulted: §35.1's row b is
// conditioned on §32, and §32 answered under R131/F8 — *"§32 RULES NO CHIP IN"*, on three
// grounds, the first being that the count would have to come off a `ShelfRow` while R119 puts
// `dependencyVerdict` on `ProjectDetail`. The test below is table-driven, so it holds over four
// rows or five without an edit here.
describe('AC-P3-35-8 every chip is one predicate and its own query row count', () => {
  const ENTRY_KEYS = ['id', 'label', 'sub', 'query', 'accent'];
  // A fixture where the four predicates give four **different** answers. One where every chip
  // counts the same number proves nothing.
  const rows = [
    base(1, { ahead: 2 }),
    base(2, { isDirty: true }),
    base(3, { isDirty: true }),
    base(4, { lastTouchedAt: NOW - 400 * DAY }),
    base(5, { lastTouchedAt: NOW - 500 * DAY }),
    base(6, { lastTouchedAt: NOW - 600 * DAY }),
  ];

  it('counts each chip through the evaluator, and nothing else does the counting', () => {
    expect(ATTENTION_CHIPS.length, `${String(ATTENTION_CHIPS.length)} chips`).toBeGreaterThan(0);
    const counts = attentionCounts(rows, ctx);
    const seen = new Set<number>();
    for (const chip of ATTENTION_CHIPS) {
      expect(typeof chip.query, `${chip.id} has no query`).toBe('string');
      const ast = parseQuery(chip.query);
      expect(ast.ignored, `${chip.id} carries a term the grammar drops`).toEqual([]);
      expect(counts[chip.id], `${chip.id} is not its own query row count`).toBe(
        evaluateQuery(rows, ast, ctx).rows.length,
      );
      seen.add(counts[chip.id] ?? -1);
      // No entry carries a number of its own: no literal, no precomputed total, no second
      // predicate field. The rendered number has one producer.
      expect(Object.keys(chip).sort()).toEqual([...ENTRY_KEYS].sort());
      for (const value of Object.values(chip)) expect(typeof value).toBe('string');
    }
    expect(seen.size, 'a fixture where every chip counts the same proves nothing').toBe(
      ATTENTION_CHIPS.length,
    );
  });

  it('renders no aggregate, no score and no band', () => {
    // §25.4 drew the line already — *the aggregate is the check, and the check is phase 3* — and
    // phase 3 inherits the distinction, not permission to erase it. The aggregate may be rendered
    // on the project page, where the ticks and the debt list show what it is made of.
    for (const chip of ATTENTION_CHIPS) {
      for (const copy of [chip.label, chip.sub]) {
        expect(copy, `${chip.id} renders an aggregate`).not.toMatch(
          /\b(score|band|grade|health|rank|points?)\b/iu,
        );
        expect(copy, `${chip.id} renders a number`).not.toMatch(/\d/u);
      }
    }
  });

  it('names no health identifier in the module that produces the counts', () => {
    // The derived set is the health-escape gate's own, so the two cannot drift.
    const identifiers = healthIdentifiers(JSON.parse(schemaRaw));
    expect(identifiers.length, 'a run with no derived identifier proves nothing').toBeGreaterThan(
      0,
    );
    for (const identifier of identifiers) {
      expect(countsRaw, `counts.ts names ${identifier}`).not.toMatch(
        new RegExp(`\\b${identifier}\\b`, 'u'),
      );
    }
  });
});

describe('ATTENTION_CHIPS', () => {
  it("is exactly §8.0b's four, with their exact queries", () => {
    expect(ATTENTION_CHIPS.map((c) => [c.label, c.query])).toEqual([
      ['ALL', ''],
      ['UNPUSHED', 'is:unpushed'],
      ['UNCOMMITTED', 'is:dirty'],
      ['COLD', 'touched:>365d'],
    ]);
  });
  it('carries no UNSORTED chip — phase 1 issues no verdict', () => {
    expect(ATTENTION_CHIPS.map((c) => c.label)).not.toContain('UNSORTED');
  });
});
