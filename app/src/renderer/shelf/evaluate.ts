import type { ProjectId } from '../../generated/protocol.js';
import { isNewArrival } from '../firstrun/newArrivals.js';
import type { IgnoredTerm, QueryAst, QueryTerm } from '../../shared/query/ast.js';
import { renderTerm } from '../../shared/query/format.js';
import type { ProjectionCapabilities, ShelfRow } from './row.js';

export type Truth = true | false | null;

export interface QueryContext {
  readonly now: number;
  /** `app_meta.first_run_completed_at`; `null` when the shell has not supplied it. */
  readonly firstRunCompletedAt: number | null;
  readonly collectionIdsByName: ReadonlyMap<string, number>;
  readonly pathsAreCaseSensitive: boolean;
  readonly capabilities: ProjectionCapabilities;
  /** Filled by the debounced commit-subject lane; `null` while no search is in flight. */
  readonly commitSubjectHits: ReadonlySet<ProjectId> | null;
}

export const BASE_PREDICATE_NOTE =
  'A bare query returns no is_reference rows and no is_hidden rows (§8.0b). ' +
  'is:reference and is:hidden opt their own rows back in.';

const DAY = 86_400;
const LOCATION_KEYWORDS = new Set(['local', 'wsl']);

function boolTruth(value: boolean | null | undefined): Truth {
  return value === null || value === undefined ? null : value;
}

function countTruth(value: number | null | undefined): Truth {
  return value === null || value === undefined ? null : value > 0;
}

function fold(needle: string, haystack: string | null, caseSensitive: boolean): boolean {
  if (haystack === null) return false;
  return caseSensitive ? haystack.includes(needle) : haystack.toLowerCase().includes(needle);
}

function localYear(epochSeconds: number): number {
  return new Date(epochSeconds * 1000).getFullYear();
}

export function termTruth(row: ShelfRow, term: QueryTerm, ctx: QueryContext): Truth {
  switch (term.kind) {
    case 'bare': {
      const needle = term.text;
      const fields = [
        row.name,
        row.owner,
        row.description,
        row.primaryLocation?.pathDisplay ?? null,
        row.lastCommitSubject,
      ];
      if (fields.some((field) => fold(needle, field, false))) return true;
      if (ctx.commitSubjectHits !== null && ctx.commitSubjectHits.has(row.id)) return true;
      return false;
    }
    case 'text':
      switch (term.field) {
        case 'lang':
          return row.primaryLanguage === null
            ? null
            : row.primaryLanguage.toLowerCase() === term.value.toLowerCase();
        case 'owner':
          return row.owner === null ? null : row.owner.toLowerCase() === term.value.toLowerCase();
        case 'collection': {
          const id = ctx.collectionIdsByName.get(term.value.toLowerCase());
          if (id === undefined) return false;
          return (row.collectionIds as readonly number[]).includes(id);
        }
        case 'in': {
          const wanted = term.value.toLowerCase();
          if (LOCATION_KEYWORDS.has(wanted) || wanted.startsWith('wsl:')) {
            if (row.locationKind === null) return null;
            if (wanted === 'local') return row.locationKind !== 'wsl';
            if (wanted === 'wsl') return row.locationKind === 'wsl';
            return (
              row.locationKind === 'wsl' && (row.distro ?? '').toLowerCase() === wanted.slice(4)
            );
          }
          const path = row.primaryLocation?.pathDisplay ?? null;
          if (path === null) return null;
          const caseSensitive = term.quoted && ctx.pathsAreCaseSensitive;
          return caseSensitive
            ? path.startsWith(term.value)
            : path.toLowerCase().startsWith(term.value.toLowerCase());
        }
      }
    case 'flag':
      switch (term.flag) {
        case 'dirty':
          return boolTruth(row.isDirty);
        case 'unpushed':
          return countTruth(row.ahead);
        case 'behind':
          return countTruth(row.behind);
        case 'interrupted':
          return row.refstateObservedAt === null ? null : row.interruptedOp !== null;
        case 'archived':
          return row.isArchived;
        case 'pinned':
          return row.isPinned;
        case 'hidden':
          return row.isHidden;
        case 'reference':
          return row.isReference;
        case 'bare':
          return row.isBare;
        case 'fork':
          return row.isFork;
        case 'empty':
          return row.conditionSignal === null ? null : row.conditionSignal === 'empty';
        case 'shallow':
          return row.isShallow;
        case 'local':
          return row.locationKind === null ? null : row.locationKind !== 'wsl';
        case 'wsl':
          return row.locationKind === null ? null : row.locationKind === 'wsl';
        case 'new': {
          // R18: one predicate. Returning `null` rather than `false` for an unstamped install
          // is this engine's own distinction — §8.3 drops a term it cannot answer instead of
          // rendering an empty grid — but what *counts* as new is not restated here.
          if (ctx.firstRunCompletedAt === null) return null;
          return isNewArrival(row, ctx.firstRunCompletedAt);
        }
      }
    case 'has':
      switch (term.attribute) {
        case 'stash':
          return countTruth(row.stashCount);
        case 'readme':
          return boolTruth(row.hasReadme);
        case 'license':
          return boolTruth(row.hasLicense);
        case 'tests':
          return boolTruth(row.hasTests);
        case 'ci':
          return boolTruth(row.hasCi);
        case 'remote':
          return boolTruth(row.hasRemote);
        case 'submodules':
          return boolTruth(row.hasSubmodules);
      }
    case 'size': {
      if (row.sizeTrackedBytes === null) return null;
      return term.op === 'gt'
        ? row.sizeTrackedBytes > term.bytes
        : row.sizeTrackedBytes < term.bytes;
    }
    case 'touchedAge': {
      const ageDays = (ctx.now - row.lastTouchedAt) / DAY;
      return term.op === 'gt' ? ageDays > term.days : ageDays < term.days;
    }
    case 'touchedYear':
      return localYear(row.lastTouchedAt) === term.year;
  }
}

