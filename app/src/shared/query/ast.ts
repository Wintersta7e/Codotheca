import type { HasAttribute, IsFlag, QueryField } from './grammar.js';

export type Cmp = 'lt' | 'gt';
export type TextField = 'lang' | 'owner' | 'in' | 'collection';

/** `notAvailable` is renderer-only: the projection cannot answer the term. The parser never
 *  emits it, so it never appears in the shared corpus and the Rust enum carries the other three. */
export type IgnoredReason = 'unknownField' | 'notComputed' | 'malformedValue' | 'notAvailable';

export interface IgnoredTerm {
  readonly text: string;
  readonly reason: IgnoredReason;
}

export type QueryTerm =
  | { readonly kind: 'bare'; readonly negated: boolean; readonly text: string }
  | {
      readonly kind: 'text';
      readonly negated: boolean;
      readonly field: TextField;
      readonly value: string;
      readonly quoted: boolean;
    }
  | { readonly kind: 'flag'; readonly negated: boolean; readonly flag: IsFlag }
  | { readonly kind: 'has'; readonly negated: boolean; readonly attribute: HasAttribute }
  | { readonly kind: 'size'; readonly negated: boolean; readonly op: Cmp; readonly bytes: number }
  | {
      readonly kind: 'touchedAge';
      readonly negated: boolean;
      readonly op: Cmp;
      readonly days: number;
    }
  | { readonly kind: 'touchedYear'; readonly negated: boolean; readonly year: number };

export interface QueryAst {
  readonly grammarVersion: number;
  readonly terms: readonly QueryTerm[];
  readonly ignored: readonly IgnoredTerm[];
}

export const EMPTY_AST: QueryAst = { grammarVersion: 1, terms: [], ignored: [] };

/**
 * The field a term constrains, or `null` for bare free text, which constrains none. `completion`
 * is never returned: §8.3a drops it into `ignored`, so no `completion` term exists in phase 1.
 */
function fieldOf(term: QueryTerm): QueryField | null {
  switch (term.kind) {
    case 'bare':
      return null;
    case 'text':
      return term.field;
    case 'flag':
      return 'is';
    case 'has':
      return 'has';
    case 'size':
      return 'size';
    case 'touchedAge':
    case 'touchedYear':
      return 'touched';
  }
}

/**
 * R43: how many terms the query carries. **`ast.terms` only** — `ignored` is text that never
 * became a term, so counting it would disagree with `effectiveQueryText`, which prints `terms`
 * and nothing else.
 */
export function queryTermCount(ast: QueryAst): number {
  return ast.terms.length;
}

/** R43: whether the query constrains `field`. Reads `ast.terms` only, for the same reason. */
export function queryHasField(ast: QueryAst, field: QueryField): boolean {
  return ast.terms.some((term) => fieldOf(term) === field);
}
