import type {
  CommandArgs,
  CommandName,
  CommandResult,
  ProjectRow,
} from '../../generated/protocol.js';
import type { QueryAst, QueryTerm } from '../../shared/query/ast.js';
import type { QueryField } from '../../shared/query/grammar.js';

/**
 * §8.3's AST is **plan 13's**, in both language mirrors (R13, R24). Collections never inspect a
 * term — they parse, canonicalise, count and ask whether one field is present — but that is a
 * reason not to *reach into* the type, not a licence to declare a second, opaque one. It is
 * re-exported here so `grammar.ts`, `chipModel.ts` and `upstream.ts` keep one import path.
 */
export type { QueryAst };

export interface DroppedTerm {
  /** The term as the user wrote it, for §8.3a's struck rendering. */
  readonly text: string;
  /** §8.3a's soft-error reason, e.g. `unknownField`, `notComputed`. */
  readonly reason: string;
}

export interface QueryParse {
  readonly ast: QueryAst;
  readonly dropped: readonly DroppedTerm[];
}

export interface QueryEngine {
  /** The grammar version this build speaks — §8.3's `query_grammar_version`. */
  readonly grammarVersion: number;
  parse(text: string): QueryParse;
  /** The canonical rendering of the AST — never a re-print of the raw string. */
  canonical(ast: QueryAst): string;
  /**
   * §8.3a's one function per count: the same filter the shelf and the built-in chips run, so a
   * collection's number can never disagree with the grid it activates.
   */
  filter(ast: QueryAst, rows: readonly ProjectRow[]): readonly ProjectRow[];
  /**
   * R43: plan 13's two readers of `QueryAst`, behind the seam. Both read `ast.terms` only —
   * a term the parser dropped into `ast.ignored` never ran, so it counts nothing and constrains
   * nothing, and a zero count is what §8.8 calls a query that no longer parses.
   */
  termCount(ast: QueryAst): number;
  hasField(ast: QueryAst, field: QueryField): boolean;
}

export interface CoreRpc {
  request<K extends CommandName>(name: K, args: CommandArgs[K]): Promise<CommandResult[K]>;
}

/**
 * A deliberately literal stand-in: text is whitespace-split and never interpreted. R13 removed the
 * local `FakeAst` and its casts — the fake now builds a **real** `QueryAst` out of plan 13's own
 * `bare` term, so a test double cannot be structurally valid against a type the product does not
 * have. `bareText` is the only place this plan reads a term's inside, and it reads one variant.
 */
function bareText(term: QueryTerm): string {
  return term.kind === 'bare' ? term.text : '';
}

export function fakeEngine(overrides: Partial<QueryEngine> = {}): QueryEngine {
  const base: QueryEngine = {
    grammarVersion: 1,
    parse: (text) => ({
      ast: {
        grammarVersion: 1,
        terms: (text.trim() === '' ? [] : text.trim().split(/\s+/)).map((token): QueryTerm => ({
          kind: 'bare',
          negated: token.startsWith('-'),
          text: token,
        })),
        ignored: [],
      },
      dropped: [],
    }),
    canonical: (ast) => ast.terms.map(bareText).join(' '),
    filter: (_ast, rows) => rows,
    termCount: (ast) => ast.terms.length,
    hasField: (ast, field) =>
      ast.terms.some((t) => bareText(t).replace(/^-/, '').startsWith(`${field}:`)),
  };
  return { ...base, ...overrides };
}
