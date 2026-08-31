import { describe, expect, it } from 'vitest';
import type { Collection, CollectionId, ProjectId } from '../../generated/protocol.js';
import type { QueryAst, QueryTerm } from '../../shared/query/ast.js';
import { parseQuery } from '../../shared/query/parse.js';
import { makeProjectRow } from '../testing/projectRow.js';
import {
  COLLECTION_ACCENT,
  MANUAL_SUB_LINE,
  collectionActivationQuery,
  collectionChipModel,
  isCollectionActive,
} from './chipModel.js';
import { fakeEngine } from './engine.js';

const query = (over: Partial<Collection> = {}): Collection => ({
  id: 7 as CollectionId,
  name: 'Rust work',
  kind: 'query',
  queryText: 'lang:rust is:dirty',
  queryGrammarVersion: 1,
  sortIndex: 3,
  memberCount: null,
  ...over,
});

const rows = [
  makeProjectRow({ id: 1 as ProjectId, collectionIds: [7 as CollectionId] }),
  makeProjectRow({ id: 2 as ProjectId, collectionIds: [] }),
  makeProjectRow({ id: 3 as ProjectId, collectionIds: [7 as CollectionId] }),
];

// R13: plan 13's own `QueryAst`, built rather than cast.
const ast = (...texts: readonly string[]): QueryAst => ({
  grammarVersion: 1,
  terms: texts.map((text): QueryTerm => ({ kind: 'bare', negated: false, text })),
  ignored: [],
});

const textOf = (term: QueryTerm): string => (term.kind === 'bare' ? term.text : '');

describe('collectionChipModel — query kind', () => {
  it('counts through the one filter, labels upper and states the query in its own case', () => {
    const engine = fakeEngine({ filter: (_ast, all) => all.slice(0, 2) });
    const model = collectionChipModel(query(), rows, engine);
    expect(model.state).toBe('normal');
    expect(model.count).toBe(2);
    expect(model.label).toBe('RUST WORK');
    expect(model.name).toBe('Rust work');
    expect(model.subLine).toEqual({ kind: 'text', text: 'lang:rust is:dirty' });
    expect(model.accessibleName).toBe('Rust work, 2 projects');
    expect(COLLECTION_ACCENT).toBe('var(--text-2)');
  });

  // A measured zero is a fact. It is never dimmed into looking like an absent measurement.
  it('renders a measured zero as a number', () => {
    const engine = fakeEngine({ filter: () => [] });
    const model = collectionChipModel(query(), rows, engine);
    expect(model.count).toBe(0);
    expect(model.accessibleName).toBe('Rust work, 0 projects');
  });

  it('says one project, not one projects', () => {
    const engine = fakeEngine({ filter: (_a, all) => all.slice(0, 1) });
    expect(collectionChipModel(query(), rows, engine).accessibleName).toBe('Rust work, 1 project');
  });

  it('degraded runs the survivors, strikes the dropped and attributes the count to what ran', () => {
    const engine = fakeEngine({
      parse: () => ({
        ast: ast('lang:rust'),
        dropped: [{ text: 'sparkle:yes', reason: 'unknownField' }],
      }),
      canonical: () => 'lang:rust',
      filter: (_a, all) => all.slice(0, 2),
    });
    const model = collectionChipModel(query(), rows, engine);
    expect(model.state).toBe('degraded');
    expect(model.count).toBe(2);
    expect(model.subLine).toEqual({ kind: 'terms', ran: 'lang:rust', dropped: ['sparkle:yes'] });
    expect(model.activatable).toBe(true);
    expect(model.accessibleName).toBe('Rust work, 2 projects, 1 term no longer parses');
  });

  it('says two terms no longer parse, in the plural §8.8 writes', () => {
    const engine = fakeEngine({
      parse: () => ({
        ast: ast('lang:rust'),
        dropped: [
          { text: 'sparkle:yes', reason: 'unknownField' },
          { text: 'glitter:no', reason: 'unknownField' },
        ],
      }),
      canonical: () => 'lang:rust',
      filter: (_a, all) => all.slice(0, 2),
    });
    expect(collectionChipModel(query(), rows, engine).accessibleName).toBe(
      'Rust work, 2 projects, 2 terms no longer parse',
    );
  });

  // §8.8: broken renders no number at all — not 0, not an em dash, not a dimmed digit.
  it('broken carries no count and is not activatable', () => {
    const engine = fakeEngine({ grammarVersion: 1 });
    const model = collectionChipModel(query({ queryGrammarVersion: 9 }), rows, engine);
    expect(model.state).toBe('broken');
    expect(model.count).toBeNull();
    expect(model.activatable).toBe(false);
    expect(model.activationQuery).toBeNull();
    expect(model.subLine).toEqual({
      kind: 'text',
      text: 'WRITTEN FOR A LATER VERSION OF THE QUERY LANGUAGE',
    });
    expect(model.accessibleName).toBe('Rust work, not counted, query no longer parses');
  });

  // A broken chip must not be counted either: `engine.filter` is never reached for one.
  it('does not count a broken collection at all', () => {
    let filtered = 0;
    const engine = fakeEngine({
      grammarVersion: 1,
      filter: (_a, all) => {
        filtered += 1;
        return all;
      },
    });
    collectionChipModel(query({ queryGrammarVersion: 9 }), rows, engine);
    expect(filtered).toBe(0);
  });
});

