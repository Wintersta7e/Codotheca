import { describe, expect, it } from 'vitest';
import { queryHasField, queryTermCount } from './ast.js';
import { parseQuery } from './parse.js';

describe('parseQuery', () => {
  it('returns an empty AST for an empty query', () => {
    expect(parseQuery('   ')).toEqual({ grammarVersion: 1, terms: [], ignored: [] });
  });

  it('parses a bare word as free text, lowercased', () => {
    expect(parseQuery('Codo').terms).toEqual([{ kind: 'bare', negated: false, text: 'codo' }]);
  });

  it('negates with a single leading dash and keeps the field', () => {
    // The design tokenizer pilled `-is:dirty` whole and yielded the field `-is`, which matches
    // no branch, so a negated term silently filtered nothing (§8.3a).
    expect(parseQuery('-is:dirty').terms).toEqual([{ kind: 'flag', negated: true, flag: 'dirty' }]);
  });

  it('parses a quoted path value without splitting it', () => {
    expect(parseQuery('in:"two words"').terms).toEqual([
      { kind: 'text', negated: false, field: 'in', value: 'two words', quoted: true },
    ]);
  });

  it('normalises size units to bytes', () => {
    expect(parseQuery('size:>10mb').terms).toEqual([
      { kind: 'size', negated: false, op: 'gt', bytes: 10485760 },
    ]);
  });

  it('normalises touched units to days', () => {
    expect(parseQuery('touched:>6mo').terms).toEqual([
      { kind: 'touchedAge', negated: false, op: 'gt', days: 180 },
    ]);
  });

  it('parses a bare four-digit year on touched', () => {
    expect(parseQuery('touched:2019').terms).toEqual([
      { kind: 'touchedYear', negated: false, year: 2019 },
    ]);
  });

  it('soft-errors an unknown field and keeps the rest of the query running', () => {
    const ast = parseQuery('nosuch:value is:dirty');
    expect(ast.terms).toEqual([{ kind: 'flag', negated: false, flag: 'dirty' }]);
    expect(ast.ignored).toEqual([{ text: 'nosuch:value', reason: 'unknownField' }]);
  });

  it('drops completion from the effective query and never evaluates it', () => {
    // §8.3a: matching nothing kills the term for every phase-1 user; coercing NULL to 0 renders
    // unknown as zero inside the filter engine.
    const ast = parseQuery('completion:>5 lang:rust');
    expect(ast.terms).toEqual([
      { kind: 'text', negated: false, field: 'lang', value: 'rust', quoted: false },
    ]);
    expect(ast.ignored).toEqual([{ text: 'completion:>5', reason: 'notComputed' }]);
  });

  it("soft-errors a value that is not in the field's enum", () => {
    expect(parseQuery('is:sideways').ignored).toEqual([
      { text: 'is:sideways', reason: 'malformedValue' },
    ]);
  });

  it('soft-errors an empty value', () => {
    expect(parseQuery('lang:').ignored).toEqual([{ text: 'lang:', reason: 'malformedValue' }]);
  });

  it('treats a colon-bearing word that is not a field as free text', () => {
    // §8.3: a bare word containing a colon that is not a known field is free text. A URL and a
    // Windows drive letter are the two shapes that reach a query field by accident.
    expect(parseQuery('http://example.invalid').terms).toEqual([
      { kind: 'bare', negated: false, text: 'http://example.invalid' },
    ]);
    expect(parseQuery('C:\\work').terms).toEqual([
      { kind: 'bare', negated: false, text: 'c:\\work' },
    ]);
  });

  it('lowercases an unquoted field value and preserves a quoted one', () => {
    expect(parseQuery('owner:Someone').terms).toEqual([
      { kind: 'text', negated: false, field: 'owner', value: 'someone', quoted: false },
    ]);
    expect(parseQuery('in:"Mixed Case"').terms).toEqual([
      { kind: 'text', negated: false, field: 'in', value: 'Mixed Case', quoted: true },
    ]);
  });

  it('ANDs terms and has no OR', () => {
    expect(parseQuery('lang:rust is:dirty touched:>6mo').terms).toHaveLength(3);
    expect(parseQuery('lang:rust OR is:dirty').terms.map((t) => t.kind)).toEqual([
      'text',
      'bare',
      'flag',
    ]);
  });

  it('stamps the grammar version', () => {
    expect(parseQuery('is:dirty').grammarVersion).toBe(1);
  });
});

// R43: the two readers plan 15's `QueryEngine` is written against.
describe('queryTermCount', () => {
  it('counts the terms that ran', () => {
    expect(queryTermCount(parseQuery('is:dirty lang:rust'))).toBe(2);
  });

  it('is zero for an empty query', () => {
    expect(queryTermCount(parseQuery('   '))).toBe(0);
  });

  it('counts bare free text — it filters, so it is a term', () => {
    expect(queryTermCount(parseQuery('codo lang:rust'))).toBe(2);
  });

  it('does not count ignored terms, so it agrees with effectiveQueryText', () => {
    // §8.3a: `terms` is what ran, `ignored` is text that never became a term. A query whose
    // every term was dropped counts zero — which is what makes plan 15 call it broken.
    expect(queryTermCount(parseQuery('completion:>5'))).toBe(0);
    expect(queryTermCount(parseQuery('completion:>5 nosuch:x lang:rust'))).toBe(1);
  });
});

describe('queryHasField', () => {
  it('answers for each field a term can carry', () => {
    expect(queryHasField(parseQuery('lang:rust'), 'lang')).toBe(true);
    expect(queryHasField(parseQuery('owner:someone'), 'owner')).toBe(true);
    expect(queryHasField(parseQuery('in:"two words"'), 'in')).toBe(true);
    expect(queryHasField(parseQuery('collection:"side projects"'), 'collection')).toBe(true);
    expect(queryHasField(parseQuery('is:dirty'), 'is')).toBe(true);
    expect(queryHasField(parseQuery('has:stash'), 'has')).toBe(true);
    expect(queryHasField(parseQuery('size:>10mb'), 'size')).toBe(true);
    expect(queryHasField(parseQuery('touched:>6mo'), 'touched')).toBe(true);
    expect(queryHasField(parseQuery('touched:2019'), 'touched')).toBe(true);
  });

  it('is false for a field the query does not constrain', () => {
    expect(queryHasField(parseQuery('is:dirty'), 'collection')).toBe(false);
    expect(queryHasField(parseQuery(''), 'lang')).toBe(false);
  });

  it('counts a negated term as constraining the field', () => {
    expect(queryHasField(parseQuery('-is:dirty'), 'is')).toBe(true);
  });

  it('does not read bare free text as a field', () => {
    expect(queryHasField(parseQuery('lang:rust'), 'owner')).toBe(false);
    expect(queryHasField(parseQuery('C:\\work'), 'collection')).toBe(false);
  });

  it('is false for completion, which never becomes a term in phase 1', () => {
    // The parser emits an `ignored` entry with reason `notComputed`, never a term. A dropped
    // term constrains nothing, so the honest answer is false.
    expect(queryHasField(parseQuery('completion:>5'), 'completion')).toBe(false);
  });
});
