import type { ReactElement } from 'react';
import type { IgnoredTerm, QueryAst } from '../../shared/query/ast.js';
import type { Pill } from '../../shared/query/format.js';
import { pillsOf } from '../../shared/query/format.js';
import type { RawToken } from '../../shared/query/tokenize.js';
import { tokenizeQuery } from '../../shared/query/tokenize.js';

export const QUERY_PLACEHOLDER = 'lang:rust is:dirty touched:>6mo' as const;

export interface QueryFieldModel {
  readonly pills: readonly Pill[];
  readonly committed: string;
  readonly draft: string;
}

export interface QueryFieldProps {
  readonly text: string;
  /** The same AST `model` was built from. A pill carries no source span, so dropping one needs
   *  the AST to find the token the user typed; see `pillTokens`. */
  readonly ast: QueryAst;
  readonly model: QueryFieldModel;
  readonly onQueryChange: (text: string) => void;
}

/** A token whose body is empty — a lone `-` — produces neither a term nor an ignored entry. */
const isEmptyBody = (token: RawToken): boolean => token.text === '-';

interface PilledToken {
  readonly pill: Pill;
  readonly token: RawToken;
}

/**
 * Which token each pill came from.
 *
 * A `Pill` carries no source span: its `label` is a *re-rendering* of the term, so `LANG:Rust`
 * comes back as `lang:rust` and addressing a token by the label would silently miss. The parser
 * consumes tokens in order and emits exactly one artefact per token, and every ignored entry
 * carries its source text verbatim — so the ignored entries can be matched by that text and
 * whatever is left over is, in order, the terms. That is the whole alignment, and it needs no
 * second tokenizer and no second parse.
 *
 * `Pill.key` is treated as opaque throughout: the pairing is rebuilt by walking the AST in the
 * order `pillsOf` walks it, never by decoding the key's spelling.
 */
function pillTokens(
  tokens: readonly RawToken[],
  ast: QueryAst,
  extraIgnored: readonly IgnoredTerm[],
): readonly PilledToken[] {
  const termSources: (RawToken | undefined)[] = [];
  const ignoredSources: (RawToken | undefined)[] = [];
  let pendingIgnored = 0;
  for (const token of tokens) {
    if (isEmptyBody(token)) continue;
    if (ast.ignored[pendingIgnored]?.text === token.text) {
      ignoredSources[pendingIgnored] = token;
      pendingIgnored += 1;
    } else {
      termSources.push(token);
    }
  }

  const pills = pillsOf(ast, extraIgnored);
  const pairs: PilledToken[] = [];
  let index = 0;
  ast.terms.forEach((term, termIndex) => {
    // `pillsOf` renders no pill for a bare term: free text stays in the input.
    if (term.kind === 'bare') return;
    const token = termSources[termIndex];
    const pill = pills[index];
    index += 1;
    if (token !== undefined && pill !== undefined) pairs.push({ pill, token });
  });
  ast.ignored.forEach((_entry, ignoredIndex) => {
    const token = ignoredSources[ignoredIndex];
    const pill = pills[index];
    index += 1;
    if (token !== undefined && pill !== undefined) pairs.push({ pill, token });
  });
  // `extraIgnored` is the renderer's own "the projection cannot answer this" set; those pills
  // stand for a term that is already accounted for above and address no token of their own.
  return pairs;
}

/**
 * The field's two halves, from one AST and one pass of the one tokenizer.
 *
 * `draft` is what the input shows: every token that produced no pill (free text keeps the case
 * the user typed) plus the trailing token while it is still unterminated. `pills` is every
 * pill except that trailing one — §8.3a's *the pill text is never rendered twice*.
 */
export function fieldModel(
  text: string,
  ast: QueryAst,
  extraIgnored: readonly IgnoredTerm[],
): QueryFieldModel {
  const tokens = tokenizeQuery(text);
  const last = tokens.at(-1);
  // A token that ends where the text ends is still being typed: no space has terminated it.
  const typing = last?.end === text.length;

  const pairs = pillTokens(tokens, ast, extraIgnored);
  const allPills = pillsOf(ast, extraIgnored);
  const lifted = new Set(pairs.map((pair) => pair.token));
  const stillTyped = new Set(
    pairs.filter((pair) => typing && pair.token === last).map((pair) => pair.pill.key),
  );

  const pills = allPills.filter((pill) => !stillTyped.has(pill.key));
  const draftTokens = tokens.filter((token) => !lifted.has(token) || (typing && token === last));
  const draftSet = new Set(draftTokens);
  const committed = tokens
    .filter((token) => !draftSet.has(token))
    .map((token) => token.text)
    .join(' ');

  return { pills, committed, draft: draftTokens.map((token) => token.text).join(' ') };
}

export function joinQuery(committed: string, draft: string): string {
  if (committed.length === 0) return draft;
  if (draft.length === 0) return `${committed} `;
  return `${committed} ${draft}`;
}

/** Cuts the pill's own source span out of the query, so every other term survives as typed.
 *  A pill this query cannot place is a no-op: cutting the wrong span is worse than cutting
 *  none. */
export function dropPill(text: string, ast: QueryAst, pill: Pill): string {
  const pairs = pillTokens(tokenizeQuery(text), ast, []);
  // The key alone is positional, so a pill built from a different query can collide with one of
  // this query's. The label has to agree too, or the drop cuts a term the user did not click.
  const token = pairs.find(
    (pair) => pair.pill.key === pill.key && pair.pill.label === pill.label,
  )?.token;
  if (token === undefined) return text;
  return `${text.slice(0, token.start)}${text.slice(token.end)}`.replace(/\s{2,}/g, ' ').trim();
}

export function QueryFieldView(props: QueryFieldProps): ReactElement {
  const { model } = props;
  return (
    <div className="cdt-shelf-field">
      {model.pills.map((pill) => (
        <button
          key={pill.key}
          type="button"
          className={`cdt-shelf-pill${pill.state === 'error' ? ' is-struck' : ''}`}
          data-state={pill.state}
          title={pill.reason ?? undefined}
          onClick={() => {
            props.onQueryChange(dropPill(props.text, props.ast, pill));
          }}
        >
          <span className="cdt-shelf-pill-label">{pill.label}</span>
          <span className="cdt-shelf-pill-drop" aria-hidden="true">
            ×
          </span>
        </button>
      ))}
      <input
        type="search"
        className="cdt-shelf-field-input"
        value={model.draft}
        placeholder={QUERY_PLACEHOLDER}
        onChange={(event) => {
          props.onQueryChange(joinQuery(model.committed, event.currentTarget.value));
        }}
      />
    </div>
  );
}
