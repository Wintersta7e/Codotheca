import { describe, expect, it } from 'vitest';
import type { ProjectRow } from '../../generated/protocol.js';
import type { ShelfRow } from './row.js';
import { toShelfRow } from './row.js';
import type { QueryContext } from './evaluate.js';
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
    primaryLocation: null,
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
