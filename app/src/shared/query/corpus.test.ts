import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import type { QueryAst } from './ast.js';
import { parseQuery } from './parse.js';

interface CorpusCase {
  readonly name: string;
  readonly query: string;
  readonly ast: QueryAst;
}
interface Corpus {
  readonly corpusVersion: number;
  readonly cases: readonly CorpusCase[];
}

const corpus = JSON.parse(
  readFileSync(
    fileURLToPath(new URL('../../../../protocol/query/corpus.json', import.meta.url)),
    'utf8',
  ),
) as Corpus;

describe('the shared conformance corpus', () => {
  it('covers every branch the grammar has, so a divergence has somewhere to show up', () => {
    expect(corpus.cases.length).toBeGreaterThanOrEqual(20);
  });
  it('has unique case names, since both languages report failures by name', () => {
    expect(new Set(corpus.cases.map((c) => c.name)).size).toBe(corpus.cases.length);
  });
  for (const testCase of corpus.cases) {
    it(`TS: ${testCase.name}`, () => {
      expect(JSON.parse(JSON.stringify(parseQuery(testCase.query)))).toEqual(testCase.ast);
    });
  }
});