/** True when the term can be answered for *any* row, given what the projection carries. */
function answerable(term: QueryTerm, ctx: QueryContext): boolean {
  const caps = ctx.capabilities;
  if (term.kind === 'flag') {
    if (term.flag === 'local' || term.flag === 'wsl') return caps.location;
    if (term.flag === 'new') return ctx.firstRunCompletedAt !== null;
    return true;
  }
  if (term.kind === 'has') {
    switch (term.attribute) {
      case 'stash':
        return true;
      case 'readme':
        return caps.hasReadme;
      case 'license':
        return caps.hasLicense;
      case 'tests':
        return caps.hasTests;
      case 'ci':
        return caps.hasCi;
      case 'remote':
        return caps.hasRemote;
      case 'submodules':
        return caps.hasSubmodules;
    }
  }
  if (term.kind === 'text' && term.field === 'in') {
    const value = term.value.toLowerCase();
    if (LOCATION_KEYWORDS.has(value) || value.startsWith('wsl:')) return caps.location;
  }
  return true;
}

export function partitionAnswerable(
  ast: QueryAst,
  ctx: QueryContext,
): { readonly runnable: readonly QueryTerm[]; readonly ignored: readonly IgnoredTerm[] } {
  const runnable: QueryTerm[] = [];
  const ignored: IgnoredTerm[] = [];
  for (const term of ast.terms) {
    if (answerable(term, ctx)) runnable.push(term);
    else ignored.push({ text: renderTerm(term), reason: 'notAvailable' });
  }
  return { runnable, ignored };
}

/** §8.0b's base predicate: reference and hidden rows are out unless the query asks for them. */
export function inBaseSet(row: ShelfRow, ast: QueryAst): boolean {
  const asksFor = (flag: 'reference' | 'hidden'): boolean =>
    ast.terms.some((t) => t.kind === 'flag' && t.flag === flag && !t.negated);
  if (row.isReference && !asksFor('reference')) return false;
  if (row.isHidden && !asksFor('hidden')) return false;
  return true;
}

export function evaluateQuery(
  rows: readonly ShelfRow[],
  ast: QueryAst,
  ctx: QueryContext,
): { readonly rows: readonly ShelfRow[]; readonly ignored: readonly IgnoredTerm[] } {
  const { runnable, ignored } = partitionAnswerable(ast, ctx);
  const matched = rows.filter((row) => {
    if (!inBaseSet(row, ast)) return false;
    return runnable.every((term) => {
      const truth = termTruth(row, term, ctx);
      if (truth === null) return false;
      return term.negated ? truth === false : truth === true;
    });
  });
  return { rows: matched, ignored: [...ast.ignored, ...ignored] };
}
