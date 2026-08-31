import type { Collection } from '../../generated/protocol.js';
import type { DroppedTerm, QueryAst, QueryEngine } from './engine.js';

export type BrokenReason = 'no_longer_parses' | 'future_grammar';

export type CollectionQueryHealth =
  | { readonly kind: 'ok'; readonly ast: QueryAst }
  | {
      readonly kind: 'degraded';
      readonly ast: QueryAst;
      readonly dropped: readonly DroppedTerm[];
    }
  | { readonly kind: 'broken'; readonly reason: BrokenReason };

/** §8.8's sub-line for a broken chip. `--text-3`; the caller supplies the ink. */
export const BROKEN_REASON_TEXT: Readonly<Record<BrokenReason, (queryText: string) => string>> = {
  no_longer_parses: (queryText) => `NO LONGER PARSES · ${queryText}`,
  future_grammar: () => 'WRITTEN FOR A LATER VERSION OF THE QUERY LANGUAGE',
};

/**
 * §8.8's four-row table. `query_grammar_version` is evidence of when the query was written and
 * is never rewritten here — a successful parse against a newer grammar changes nothing on disk.
 */
export function collectionQueryHealth(
  collection: Collection,
  engine: QueryEngine,
): CollectionQueryHealth {
  if (collection.queryGrammarVersion > engine.grammarVersion) {
    return { kind: 'broken', reason: 'future_grammar' };
  }
  const text = collection.queryText;
  if (text === null || text.trim() === '') {
    return { kind: 'broken', reason: 'no_longer_parses' };
  }
  const parsed = engine.parse(text);
  if (engine.termCount(parsed.ast) === 0) {
    return { kind: 'broken', reason: 'no_longer_parses' };
  }
  if (parsed.dropped.length > 0) {
    return { kind: 'degraded', ast: parsed.ast, dropped: parsed.dropped };
  }
  return { kind: 'ok', ast: parsed.ast };
}
