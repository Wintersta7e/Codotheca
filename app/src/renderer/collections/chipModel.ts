import type {
  Collection,
  CollectionId,
  CollectionKind,
  ProjectRow,
} from '../../generated/protocol.js';
import type { QueryEngine } from './engine.js';
import { BROKEN_REASON_TEXT, collectionQueryHealth } from './grammar.js';

/**
 * §8.8: the accent is `--text-2`, deliberately the neutral `ALL` already resolves to. The
 * built-in accents divide by meaning — `--sig` is state the app observed, amber is age, neutral
 * is scope — and a saved query is a scope. `--sig` is wrong twice over: §8.7 reserves it for
 * *system*, and a query the user wrote is not system.
 */
export const COLLECTION_ACCENT = 'var(--text-2)';

export const MANUAL_SUB_LINE = 'CHOSEN BY HAND';

export type CollectionSubLine =
  | { readonly kind: 'text'; readonly text: string }
  | { readonly kind: 'terms'; readonly ran: string; readonly dropped: readonly string[] };

export interface CollectionChipModel {
  readonly id: CollectionId;
  /** The saved name in its own case, for the accessible name. */
  readonly name: string;
  /** The saved name upper-cased, for the chip label. */
  readonly label: string;
  readonly kind: CollectionKind;
  readonly state: 'normal' | 'degraded' | 'broken';
  /** `null` means nothing was counted. It is never `0` standing in for that. */
  readonly count: number | null;
  readonly subLine: CollectionSubLine;
  readonly activatable: boolean;
  readonly activationQuery: string | null;
  readonly accessibleName: string;
}

/**
 * Quoted unless the bare form round-trips — and "needs quoting" is the parser's answer, not a
 * guess. §8.3's tokenizer **lower-cases an unquoted value** and preserves a quoted one (measured:
 * `collection:Weekend` parses to `value: 'weekend'`, `collection:"Two Words"` keeps its case), so
 * a name carrying an upper-case letter must be quoted or the field would show a name the user
 * never typed while the chip's label shouts the one they did. A colon or whitespace would end the
 * value early, and a quote has to be escaped.
 */
function quoteName(name: string): string {
  const roundTripsBare = name === name.toLowerCase() && !/["\s:]/.test(name);
  if (roundTripsBare) return name;
  return `"${name.replace(/"/g, '\\"')}"`;
}

/** §8.8: activating writes the collection's query into the query field. */
export function collectionActivationQuery(collection: Collection): string | null {
  if (collection.kind === 'manual') return `collection:${quoteName(collection.name)}`;
  return collection.queryText;
}

function plural(n: number, one: string, many: string): string {
  return `${String(n)} ${n === 1 ? one : many}`;
}

export function collectionChipModel(
  collection: Collection,
  rows: readonly ProjectRow[],
  engine: QueryEngine,
): CollectionChipModel {
  const label = collection.name.toUpperCase();
  const shared = { id: collection.id, name: collection.name, label, kind: collection.kind };

  if (collection.kind === 'manual') {
    // A membership list, immune to a project's facts changing: no predicate, no re-evaluation.
    const count = rows.filter((row) => row.collectionIds.includes(collection.id)).length;
    return {
      ...shared,
      state: 'normal',
      count,
      subLine: { kind: 'text', text: MANUAL_SUB_LINE },
      activatable: true,
      activationQuery: collectionActivationQuery(collection),
      accessibleName: `${collection.name}, ${plural(count, 'project', 'projects')}`,
    };
  }

  const health = collectionQueryHealth(collection, engine);
  if (health.kind === 'broken') {
    return {
      ...shared,
      state: 'broken',
      count: null,
      subLine: {
        kind: 'text',
        text: BROKEN_REASON_TEXT[health.reason](collection.queryText ?? ''),
      },
      activatable: false,
      activationQuery: null,
      // §8.8 states this name verbatim, for both broken reasons. It carries `not counted`
      // rather than a figure, which is *never render unknown as zero* in the a11y tree.
      accessibleName: `${collection.name}, not counted, query no longer parses`,
    };
  }

  const count = engine.filter(health.ast, rows).length;
  const counted = plural(count, 'project', 'projects');

  if (health.kind === 'degraded') {
    return {
      ...shared,
      state: 'degraded',
      count,
      subLine: {
        kind: 'terms',
        ran: engine.canonical(health.ast),
        dropped: health.dropped.map((term) => term.text),
      },
      activatable: true,
      activationQuery: engine.canonical(health.ast),
      accessibleName: `${collection.name}, ${counted}, ${plural(
        health.dropped.length,
        'term no longer parses',
        'terms no longer parse',
      )}`,
    };
  }

  return {
    ...shared,
    state: 'normal',
    count,
    // Verbatim and in its own case: §8.3 makes quoted paths case-sensitive on Linux, so a
    // shouted `in:"Some Dir"` is a different term.
    subLine: { kind: 'text', text: collection.queryText ?? '' },
    activatable: true,
    activationQuery: collection.queryText,
    accessibleName: `${collection.name}, ${counted}`,
  };
}

/**
 * §8.8: the chip unlights the moment the field stops matching, compared over the canonical AST —
 * which is what keeps two orderings of the same terms from reading as two different scopes.
 */
export function isCollectionActive(
  model: CollectionChipModel,
  currentQuery: string,
  engine: QueryEngine,
): boolean {
  if (model.activationQuery === null) return false;
  const mine = engine.canonical(engine.parse(model.activationQuery).ast);
  const theirs = engine.canonical(engine.parse(currentQuery).ast);
  return mine !== '' && mine === theirs;
}
