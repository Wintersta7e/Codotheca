import { describe, expect, it } from 'vitest';
import { makeProjectRow } from '../testing/projectRow.js';
import { fakeEngine } from './engine.js';

describe('fakeEngine', () => {
  it('is a complete QueryEngine so every other test can be written against one', () => {
    const engine = fakeEngine();
    expect(engine.grammarVersion).toBeGreaterThan(0);
    expect(engine.parse('is:dirty').dropped).toEqual([]);
    expect(engine.canonical(engine.parse('is:dirty').ast)).toBe('is:dirty');
    expect(engine.termCount(engine.parse('is:dirty lang:rust').ast)).toBe(2);
    expect(engine.hasField(engine.parse('collection:"Rust work"').ast, 'collection')).toBe(true);
    expect(engine.hasField(engine.parse('is:dirty').ast, 'collection')).toBe(false);
  });

  it('lets a test override one method without restating the rest', () => {
    const rows = [makeProjectRow()];
    const engine = fakeEngine({ filter: () => rows });
    expect(engine.filter(engine.parse('anything').ast, [])).toBe(rows);
    expect(engine.grammarVersion).toBeGreaterThan(0);
  });

  // R13 deleted the local `FakeAst` and its casts. The double builds plan 13's own `QueryAst`,
  // so a test cannot be structurally valid against a type the product does not have.
  it('builds a real QueryAst, not a shape that only looks like one', () => {
    const ast = fakeEngine().parse('-lang:rust is:dirty').ast;
    expect(ast.grammarVersion).toBeGreaterThan(0);
    expect(ast.ignored).toEqual([]);
    expect(ast.terms.map((t) => t.kind)).toEqual(['bare', 'bare']);
    expect(ast.terms.map((t) => t.negated)).toEqual([true, false]);
  });
});
