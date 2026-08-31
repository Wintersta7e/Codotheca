import { describe, expect, it } from 'vitest';
import type { ProjectId } from '../generated/protocol.js';
import { QUERY_GRAMMAR_VERSION } from '../shared/query/grammar.js';
import { projectionCapabilities } from './shelf/row.js';
import type { QueryContext } from './shelf/evaluate.js';
import { toShelfRow } from './shelf/row.js';
import { makeProjectRow } from './testing/projectRow.js';
import { createRendererEngine } from './upstream.js';

/**
 * The production `QueryEngine`, not the fake. R1/R40's recurring defect is a seam that ships with
 * only its test double behind it: every module in this plan is tested against `fakeEngine`, so
 * without this file the real adapter onto plan 13 would be typecheck-only and its shape
 * adaptation — `(rows, ast, ctx)` returning `{rows, ignored}` versus `(ast, rows)` returning the
 * rows — would first be exercised in the product.
 */
const rows = [
  makeProjectRow({ id: 1 as ProjectId, name: 'Nightfall', primaryLanguage: 'Rust' }),
  makeProjectRow({ id: 2 as ProjectId, name: 'Offshore', primaryLanguage: 'Go' }),
];

const context = (): QueryContext => ({
  now: 1_800_000_000,
  firstRunCompletedAt: null,
  collectionIdsByName: new Map<string, number>(),
  pathsAreCaseSensitive: false,
  capabilities: projectionCapabilities(rows.map(toShelfRow)),
  commitSubjectHits: null,
});

describe('createRendererEngine', () => {
  it('speaks the grammar version plan 13 declares, with no second copy of the number', () => {
    expect(createRendererEngine(context()).grammarVersion).toBe(QUERY_GRAMMAR_VERSION);
  });

  it('parses through plan 13’s parser and reads soft errors off the AST it returns', () => {
    const engine = createRendererEngine(context());
    const clean = engine.parse('lang:rust');
    expect(clean.dropped).toEqual([]);
    expect(engine.termCount(clean.ast)).toBe(1);
    expect(engine.hasField(clean.ast, 'lang')).toBe(true);
    expect(engine.hasField(clean.ast, 'collection')).toBe(false);

    const soft = engine.parse('sparkle:yes');
    expect(soft.dropped.map((d) => d.text)).toEqual(['sparkle:yes']);
    expect(engine.termCount(soft.ast)).toBe(0);
  });

  it('canonicalises from the AST rather than reprinting the raw string', () => {
    const engine = createRendererEngine(context());
    expect(engine.canonical(engine.parse('   lang:rust    ').ast)).toBe('lang:rust');
  });

  // The adapter's whole job: `evaluateQuery` is `(rows, ast, ctx) -> {rows, ignored}` and the
  // seam is `(ast, rows) -> rows`. A shape mistake here silently returns everything or nothing.
  it('filters the projection through the same function the shelf runs', () => {
    const engine = createRendererEngine(context());
    expect(engine.filter(engine.parse('lang:rust').ast, rows).map((r) => r.name)).toEqual([
      'Nightfall',
    ]);
    expect(engine.filter(engine.parse('').ast, rows)).toHaveLength(2);
    expect(engine.filter(engine.parse('lang:cobol').ast, rows)).toHaveLength(0);
  });
});