describe('collectionChipModel — manual kind', () => {
  const manual = query({ kind: 'manual', queryText: null, name: 'Weekend' });

  it('counts membership over the same projection and never re-evaluates a predicate', () => {
    let filtered = 0;
    const engine = fakeEngine({
      filter: (_a, all) => {
        filtered += 1;
        return all;
      },
    });
    const model = collectionChipModel(manual, rows, engine);
    expect(model.count).toBe(2);
    expect(filtered).toBe(0);
    expect(model.subLine).toEqual({ kind: 'text', text: MANUAL_SUB_LINE });
    expect(model.state).toBe('normal');
    expect(model.activationQuery).toBe('collection:"Weekend"');
  });
});

describe('isCollectionActive', () => {
  it('compares over the canonical AST, so term order does not unlight the chip', () => {
    const engine = fakeEngine({
      canonical: (a) => [...a.terms].map(textOf).sort().join(' '),
    });
    const model = collectionChipModel(query(), rows, engine);
    expect(isCollectionActive(model, 'is:dirty lang:rust', engine)).toBe(true);
    expect(isCollectionActive(model, 'lang:rust', engine)).toBe(false);
  });

  it('a broken chip is never active', () => {
    const engine = fakeEngine({ grammarVersion: 1 });
    const model = collectionChipModel(query({ queryGrammarVersion: 9 }), rows, engine);
    expect(isCollectionActive(model, 'lang:rust is:dirty', engine)).toBe(false);
  });

  // An empty field is not "the same scope" as every collection that failed to canonicalise.
  it('an empty query lights nothing', () => {
    const engine = fakeEngine();
    const model = collectionChipModel(query({ queryText: '  ' }), rows, engine);
    expect(isCollectionActive(model, '', engine)).toBe(false);
  });
});

describe('collectionActivationQuery', () => {
  // The rule is the parser's, not a guess: §8.3's tokenizer lower-cases an unquoted value, so a
  // name with an upper-case letter does not survive the bare form. This drives the **real**
  // parser, which is what keeps the quoting rule from being one developer's opinion.
  it('produces a term the real parser reads back as the name that was saved', () => {
    for (const name of ['weekend', 'Weekend', 'Two Words', 'say "hi"', 'a:b']) {
      const text = collectionActivationQuery(query({ kind: 'manual', name }));
      if (text === null) throw new Error('a manual collection always has an activation query');
      const ast = parseQuery(text);
      expect(ast.ignored, name).toEqual([]);
      expect(ast.terms, name).toHaveLength(1);
      const term = ast.terms[0];
      expect(term?.kind === 'text' ? term.field : null, name).toBe('collection');
      expect(term?.kind === 'text' ? term.value : null, name).toBe(name);
    }
  });

  it('quotes a manual name only when it needs quoting', () => {
    expect(collectionActivationQuery(query({ kind: 'manual', name: 'weekend' }))).toBe(
      'collection:weekend',
    );
    expect(collectionActivationQuery(query({ kind: 'manual', name: 'Two Words' }))).toBe(
      'collection:"Two Words"',
    );
    expect(collectionActivationQuery(query({ kind: 'manual', name: 'say "hi"' }))).toBe(
      'collection:"say \\"hi\\""',
    );
  });
});
