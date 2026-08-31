import { describe, expect, it } from 'vitest';
import type { Collection, CollectionId } from '../../generated/protocol.js';
import type { QueryAst, QueryTerm } from '../../shared/query/ast.js';
import { fakeEngine } from './engine.js';
import { BROKEN_REASON_TEXT, collectionQueryHealth } from './grammar.js';

const saved = (over: Partial<Collection> = {}): Collection => ({
  id: 1 as CollectionId,
  name: 'Rust work',
  kind: 'query',
  queryText: 'lang:rust is:dirty',
  queryGrammarVersion: 1,
  sortIndex: 0,
  memberCount: null,
  ...over,
});

// R13: plan 13's own `QueryAst`, built honestly rather than a cast over a shape that only looks
// like one. The plan's own snippet used `{ terms: ['lang:rust'] }`, which does not typecheck.
const ast = (...texts: readonly string[]): QueryAst => ({
  grammarVersion: 1,
  terms: texts.map((text): QueryTerm => ({ kind: 'bare', negated: false, text })),
  ignored: [],
});

describe('collectionQueryHealth', () => {
  it('runs every term when nothing was dropped', () => {
    const health = collectionQueryHealth(saved(), fakeEngine());
    expect(health.kind).toBe('ok');
  });

  it('is degraded when some terms parse and some do not', () => {
    const engine = fakeEngine({
      parse: (text) => ({
        ast: ast(text.split(' ')[0] ?? ''),
        dropped: [{ text: 'sparkle:yes', reason: 'unknownField' }],
      }),
    });
    const health = collectionQueryHealth(saved(), engine);
    expect(health.kind).toBe('degraded');
    if (health.kind !== 'degraded') return;
    expect(health.dropped).toHaveLength(1);
  });

  it('is broken when no term survives', () => {
    const engine = fakeEngine({
      parse: () => ({
        ast: ast(),
        dropped: [{ text: 'sparkle:yes', reason: 'unknownField' }],
      }),
    });
    expect(collectionQueryHealth(saved(), engine)).toEqual({
      kind: 'broken',
      reason: 'no_longer_parses',
    });
  });

  // §8.8: above the running build's version the parse is not attempted at all.
  it('does not attempt a query written for a later grammar', () => {
    let parsed = 0;
    const engine = fakeEngine({
      grammarVersion: 2,
      parse: (text) => {
        parsed += 1;
        return { ast: ast(...text.split(' ')), dropped: [] };
      },
    });
    expect(collectionQueryHealth(saved({ queryGrammarVersion: 3 }), engine)).toEqual({
      kind: 'broken',
      reason: 'future_grammar',
    });
    expect(parsed).toBe(0);
  });

  it('accepts a query written against an older grammar', () => {
    const engine = fakeEngine({ grammarVersion: 4 });
    expect(collectionQueryHealth(saved({ queryGrammarVersion: 1 }), engine).kind).toBe('ok');
  });

  it('treats a missing or empty query text as broken, never as a query matching everything', () => {
    expect(collectionQueryHealth(saved({ queryText: null }), fakeEngine())).toEqual({
      kind: 'broken',
      reason: 'no_longer_parses',
    });
    expect(collectionQueryHealth(saved({ queryText: '   ' }), fakeEngine())).toEqual({
      kind: 'broken',
      reason: 'no_longer_parses',
    });
  });

  // §8.8: `query_grammar_version` records the version the text was written against and is never
  // silently rewritten — a successful parse against a newer grammar changes nothing.
  it('leaves the stored collection exactly as it found it', () => {
    const collection = saved({ queryGrammarVersion: 1 });
    const before = { ...collection };
    collectionQueryHealth(collection, fakeEngine({ grammarVersion: 9 }));
    expect(collection).toEqual(before);
  });

  it('states the reason in §8.8’s words', () => {
    expect(BROKEN_REASON_TEXT.no_longer_parses('lang:rust')).toBe('NO LONGER PARSES · lang:rust');
    expect(BROKEN_REASON_TEXT.future_grammar('lang:rust')).toBe(
      'WRITTEN FOR A LATER VERSION OF THE QUERY LANGUAGE',
    );
  });
});
