import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  HAS_ATTRIBUTES,
  IS_FLAGS,
  NEVER_EVALUATED,
  QUERY_FIELDS,
  QUERY_GRAMMAR_VERSION,
  SIZE_UNIT_BYTES,
  TOUCHED_UNIT_DAYS,
} from './grammar.js';

const production: unknown = JSON.parse(
  readFileSync(
    fileURLToPath(new URL('../../../../protocol/query/grammar.json', import.meta.url)),
    'utf8',
  ),
) as unknown;

function at(key: string): unknown {
  return (production as Record<string, unknown>)[key];
}

describe('the TypeScript tables transcribe protocol/query/grammar.json', () => {
  it('agrees on the version', () => {
    expect(at('queryGrammarVersion')).toBe(QUERY_GRAMMAR_VERSION);
  });
  it('agrees on the field list, in order', () => {
    expect(at('fields')).toEqual([...QUERY_FIELDS]);
  });
  it('agrees on the is enum, in order', () => {
    expect(at('is')).toEqual([...IS_FLAGS]);
  });
  it('agrees on the has enum, in order', () => {
    expect(at('has')).toEqual([...HAS_ATTRIBUTES]);
  });
  it('agrees on the unit tables', () => {
    expect(at('sizeUnitBytes')).toEqual(SIZE_UNIT_BYTES);
    expect(at('touchedUnitDays')).toEqual(TOUCHED_UNIT_DAYS);
  });
  it('agrees on which fields parse but never evaluate', () => {
    expect(at('neverEvaluated')).toEqual([...NEVER_EVALUATED]);
  });
  it('keeps completion in the field list even though it never evaluates', () => {
    // §8.3a: removing a field would break every stored query that used it.
    expect(QUERY_FIELDS).toContain('completion');
  });
  it('offers no is:mine and no is:remote', () => {
    // §8.3a rejects both: is:mine duplicates -is:reference, is:remote is phase 2.
    expect(IS_FLAGS).not.toContain('mine');
    expect(IS_FLAGS).not.toContain('remote');
  });
});
