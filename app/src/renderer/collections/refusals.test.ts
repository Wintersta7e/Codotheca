import { describe, expect, it } from 'vitest';
import type { Collection, CollectionId } from '../../generated/protocol.js';
import type { QueryAst } from '../../shared/query/ast.js';
import { fakeEngine } from './engine.js';
import {
  COLLECTION_LIMIT,
  COLLECTION_NAME_MAX,
  REFUSAL_CONTROL_TEXT,
  SAVE_CONTROL_TEXT,
  proposedName,
  refuseSave,
  saveControlVisible,
} from './refusals.js';
import type { SaveAttempt } from './refusals.js';

const existing = (names: readonly string[]): Collection[] =>
  names.map((name, i) => ({
    id: (i + 1) as CollectionId,
    name,
    kind: 'query',
    queryText: `saved:${name}`,
    queryGrammarVersion: 1,
    sortIndex: i,
    memberCount: null,
  }));

const attempt = (over: Partial<SaveAttempt> = {}): SaveAttempt => ({
  name: 'Rust work',
  queryText: 'lang:rust is:dirty',
  existing: [] as readonly Collection[],
  ...over,
});

const emptyAst: QueryAst = { grammarVersion: 1, terms: [], ignored: [] };

describe('refuseSave', () => {
  it('accepts a good name over a good query', () => {
    expect(refuseSave(attempt(), fakeEngine())).toBeNull();
  });

  it('refuses an empty name, because a nameless chip cannot be told from another', () => {
    expect(refuseSave(attempt({ name: '   ' }), fakeEngine())).toBe('empty_name');
    expect(REFUSAL_CONTROL_TEXT.empty_name).toBe('NAME IT');
  });

  it('refuses a taken name case-insensitively', () => {
    expect(refuseSave(attempt({ existing: existing(['rust WORK']) }), fakeEngine())).toBe(
      'name_taken',
    );
    expect(REFUSAL_CONTROL_TEXT.name_taken).toBe('THAT NAME IS TAKEN');
  });

  it('refuses a name over 48 characters or containing a newline', () => {
    expect(refuseSave(attempt({ name: 'x'.repeat(49) }), fakeEngine())).toBe('too_long');
    expect(refuseSave(attempt({ name: 'x'.repeat(48) }), fakeEngine())).toBeNull();
    expect(refuseSave(attempt({ name: 'two\nlines' }), fakeEngine())).toBe('too_long');
    expect(REFUSAL_CONTROL_TEXT.too_long).toBe('SHORTER');
    expect(COLLECTION_NAME_MAX).toBe(48);
  });

  it('refuses a query that names another collection', () => {
    expect(refuseSave(attempt({ queryText: 'collection:"Rust work"' }), fakeEngine())).toBe(
      'contains_collection_term',
    );
    expect(REFUSAL_CONTROL_TEXT.contains_collection_term).toBe(
      'NO COLLECTIONS INSIDE A COLLECTION',
    );
  });

  it('refuses the seventeenth collection', () => {
    const sixteen = existing(Array.from({ length: 16 }, (_, i) => `c${String(i)}`));
    expect(refuseSave(attempt({ existing: sixteen }), fakeEngine())).toBe('limit_reached');
    expect(REFUSAL_CONTROL_TEXT.limit_reached).toBe('16 IS THE LIMIT');
    expect(COLLECTION_LIMIT).toBe(16);
  });

  // The refusal table is what the control reads; the core is the writer and refuses again.
  // Every §8.8 refusal has control text and no refusal has any other.
  it('has control text for every refusal the wire declares and for nothing else', () => {
    expect(Object.keys(REFUSAL_CONTROL_TEXT).sort()).toEqual([
      'contains_collection_term',
      'empty_name',
      'limit_reached',
      'name_taken',
      'too_long',
    ]);
  });
});

describe('proposedName', () => {
  it('is the canonical query rendered back from the AST, never the raw string', () => {
    const engine = fakeEngine({ canonical: () => 'lang:rust is:dirty' });
    expect(proposedName('  is:dirty    lang:rust ', engine)).toBe('lang:rust is:dirty');
  });
});

describe('saveControlVisible', () => {
  it('renders for a non-empty query with no chip of its own', () => {
    expect(saveControlVisible('lang:rust', [], fakeEngine())).toBe(true);
    expect(SAVE_CONTROL_TEXT).toBe('SAVE');
  });

  it('does not render for an empty or all-soft-errored query', () => {
    expect(saveControlVisible('', [], fakeEngine())).toBe(false);
    expect(saveControlVisible('   ', [], fakeEngine())).toBe(false);
    const allDropped = fakeEngine({
      parse: () => ({ ast: emptyAst, dropped: [{ text: 'nope:1', reason: 'unknownField' }] }),
    });
    expect(saveControlVisible('nope:1', [], allDropped)).toBe(false);
  });

  // Neither path can mint a second chip for a query that already has one.
  it('does not render when a saved collection already equals the query', () => {
    const saved = existing(['Rust work']);
    const engine = fakeEngine({ canonical: () => 'same' });
    expect(saveControlVisible('lang:rust', saved, engine)).toBe(false);
  });

  // A broken saved collection has no activation query at all, so it cannot claim the field's
  // query and hide the control that would let the user save it under a working name.
  it('a saved collection with no query of its own claims nothing', () => {
    const broken = existing(['Broken']).map((c) => ({ ...c, queryText: null }));
    expect(saveControlVisible('lang:rust', broken, fakeEngine())).toBe(true);
  });
});
