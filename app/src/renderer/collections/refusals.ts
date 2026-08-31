import type { Collection, CollectionRefusal } from '../../generated/protocol.js';
import { collectionActivationQuery } from './chipModel.js';
import type { QueryEngine } from './engine.js';

/** §8.8: the chip has a fixed label line. */
export const COLLECTION_NAME_MAX = 48;

/**
 * §8.8: composed, not measured — the count that cannot outgrow two wrapped lines of §8.0b's row
 * at the minimum window width, which §8.0a leaves measured rather than asserted. When that
 * measurement lands the cap is re-derived from it.
 */
export const COLLECTION_LIMIT = 16;

export const COLLECTION_TERM_FIELD = 'collection';

export const SAVE_CONTROL_TEXT = 'SAVE';

/** §8.8's refusal table. Each says which refusal fired; a bare error would lose that. */
export const REFUSAL_CONTROL_TEXT: Readonly<Record<CollectionRefusal, string>> = {
  empty_name: 'NAME IT',
  name_taken: 'THAT NAME IS TAKEN',
  too_long: 'SHORTER',
  contains_collection_term: 'NO COLLECTIONS INSIDE A COLLECTION',
  limit_reached: '16 IS THE LIMIT',
};

export interface SaveAttempt {
  readonly name: string;
  readonly queryText: string;
  readonly existing: readonly Collection[];
}

/**
 * §8.8's five refusals, in the order its table states them. The core refuses again — it is the
 * writer and has the last word — and this is what the control reads while the user is typing.
 */
export function refuseSave(attempt: SaveAttempt, engine: QueryEngine): CollectionRefusal | null {
  const name = attempt.name.trim();
  if (name === '') return 'empty_name';
  if (attempt.existing.some((c) => c.name.toLowerCase() === name.toLowerCase())) {
    return 'name_taken';
  }
  if (name.length > COLLECTION_NAME_MAX || /[\r\n]/.test(attempt.name)) return 'too_long';
  if (engine.hasField(engine.parse(attempt.queryText).ast, COLLECTION_TERM_FIELD)) {
    return 'contains_collection_term';
  }
  if (attempt.existing.length >= COLLECTION_LIMIT) return 'limit_reached';
  return null;
}

/**
 * §8.8: the proposal is generated from the AST and never from the raw string, so a default name
 * can never describe a query the parser did not build.
 */
export function proposedName(queryText: string, engine: QueryEngine): string {
  return engine.canonical(engine.parse(queryText).ast);
}

/**
 * §8.8: the control renders only when the effective query is non-empty and does not already equal
 * a saved collection — so the absence of the control is the explanation for `Ctrl+S` doing nothing.
 */
export function saveControlVisible(
  effectiveQueryText: string,
  existing: readonly Collection[],
  engine: QueryEngine,
): boolean {
  const parsed = engine.parse(effectiveQueryText);
  if (engine.termCount(parsed.ast) === 0) return false;
  const mine = engine.canonical(parsed.ast);
  if (mine === '') return false;
  return !existing.some((collection) => {
    const theirs = collectionActivationQuery(collection);
    return theirs !== null && engine.canonical(engine.parse(theirs).ast) === mine;
  });
}
