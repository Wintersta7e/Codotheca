/**
 * `AC-P3-35-3`, the TypeScript half. §35.7: the comparator exists twice by design, and nothing
 * compared the two orders until this pair landed.
 *
 * The fixture is `protocol/shelf/order-corpus.json`, read here and by `core/tests/sort_corpus.rs`
 * in the shape `protocol/query/corpus.json` already uses. Each case carries `expectedOrderKey` as
 * well as `expectedIds`, because two comparators can agree on ids and disagree on the cursor §8.2
 * windows on.
 */
import { describe, expect, it } from 'vitest';
import corpusRaw from '../../../../protocol/shelf/order-corpus.json?raw';
import schemaRaw from '../../../../protocol/schema/protocol.json?raw';
import type { HealthState, LocationRef, ProjectRow, SortKey } from '../../generated/protocol.js';
import { compareRows, orderKeyOf } from './page.js';
import type { ShelfRow } from './row.js';
import { toShelfRow } from './row.js';

interface CorpusRow {
  readonly id: number;
  readonly name: string;
  readonly lastTouchedAt: number;
  readonly sizeTrackedBytes: number | null;
  readonly healthState: HealthState;
  readonly scoredOpen: number | null;
}
interface CorpusCase {
  readonly name: string;
  readonly sort: SortKey;
  readonly now: number;
  readonly rows: readonly CorpusRow[];
  readonly expectedIds: readonly number[];
  readonly expectedOrderKey: string;
}
interface Corpus {
  readonly corpusVersion: number;
  readonly cases: readonly CorpusCase[];
}

/**
 * Both files arrive through Vite's `?raw`, and **not** through `node:fs`.
 *
 * This file belongs to the **dom** project, and `tsconfig.web.json` withholds `@types/node` on
 * purpose — *withholding them makes reaching for one a compile error rather than a review
 * comment* (`app/test/toolchain.test.ts:77-82`). The idiom every node-project test here uses,
 * `fileURLToPath(new URL(…, import.meta.url))`, does not work either: modules are served rather
 * than loaded off disk, so `import.meta.url` is an `http:` URL and `fileURLToPath` throws *The
 * URL must be of scheme file*.
 *
 * It is still the **tracked** schema and not the gitignored generated file, which is the part
 * that matters: a check that reads an ignored path is a check that can never fail.
 */
const corpus = JSON.parse(corpusRaw) as Corpus;
const schema = JSON.parse(schemaRaw) as {
  types: { SortKey: { variants: readonly string[] } };
};

/**
 * Every key a case row may carry. **An unknown key is rejected rather than ignored** — a field
 * added to one side's loader and not the other is exactly the failure a shared fixture would
 * otherwise hide, so both loaders name the offending key.
 */
const ROW_KEYS = [
  'id',
  'name',
  'lastTouchedAt',
  'sizeTrackedBytes',
  'healthState',
  'scoredOpen',
] as const;

function rowOf(entry: CorpusRow): ShelfRow {
  for (const key of Object.keys(entry)) {
    if (!(ROW_KEYS as readonly string[]).includes(key)) {
      throw new Error(`unknown row key in order-corpus.json: ${key}`);
    }
  }
  return toShelfRow({
    id: entry.id,
    name: entry.name,
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
    lastTouchedAt: entry.lastTouchedAt,
    lastInteractionAt: null,
    lastCommitAt: null,
    lastCommitSubject: null,
    firstCommitAt: null,
    createdAt: 0,
    acknowledgedAt: null,
    sizeTrackedBytes: entry.sizeTrackedBytes,
    trackedFiles: null,
    collectionIds: [],
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
    healthSummary: {
      state: entry.healthState,
      scoredOpen: entry.scoredOpen,
      unverified: null,
      unknownChecks: null,
      observedAt: null,
    },
    lifecycle: 'active',
  } as unknown as ProjectRow);
}

describe('AC-P3-35-3 the two comparators produce the same order', () => {
  it('compares a non-empty corpus', () => {
    expect(corpus.cases.length, `compared ${String(corpus.cases.length)} case(s)`).toBeGreaterThan(
      0,
    );
  });

  it('covers every SortKey variant the schema declares, and no other', () => {
    const variants = [...schema.types.SortKey.variants].sort();
    expect(variants.length, `derived ${String(variants.length)} variant(s)`).toBeGreaterThan(0);
    const covered = [...new Set(corpus.cases.map((c) => c.sort))].sort();
    expect(covered).toEqual(variants);
  });

  it('has unique case names, since both languages report failures by name', () => {
    expect(new Set(corpus.cases.map((c) => c.name)).size).toBe(corpus.cases.length);
  });

  it('names a row key it does not know', () => {
    const rogue = { ...corpus.cases[0]!.rows[0]!, weighting: 3 } as unknown as CorpusRow;
    expect(() => rowOf(rogue)).toThrow(/weighting/);
  });

  // One `it` per case, so a divergence names itself the way the Rust half's collected list does.
  for (const testCase of corpus.cases) {
    it(`TS: ${testCase.name}`, () => {
      expect(testCase.now).toBeGreaterThan(0);
      const rows = testCase.rows.map(rowOf);
      const ids = [...rows].sort(compareRows(testCase.sort)).map((row) => row.id as number);
      expect(ids).toEqual([...testCase.expectedIds]);
      expect(orderKeyOf(ids)).toBe(testCase.expectedOrderKey);
    });
  }
});
